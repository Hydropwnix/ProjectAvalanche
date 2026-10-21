struct Wave {
    world: array<vec4<f32>, 4>,
    view_proj: array<vec4<f32>, 4>,
    view: array<vec4<f32>, 4>,
    camera_pos: vec4<f32>,
    light_dir: vec4<f32>,
    light_colour: vec4<f32>,
    light_view_offset: vec4<f32>,
    bounce_view: vec4<f32>,
    ramp0: vec4<f32>,
    ramp1: vec4<f32>,
    ramp2: vec4<f32>,
    ramp: vec4<f32>,
    misc: vec4<f32>,
    spec_colour: vec4<f32>,
    beta_r: vec4<f32>,
    beta_dash_r: vec4<f32>,
    beta_m: vec4<f32>,
    beta_dash_m: vec4<f32>,
    heights: vec4<f32>,
    multipliers: vec4<f32>,
    uv: vec4<f32>,
    tiles: vec4<f32>,
    hable_a: vec4<f32>,
    hable_b: vec4<f32>,
}

struct Exposure {
    log2_mean: f32,
    lavg: f32,
    exposure: f32,
    bloom_exposure: f32,
}

@group(0) @binding(0) var<uniform> g: Wave;
@group(0) @binding(1) var normal_tex: texture_2d<f32>;
@group(0) @binding(2) var diffuse_tex: texture_2d<f32>;
@group(0) @binding(3) var spec_tex: texture_2d<f32>;
@group(0) @binding(4) var wrap_trilinear: sampler;
@group(0) @binding(5) var<storage, read> exposure: Exposure;

fn xnormalize(v: vec3<f32>) -> vec3<f32> {
    let r = inverseSqrt(abs(dot(v, v)));
    return select(v * r, vec3<f32>(0.0), v == vec3<f32>(0.0));
}

fn row_mul(p: vec3<f32>, m: array<vec4<f32>, 4>) -> vec4<f32> {
    return p.x * m[0] + p.y * m[1] + p.z * m[2] + m[3];
}

fn rot_mul(p: vec3<f32>, m: array<vec4<f32>, 4>) -> vec3<f32> {
    return p.x * m[0].xyz + p.y * m[1].xyz + p.z * m[2].xyz;
}

struct WaveOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) o1: vec4<f32>,
    @location(1) o2: vec4<f32>,
    @location(2) o3: vec3<f32>,
    @location(3) o4: vec3<f32>,
    @location(4) o5: vec3<f32>,
    @location(5) o6: vec4<f32>,
}

@vertex
fn wave_vs(
    @location(0) p: vec3<f32>,
    @location(1) n: vec3<f32>,
    @location(2) t: vec3<f32>,
    @location(3) uv: vec2<f32>,
    @location(4) colour: vec4<f32>,
) -> WaveOut {
    var o: WaveOut;
    let w = row_mul(p, g.world).xyz;
    o.pos = row_mul(w, g.view_proj);
    let scroll = fract(g.camera_pos.w * g.uv.xy);
    let suv = uv + scroll;
    o.o1 = vec4<f32>(rot_mul(n, g.world), suv.x);
    o.o2 = vec4<f32>(rot_mul(t, g.world), suv.y);
    o.o3 = w;
    o.o6 = colour;
    let d = w - g.camera_pos.xyz;
    let s = sqrt(abs(dot(d, d)));
    let vw = xnormalize(d);
    let cw = saturate(dot(vw, g.light_dir.xyz));
    let rp = g.beta_dash_r.w;
    let hy = w.y - 1200.0 + rp;
    let h = sqrt(abs(w.x * w.x + w.z * w.z + hy * hy)) - rp;
    let rho_r = exp2((g.heights.y - h) * (g.heights.x * 1.442695));
    let rho_m = exp2((g.heights.w - h) * (g.heights.z * 1.442695));
    let sigma = rho_r * g.beta_r.xyz + rho_m * g.beta_m.xyz;
    let tr = exp2(-(s * 1.442695) * sigma);
    o.o4 = tr * g.multipliers.x;
    let gg = g.beta_r.w;
    let mie = exp2(log2(abs(1.0 + gg * gg - 2.0 * gg * cw)) * 1.5);
    let bd = (1.0 + cw * cw) * g.beta_dash_r.xyz + g.beta_dash_m.xyz * (1.0 - gg * gg) / mie;
    let fr = bd / (g.beta_r.xyz + g.beta_m.xyz);
    o.o5 = fr * (1.0 - tr) * g.multipliers.y * g.light_colour.xyz;
    return o;
}

@fragment
fn wave_fs(in: WaveOut) -> @location(0) vec4<f32> {
    let u = in.o1.w;
    let v = in.o2.w;
    let nt_raw = textureSample(normal_tex, wrap_trilinear, vec2<f32>(u * g.tiles.x, v * g.tiles.y)).xy;
    let dif = textureSample(diffuse_tex, wrap_trilinear, vec2<f32>(u * g.uv.z, v * g.uv.w));
    let spec = textureSample(spec_tex, wrap_trilinear, vec2<f32>(u * g.tiles.z, v * g.tiles.w)).y;
    let nt = normalize(vec3<f32>(nt_raw * 2.0 - 1.0, 1.0));
    let nn = in.o1.xyz;
    let tt = in.o2.xyz;
    let bb = cross(nn, tt);
    let np = nt.x * tt + nt.y * bb + nt.z * nn;
    let nw = xnormalize(np);
    let nv = rot_mul(nw, g.view);
    let pv = row_mul(in.o3, g.view).xyz;
    let vdir = xnormalize(-pv);
    let l = xnormalize(g.light_view_offset.xyz);
    let nl = dot(l, nv);
    let ndl = saturate(dot(nv, l));
    let wr = ndl * (g.ramp.z - g.ramp.w) + g.ramp.w;
    let rim = saturate(exp2(g.ramp.y * log2(abs(1.0 - abs(dot(vdir, nv))))));
    let dd = nl + (rim - ndl) * wr;
    let bx = saturate((dot(nv, g.bounce_view.xyz) - g.misc.y) / (1.0 - g.misc.y));
    let light = mix(g.ramp1.xyz, g.ramp0.xyz, dd) * g.ramp.x + g.misc.x * bx * bx * (3.0 - 2.0 * bx) * g.ramp2.xyz;
    let r = l - (nl + ndl) * nv;
    let ph = saturate(dot(-vdir, r));
    let sp = exp2(log2(ph) * g.multipliers.w) * g.spec_colour.xyz;
    let si = g.multipliers.z * spec;
    let c = dif.rgb * dif.rgb * light + sp * (si * si);
    let col = in.o6.rgb * g.misc.z * c * in.o4 + in.o5;
    let x = col * (g.hable_b.w / exposure.lavg);
    let tm = ((x * (x * g.hable_a.x + g.hable_a.z) + g.hable_a.w) / (x * (x * g.hable_a.x + g.hable_a.y) + g.hable_b.x)
        - g.hable_b.y) * g.hable_b.z;
    return vec4<f32>(sqrt(abs(tm)), dif.a * in.o6.a);
}
