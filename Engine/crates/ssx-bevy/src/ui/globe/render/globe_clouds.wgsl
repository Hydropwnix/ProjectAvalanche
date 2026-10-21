struct Clouds {
    proj: array<vec4<f32>, 4>,
    world: array<vec4<f32>, 4>,
    time: vec4<f32>,
    color_mult: vec4<f32>,
    color_spread: vec4<f32>,
    ramp: array<vec4<f32>, 4>,
    ramp_times: vec4<f32>,
    size_ramp: vec4<f32>,
    size_ramp_times: vec4<f32>,
    flags: vec4<f32>,
    anim: vec4<f32>,
    anim2: vec4<f32>,
    pivot: vec4<f32>,
    fade: vec4<f32>,
    feather: vec4<f32>,
    ramp0: vec4<f32>,
    ramp1: vec4<f32>,
    sizes: vec4<f32>,
    depth: vec4<f32>,
}

@group(0) @binding(0) var<uniform> c: Clouds;
@group(0) @binding(1) var<storage, read> records: array<vec4<f32>>;
@group(0) @binding(2) var tex: texture_2d<f32>;
@group(0) @binding(3) var tex_sampler: sampler;
@group(0) @binding(4) var half_depth: texture_2d<f32>;

const TAU: f32 = 6.2831855;
const PI: f32 = 3.1415927;

fn row_mul(p: vec4<f32>, m: array<vec4<f32>, 4>) -> vec4<f32> {
    return p.x * m[0] + p.y * m[1] + p.z * m[2] + p.w * m[3];
}

fn ramp4(t: f32, k0: vec4<f32>, k1: vec4<f32>, k2: vec4<f32>, k3: vec4<f32>, times: vec4<f32>) -> vec4<f32> {
    let a = saturate((vec3(t) - times.xyz) / (times.yzw - times.xyz));
    return mix(mix(mix(k0, k1, a.x), k2, a.y), k3, a.z);
}

fn spread_hash(w: f32) -> vec4<f32> {
    let f = fract(w * vec4(738.5924, 65.45774, 7.80416, 0.73196197));
    let f2 = fract(w * vec2(5.907823, 0.104400896));
    let rx = fract(f.x + (f.y + (((f.z + f.w) + f2.x) + f2.y)));
    let ya = fract(w * vec3(3.0987134, 0.054759335, 0.3839215));
    let yb = fract(w * vec3(4.0933614, 34.333252, 387.39926));
    let ry = fract(((((ya.x + ya.y) + ya.z) + yb.x) + yb.y) + yb.z);
    let za = fract(w * vec3(2.1902876, 0.038705964, 0.2713702));
    let zb = fract(w * vec3(2.8933425, 24.268038, 273.8284));
    let rz = fract(((((za.x + za.y) + za.z) + zb.x) + zb.y) + zb.z);
    let wa = fract(w * vec3(1.0, 0.017671635, 0.12389707));
    let wb = fract(w * vec3(1.3209875, 11.079841, 125.01939));
    let rw = fract(((((wa.x + wa.y) + wa.z) + wb.x) + wb.y) + wb.z);
    return vec4(rw, rz, ry, rx);
}

fn anim_uv(uv: vec2<f32>, birth: f32, t: f32) -> vec2<f32> {
    if c.flags.w < 0.5 {
        return uv;
    }
    let mode = c.anim.x;
    let count = c.anim.y;
    let rows = c.anim.z;
    let speed = c.anim.w;
    let k = fract(birth * vec4(11.079841, 1.3209875, 0.12389707, 0.017671635));
    let rnd = fract(((k.x + k.y) + (k.z + k.w)) + fract(birth) + fract(125.01939 * birth));
    let f = max(mode * (rnd * c.anim2.x + t * count * speed) - 1.0
        + (1.0 - mode) * (0.06 * speed * c.time.x + rnd * c.anim2.x), 0.0);
    let fm = f - floor(f / count) * count;
    let cols = ceil(count / rows);
    let row = floor(fm / cols);
    let col = floor(fm - cols * row);
    return vec2((uv.x + col) / cols, (uv.y + row) / rows);
}

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) depth: f32,
}

const CORNERS: array<vec2<f32>, 6> = array<vec2<f32>, 6>(
    vec2(0.0, 1.0), vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0), vec2(1.0, 0.0), vec2(1.0, 1.0),
);

@vertex
fn vs_cloud(@builtin(vertex_index) vi: u32) -> VsOut {
    var out: VsOut;
    let pos = records[(vi / 6u) * 2u];
    let tc0 = records[(vi / 6u) * 2u + 1u];
    let uv = CORNERS[vi % 6u];
    var t = 0.0;
    if c.time.y > 1e-5 {
        t = (c.time.x - tc0.w) / c.time.y;
    }
    var col = vec4(1.0);
    if c.flags.x > 0.5 {
        col = ramp4(t, c.ramp[0], c.ramp[1], c.ramp[2], c.ramp[3], c.ramp_times);
    }
    if c.flags.y > 0.5 {
        col = col * c.color_mult + (spread_hash(tc0.w) - 0.5) * c.color_spread;
    }
    var sr = 1.0;
    if c.flags.z > 0.5 {
        sr = ramp4(t, vec4(c.size_ramp.x), vec4(c.size_ramp.y), vec4(c.size_ramp.z), vec4(c.size_ramp.w),
            c.size_ramp_times).x;
    }
    let pu = uv.x + c.pivot.x - 0.5;
    let pv = uv.y + c.pivot.y - 0.5;
    let ang = fract(tc0.x / TAU + 0.5) * TAU - PI;
    let dx = pu * sr;
    let dy = pv * sr;
    let rx = dx * cos(ang) - dy * sin(ang);
    let ry = -(dx * sin(ang) + dy * cos(ang));
    let size = length(c.world[0].xyz) * pos.w;
    let view = row_mul(vec4(pos.xyz, 1.0), c.world) + vec4(size * rx, size * ry, 0.0, 0.0);
    col.a = col.a * saturate((-c.fade.y - view.z) / (c.fade.x - c.fade.y));
    var clip = row_mul(view, c.proj);
    clip.z = clip.z - c.fade.z / clip.z;
    if col.a <= 1.0 / 256.0 {
        clip = vec4(0.0, 0.0, 0.0, -1.0);
    }
    out.clip = clip;
    out.uv = anim_uv(uv, tc0.w, t);
    out.color = col;
    out.depth = clip.z / clip.w;
    return out;
}

fn view_dist(d: f32) -> f32 {
    return c.depth.y / (d + c.depth.x);
}

@fragment
fn fs_cloud(in: VsOut) -> @location(0) vec4<f32> {
    let px = vec2<i32>(floor(in.clip.xy));
    let scene = textureLoad(half_depth, clamp(px, vec2(0), vec2<i32>(c.sizes.xy) - 1), 0).x;
    let dz = view_dist(scene) - view_dist(in.depth);
    if dz < 0.0 {
        discard;
    }
    let s = textureSampleBias(tex, tex_sampler, in.uv, -1.0);
    let rgb = mix(c.ramp1.rgb, c.ramp0.rgb, s.y) * c.ramp0.w * in.color.rgb;
    let a = s.a * in.color.a * saturate(dz * c.feather.x - c.feather.y) * c.time.w;
    if a <= 1.0 / 255.0 {
        discard;
    }
    return vec4<f32>(sqrt(abs(rgb)), a);
}

struct FullOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> FullOut {
    let uv = vec2(f32(i >> 1u), f32(i & 1u)) * 2.0;
    var out: FullOut;
    out.position = vec4(uv * vec2(2.0, -2.0) + vec2(-1.0, 1.0), 0.0, 1.0);
    out.uv = uv;
    return out;
}

@group(0) @binding(5) var scene_depth: texture_depth_2d;
@group(0) @binding(6) var half_colour: texture_2d<f32>;
@group(0) @binding(7) var linear_clamp: sampler;

@fragment
fn fs_depth_filter(in: FullOut) -> @location(0) vec4<f32> {
    let full = vec2<i32>(c.sizes.zw);
    let p = vec2<i32>(in.position.xy) * 2;
    let a = textureLoad(scene_depth, min(p, full - 1), 0);
    let b = textureLoad(scene_depth, min(p + vec2(1, 0), full - 1), 0);
    let e = textureLoad(scene_depth, min(p + vec2(0, 1), full - 1), 0);
    let f = textureLoad(scene_depth, min(p + vec2(1, 1), full - 1), 0);
    return vec4(max(max(a, b), max(e, f)), 0.0, 0.0, 0.0);
}

@fragment
fn fs_composite(in: FullOut) -> @location(0) vec4<f32> {
    let full = vec2<i32>(c.sizes.zw);
    let half = vec2<i32>(c.sizes.xy);
    let off = 1.0 / c.sizes.zw;
    let uv_p = in.uv + off;
    let uv_m = in.uv - off;
    let max_d = 500.0;
    let d0 = min(view_dist(textureLoad(scene_depth, clamp(vec2<i32>(in.position.xy), vec2(0), full - 1), 0)), max_d);
    let dp = min(view_dist(textureLoad(half_depth, clamp(vec2<i32>(uv_p * c.sizes.xy), vec2(0), half - 1), 0).x), max_d);
    let dm = min(view_dist(textureLoad(half_depth, clamp(vec2<i32>(uv_m * c.sizes.xy), vec2(0), half - 1), 0).x), max_d);
    let wp = 1.0 / (abs(d0 - dp) + 1.0);
    let wm = 1.0 / (abs(d0 - dm) + 1.0);
    let sp = textureSampleLevel(half_colour, linear_clamp, uv_p, 0.0);
    let sm = textureSampleLevel(half_colour, linear_clamp, uv_m, 0.0);
    return (wp * sp + wm * sm) / (wp + wm);
}
