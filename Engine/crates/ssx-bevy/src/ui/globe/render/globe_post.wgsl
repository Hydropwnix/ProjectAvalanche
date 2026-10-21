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

struct Post {
    hable_a: vec4<f32>,
    hable_b: vec4<f32>,
    exposure: vec4<f32>,
    bloom: vec4<f32>,
    blur_h_samples: vec4<f32>,
    blur_h_offsets: vec4<f32>,
    blur_v_samples: vec4<f32>,
    blur_v_offsets: vec4<f32>,
    vignette: vec4<f32>,
    bounds: vec4<f32>,
    debug: vec4<f32>,
    sizes: vec4<u32>,
}

struct Exposure {
    log2_mean: f32,
    lavg: f32,
    exposure: f32,
    bloom_exposure: f32,
}

const LUMA: vec3<f32> = vec3<f32>(0.299, 0.587, 0.114);
const ENC: f32 = 0.0433217;
const DEC: f32 = 23.08312;

@group(0) @binding(0) var<uniform> u: Post;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var<storage, read_write> adapted: array<f32>;
@group(0) @binding(3) var<storage, read_write> exposure: Exposure;

fn store16(x: f32) -> f32 {
    let h = unpack2x16float(pack2x16float(vec2<f32>(x, 0.0))).x;
    return round(clamp(h, 0.0, 1.0) * 65535.0) / 65535.0;
}

fn hable(x: vec3<f32>) -> vec3<f32> {
    return (x * (u.hable_a.x * x + u.hable_a.z) + u.hable_a.w) / (x * (u.hable_a.x * x + u.hable_a.y) + u.hable_b.x)
        - u.hable_b.y;
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3(2.4)), c / 12.92, c <= vec3(0.04045));
}

@fragment
fn fs_quarter(in: FullOut) -> @location(0) vec4<f32> {
    let q = vec2<i32>(floor(in.position.xy));
    let last = vec2<i32>(u.sizes.xy) - vec2<i32>(1);
    var sum = vec3<f32>(0.0);
    for (var j = 0; j < 4; j++) {
        for (var i = 0; i < 4; i++) {
            sum += max(textureLoad(src, min(q * 4 + vec2<i32>(i, j), last), 0).rgb, vec3<f32>(0.0));
        }
    }
    return vec4<f32>(sum / 16.0, 1.0);
}

@compute @workgroup_size(8, 8, 1)
fn luminance(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = u.sizes.zw;
    if (id.x >= size.x || id.y >= size.y) {
        return;
    }
    let q = textureLoad(src, vec2<i32>(id.xy), 0).rgb;
    let c = q * q;
    let l_enc = store16(log2(max(dot(c, LUMA), 1.0)) * ENC);
    let i = id.y * size.x + id.x;
    var a_enc = 0.0;
    if (u.bloom.w == 1.0) {
        a_enc = l_enc;
    } else if (u.bloom.w == 0.0) {
        let a = exp2(DEC * adapted[i]);
        let lc = exp2(DEC * l_enc);
        let k = max(1.0 - exp2(-u.exposure.w * 1.442695 * u.exposure.z), 0.0);
        a_enc = store16(log2(max(a + (lc - a) * k, 1.0)) * ENC);
    }
    adapted[i] = a_enc;
}

fn fetch(base: u32, w: u32, h: u32, uv: vec2<f32>) -> f32 {
    let p = uv * vec2<f32>(f32(w), f32(h)) - 0.5;
    let p0 = floor(p);
    let f = p - p0;
    let x0 = u32(clamp(p0.x, 0.0, f32(w - 1u)));
    let y0 = u32(clamp(p0.y, 0.0, f32(h - 1u)));
    let x1 = u32(clamp(p0.x + 1.0, 0.0, f32(w - 1u)));
    let y1 = u32(clamp(p0.y + 1.0, 0.0, f32(h - 1u)));
    let a = mix(adapted[base + y0 * w + x0], adapted[base + y0 * w + x1], f.x);
    let b = mix(adapted[base + y1 * w + x0], adapted[base + y1 * w + x1], f.x);
    return mix(a, b, f.y);
}

@compute @workgroup_size(256, 1, 1)
fn reduce(@builtin(local_invocation_index) li: u32) {
    var w = u.sizes.z;
    var h = u.sizes.w;
    var base = 0u;
    loop {
        if (w == 1u && h == 1u) {
            break;
        }
        let nw = max(w / 2u, 1u);
        let nh = max(h / 2u, 1u);
        let next = base + w * h;
        for (var i = li; i < nw * nh; i += 256u) {
            let uv = (vec2<f32>(f32(i % nw), f32(i / nw)) + 0.5) / vec2<f32>(f32(nw), f32(nh));
            adapted[next + i] = store16(fetch(base, w, h, uv));
        }
        storageBarrier();
        workgroupBarrier();
        base = next;
        w = nw;
        h = nh;
    }
    if (li == 0u) {
        let m = DEC * adapted[base];
        let lavg = exp2(m);
        let e = u.exposure.y / lavg;
        exposure.log2_mean = m;
        exposure.lavg = lavg;
        exposure.exposure = e;
        exposure.bloom_exposure = exp2(log2(max(e, 0.0001)) - u.bloom.x);
    }
}

@group(0) @binding(4) var<storage, read> exposure_in: Exposure;
@group(0) @binding(5) var linear_clamp: sampler;
@group(0) @binding(6) var bloom_tex: texture_2d<f32>;
@group(0) @binding(7) var lut: texture_3d<f32>;

@fragment
fn fs_threshold(in: FullOut) -> @location(0) vec4<f32> {
    let q = textureLoad(src, vec2<i32>(floor(in.position.xy)), 0).rgb;
    let t = hable(q * q * exposure_in.bloom_exposure) * u.hable_b.z;
    return vec4<f32>(sqrt(max(t, vec3<f32>(0.0))), 1.0);
}

fn tap(uv: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(src, linear_clamp, uv, 0.0).rgb;
}

fn blur(uv: vec2<f32>, dir: vec2<f32>, s: vec4<f32>, o: vec4<f32>) -> vec4<f32> {
    let t = dir / vec2<f32>(u.sizes.zw);
    let c = s.x * tap(uv + o.x * t) + s.y * tap(uv + o.y * t) + s.z * tap(uv)
        + s.y * tap(uv + o.z * t) + s.x * tap(uv + o.w * t);
    return vec4<f32>(c, 1.0);
}

@fragment
fn fs_blur_h(in: FullOut) -> @location(0) vec4<f32> {
    return blur(in.uv, vec2<f32>(1.0, 0.0), u.blur_h_samples, u.blur_h_offsets);
}

@fragment
fn fs_blur_v(in: FullOut) -> @location(0) vec4<f32> {
    return blur(in.uv, vec2<f32>(0.0, 1.0), u.blur_v_samples, u.blur_v_offsets);
}

@fragment
fn fs_tonemap(in: FullOut) -> @location(0) vec4<f32> {
    let q = max(textureLoad(src, vec2<i32>(floor(in.position.xy)), 0).rgb, vec3<f32>(0.0));
    if (u.debug.x > 0.0) {
        return vec4<f32>(clamp(q * u.debug.x, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
    }
    let tm = hable(q * q * exposure_in.exposure) * u.hable_b.z;
    return vec4<f32>(sqrt(clamp(tm, vec3<f32>(0.0), vec3<f32>(1.0))), 1.0);
}

const EDGE_SHARPNESS: f32 = 8.0;
const EDGE_THRESHOLD: f32 = 0.125;
const EDGE_THRESHOLD_MIN: f32 = 0.05;
const CONST_DIR: vec4<f32> = vec4<f32>(1.0, -1.0, 0.25, -0.25);
const NE_BIAS: f32 = 0.0026041667;

fn rcp_frame() -> vec2<f32> {
    return 1.0 / vec2<f32>(u.sizes.xy);
}

fn fx_tap(uv: vec2<f32>, off: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(src, linear_clamp, uv + off * rcp_frame(), 0.0);
}

@fragment
fn fs_fxaa(in: FullOut) -> @location(0) vec4<f32> {
    let pos = in.uv;
    let m = fx_tap(pos, vec2<f32>(0.0, 0.0));
    let nw = fx_tap(pos, vec2<f32>(-0.5, -0.5)).y;
    let se = fx_tap(pos, vec2<f32>(0.5, 0.5)).y;
    let sw = fx_tap(pos, vec2<f32>(-0.5, 0.5)).y;
    let ne = fx_tap(pos, vec2<f32>(0.5, -0.5)).y + NE_BIAS;
    let luma_max = max(max(nw, sw), max(ne, se));
    let luma_min = min(min(nw, sw), min(ne, se));
    let luma_max_m = max(luma_max, m.y);
    let luma_min_m = min(luma_min, m.y);
    let range = luma_max_m - luma_min_m;
    let threshold = max(EDGE_THRESHOLD * luma_max, EDGE_THRESHOLD_MIN);
    if (threshold > range) {
        return m;
    }
    var dir = vec2<f32>(
        sw * CONST_DIR.x + nw * CONST_DIR.y + ne * CONST_DIR.y + se * CONST_DIR.x,
        se * CONST_DIR.y + sw * CONST_DIR.x + nw * CONST_DIR.x + ne * CONST_DIR.y,
    );
    dir = dir * inverseSqrt(abs(dot(dir, dir)));
    let opt = vec4<f32>(-0.5 * rcp_frame(), 0.5 * rcp_frame());
    let dir1 = pos.xyxy + dir.xyxy * opt;
    let inv_min = 1.0 / (EDGE_SHARPNESS * min(abs(dir.x), abs(dir.y)));
    let k = saturate(dir.xyxy * CONST_DIR.zzww * inv_min + 0.5);
    let o2 = vec4<f32>(8.0 * rcp_frame(), -4.0 * rcp_frame());
    let dir2 = k * o2.xyxy + o2.zwzw + pos.xyxy;
    let n2 = textureSampleLevel(src, linear_clamp, dir2.xy, 0.0) * 0.25;
    let p2 = textureSampleLevel(src, linear_clamp, dir2.zw, 0.0) * 0.25;
    let n1 = textureSampleLevel(src, linear_clamp, dir1.xy, 0.0) * 0.5;
    let p1 = textureSampleLevel(src, linear_clamp, dir1.zw, 0.0) * 0.5;
    let rgby_a = n1 + p1;
    let rgby_b = rgby_a * 0.5 + (n2 + p2);
    var r = select(rgby_b, rgby_a, rgby_b.y - luma_max > 0.0);
    r = select(rgby_a, r, rgby_b.y - luma_min > 0.0);
    return r;
}

@fragment
fn fs_composite(in: FullOut) -> @location(0) vec4<f32> {
    let base = textureLoad(src, vec2<i32>(floor(in.position.xy)), 0).rgb;
    if (u.debug.x > 0.0) {
        return vec4<f32>(to_linear(base), 1.0);
    }
    var lin = base * base;
    if (u.bloom.z != 0.0) {
        let b = textureSampleLevel(bloom_tex, linear_clamp, in.uv, 0.0).rgb;
        lin += b * b * u.bloom.y;
    }
    let g = sqrt(lin);
    let l = textureSampleLevel(lut, linear_clamp, g * (31.0 / 32.0) + 1.0 / 64.0, 0.0).rgb;
    lin = l * l;
    if (u.vignette.w != 0.0) {
        let d = in.uv - vec2<f32>(0.5);
        let t = saturate((sqrt(dot(d, d)) - u.bounds.x) / (u.bounds.y - u.bounds.x));
        lin = mix(lin, u.vignette.rgb, t * t * (3.0 - 2.0 * t));
    }
    let v = clamp(sqrt(lin) * u.bounds.z + u.bounds.w, vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(to_linear(v), 1.0);
}
