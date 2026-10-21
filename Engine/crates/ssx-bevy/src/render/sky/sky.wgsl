#import bevy_render::view::View

struct SkyUniform {
    beta_r: vec4<f32>,
    beta_dash_r: vec4<f32>,
    beta_m: vec4<f32>,
    beta_dash_m: vec4<f32>,
    heights: vec4<f32>,
    multipliers: vec4<f32>,
    light_dir: vec4<f32>,
    light_colour: vec4<f32>,
    cloud_ramp0: vec4<f32>,
    cloud_ramp1: vec4<f32>,
    cloud: vec4<f32>,
    sun: vec4<f32>,
}

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<uniform> sky: SkyUniform;
@group(0) @binding(2) var cloud_density: texture_2d<f32>;
@group(0) @binding(3) var cloud_normal: texture_2d<f32>;
@group(0) @binding(4) var light_texture: texture_2d<f32>;
@group(0) @binding(5) var repeat_sampler: sampler;
@group(0) @binding(6) var clamp_sampler: sampler;

const PI: f32 = 3.14159265;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) ndc: vec2<f32>,
}

@vertex
fn vertex(@builtin(vertex_index) i: u32) -> VertexOutput {
    let uv = vec2<f32>(f32(i >> 1u), f32(i & 1u)) * 2.0;
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    return VertexOutput(vec4<f32>(ndc, 0.0, 1.0), ndc);
}

fn sky_atan2(y: f32, x: f32) -> f32 {
    let t = min(abs(y), abs(x)) / max(abs(y), abs(x));
    let t2 = t * t;
    let a = t * (0.999866 + t2 * (-0.3302995 + t2 * (0.180141 + t2 * (-0.085133 + 0.0208351 * t2))));
    var th = select(a, 1.5707964 - a, abs(y) > abs(x));
    th = select(th, th - 3.1415927, x < 0.0);
    return select(th, -th, (y < 0.0) != (x < 0.0));
}

fn sky_acos(c: f32) -> f32 {
    let a = abs(c);
    let r = sqrt(1.0 - a) * (1.5707288 + a * (-0.2121144 + a * (0.074261 - 0.0187293 * a)));
    return select(r, 3.1415927 - r, c < 0.0);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let vp = view.view_from_clip * vec4<f32>(in.ndc, 1.0, 1.0);
    let v = normalize(vp.xyz / vp.w);
    let w = normalize((view.world_from_view * vec4<f32>(v, 0.0)).xyz);
    let lw = sky.light_dir.xyz;
    let lv = normalize((view.view_from_world * vec4<f32>(lw, 0.0)).xyz);
    let lc = sky.light_colour.rgb;

    let radius = sky.beta_dash_r.w;
    let s = sky.beta_m.w;
    let p = (view.world_position + s * 1000.0 * w) * 0.001;
    let h = length(vec3<f32>(p.x, p.y + radius, p.z)) - radius;
    let rho_r = exp((sky.heights.y - h) * sky.heights.x);
    let rho_m = exp((sky.heights.w - h) * sky.heights.z);
    let sigma = rho_r * sky.beta_r.xyz + rho_m * sky.beta_m.xyz;
    let t = exp(-sigma * s);
    let tc = exp(-sigma * s * sky.beta_dash_m.w);

    let cw = clamp(dot(w, lw), 0.0, 1.0);
    let g = sky.beta_r.w;
    let num = 1.0 - g * g;
    let den = pow(max(1.0 + g * g - 2.0 * g * cw, 0.0), 1.5);
    let mie_phase = select(num / den, 0.0, num == 0.0);
    let bd = sky.beta_dash_r.xyz * (1.0 + cw * cw) + sky.beta_dash_m.xyz * mie_phase;
    let fr = bd / (sky.beta_r.xyz + sky.beta_m.xyz);
    let lin = fr * (1.0 - t) * sky.multipliers.y;
    let lin_c = fr * (1.0 - tc) * sky.multipliers.y;

    let z = h / length(vec3<f32>(p.x, h, p.z));
    let cuv = vec2<f32>(
        sky_atan2(p.z, p.x) / (2.0 * PI) + 0.5 + sky.light_colour.w,
        sky_acos(z) * 0.63661975,
    );
    let d = textureSample(cloud_density, repeat_sampler, cuv).y;
    let c = clamp(d * sky.cloud_ramp1.w, 0.0, 1.0);
    let tn = textureSample(cloud_normal, repeat_sampler, cuv).xy;
    let nt = normalize(vec3<f32>(1.0, 2.0 * tn.x - 1.0, 2.0 * tn.y - 1.0));
    let tg = cross(v, vec3<f32>(0.0, 1.0, 0.0));
    let bt = cross(tg, v);
    let nc = nt.x * (-v) + nt.y * tg + nt.z * bt;
    let wrap = 0.5 * dot(nc, lv) + 0.5;
    let gl_mult = sky.multipliers.z;
    let cloud_col = gl_mult * sky.cloud_ramp0.w * mix(sky.cloud_ramp1.rgb, sky.cloud_ramp0.rgb, mix(1.0 - d, d, wrap));

    let st = textureSample(light_texture, clamp_sampler, sky.light_dir.w * (v.xy - lv.xy) + 0.5);
    let sa = sky.multipliers.w * select(st.a, 0.0, lv.z >= 0.0);
    let sky_col = lin * lc + st.rgb * gl_mult * sa;
    var col = mix(sky_col, sky.multipliers.x * tc * cloud_col + lin_c * lc, c);

    let cv = clamp(dot(lv, v), 0.0, 1.0) + 1e-7;
    let rim_d = abs(c - sky.cloud.x);
    let rim_q = select(rim_d * sky.cloud.y, 0.0, rim_d == 0.0);
    let rim = c * sky.cloud.z * pow(cv, sky.cloud.w) * clamp(1.0 - rim_q, 0.0, 1.0);
    let k = clamp((1.0 - sa) * pow(cv, sky.sun.y) + rim, 0.0, 1.0) * (1.0 - c);
    col = mix(col, lc * (sky.sun.x + lin), k);
    return vec4<f32>(col * sky.sun.z, 1.0);
}
