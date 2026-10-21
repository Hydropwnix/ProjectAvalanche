struct GameLight {
    ramp_lit: vec4<f32>,
    ramp_shade: vec4<f32>,
    ramp_bounce: vec4<f32>,
    light_dir: vec4<f32>,
    bounce_dir: vec4<f32>,
    rim_shadow: vec4<f32>,
    specular: vec4<f32>,
    fog_color: vec4<f32>,
    fog_data: vec4<f32>,
    sparkle: vec4<f32>,
    misc: vec4<f32>,
}

struct Part {
    world: mat4x4<f32>,
    light: GameLight,
    beta_r: vec4<f32>,
    beta_dash_r: vec4<f32>,
    beta_m: vec4<f32>,
    beta_dash_m: vec4<f32>,
    heights: vec4<f32>,
    multipliers: vec4<f32>,
    sun_dir: vec4<f32>,
    sun_colour: vec4<f32>,
    tiles: vec4<f32>,
    ao_tile: vec4<f32>,
}

struct ViewData {
    view_from_world: mat4x4<f32>,
    clip_from_view: mat4x4<f32>,
    camera: vec4<f32>,
}

@group(0) @binding(0) var<uniform> view_data: ViewData;
@group(0) @binding(1) var<uniform> part: Part;
@group(0) @binding(2) var normal_texture: texture_2d<f32>;
@group(0) @binding(3) var occlusion_texture: texture_2d<f32>;
@group(0) @binding(4) var mask_texture: texture_2d<f32>;
@group(0) @binding(5) var red_texture: texture_2d<f32>;
@group(0) @binding(6) var green_texture: texture_2d<f32>;
@group(0) @binding(7) var base_texture: texture_2d<f32>;
@group(0) @binding(8) var mg_sampler: sampler;
@group(0) @binding(9) var scene_depth: texture_depth_2d;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tangent: vec3<f32>,
    @location(3) uv: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) @invariant clip: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) world_tangent: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) world_position: vec3<f32>,
    @location(4) extinction: vec3<f32>,
    @location(5) inscatter: vec3<f32>,
}

const LOG2E: f32 = 1.442695;

@vertex
fn vertex(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    let pw = (part.world * vec4(in.position, 1.0)).xyz;
    let pv = (view_data.view_from_world * vec4(pw, 1.0)).xyz;
    out.clip = view_data.clip_from_view * vec4(pv / part.beta_dash_m.w, 1.0);
    out.world_normal = (part.world * vec4(in.normal, 0.0)).xyz;
    out.world_tangent = (part.world * vec4(in.tangent, 0.0)).xyz;
    out.uv = in.uv;
    out.world_position = pw;

    let d = pw * 0.001 - view_data.camera.xyz * 0.001;
    let s = sqrt(dot(d, d));
    let vw = d * inverseSqrt(dot(d, d));
    let radius = part.beta_dash_r.w;
    let q = vec3(pw.x * 0.001, pw.y * 0.001 + radius, pw.z * 0.001);
    let h = sqrt(dot(q, q)) - radius;
    let rho_r = exp2((part.heights.y - h) * (part.heights.x * LOG2E));
    let rho_m = exp2((part.heights.w - h) * (part.heights.z * LOG2E));
    let sigma = rho_r * part.beta_r.xyz + rho_m * part.beta_m.xyz;
    let t = exp2(-sigma * (s * LOG2E));
    let cw = saturate(dot(vw, part.sun_dir.xyz));
    let g = part.beta_r.w;
    let num = 1.0 - g * g;
    let den = exp2(1.5 * log2(abs(1.0 + g * g - 2.0 * g * cw)));
    let mie = select(num * (1.0 / den), 0.0, num == 0.0);
    let bd = part.beta_dash_r.xyz * (1.0 + cw * cw) + part.beta_dash_m.xyz * mie;
    let fr = bd * (1.0 / (part.beta_r.xyz + part.beta_m.xyz));
    out.extinction = t * part.multipliers.x;
    out.inscatter = fr * (1.0 - t) * part.multipliers.y * part.sun_colour.rgb;
    return out;
}

fn safe_normalize(v: vec3<f32>) -> vec3<f32> {
    let l = dot(v, v);
    return select(v * inverseSqrt(l), vec3(0.0), l == 0.0);
}

fn game_tbn(world_normal: vec3<f32>, world_tangent: vec3<f32>, nt: vec3<f32>) -> vec3<f32> {
    let n = safe_normalize(world_normal);
    let t = safe_normalize(world_tangent);
    let b = cross(n, t);
    return safe_normalize(nt.x * t + nt.y * b + nt.z * n);
}

fn ramp_diffuse(l: GameLight, n: vec3<f32>, v: vec3<f32>) -> vec3<f32> {
    var k = saturate(dot(n, l.light_dir.xyz));
    let rim = saturate(pow(max(1.0 - abs(dot(n, v)), 1e-7), l.ramp_shade.w));
    k = mix(k, rim, mix(l.rim_shadow.y, l.rim_shadow.x, k));
    let spread = l.bounce_dir.w;
    let t = saturate((dot(n, l.bounce_dir.xyz) - spread) / max(1.0 - spread, 1e-4));
    let bounce = l.ramp_bounce.w * t * t * (3.0 - 2.0 * t);
    return l.ramp_lit.w * mix(l.ramp_shade.rgb, l.ramp_lit.rgb, k) + bounce * l.ramp_bounce.rgb;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    if textureLoad(scene_depth, vec2<i32>(in.clip.xy), 0) > 0.0 {
        discard;
    }
    let uv = in.uv;
    let base = textureSample(base_texture, mg_sampler, uv * part.tiles.x).rgb;
    let red = textureSample(red_texture, mg_sampler, uv * part.tiles.y).rgb;
    let green = textureSample(green_texture, mg_sampler, uv * part.tiles.z).rgb;
    let occ = textureSample(occlusion_texture, mg_sampler, uv * part.ao_tile.xy).y;
    let m = textureSample(mask_texture, mg_sampler, uv).rgb;
    let tn = textureSample(normal_texture, mg_sampler, uv).xy;
    let ao = occ * occ;
    var alb = mix(base * base, green * green, m.g);
    alb = mix(alb, vec3(0.5), saturate(m.b + part.tiles.w));
    alb = mix(alb, red * red, m.r);
    let n = game_tbn(in.world_normal, in.world_tangent, normalize(vec3(tn * 2.0 - 1.0, 1.0)));
    let v = normalize(view_data.camera.xyz - in.world_position);
    let c = ramp_diffuse(part.light, n, v) * mix(part.light.ramp_shade.rgb, vec3(1.0), ao) * alb;
    let col = c * in.extinction + in.inscatter * part.multipliers.z;
    return vec4(col, 1.0);
}
