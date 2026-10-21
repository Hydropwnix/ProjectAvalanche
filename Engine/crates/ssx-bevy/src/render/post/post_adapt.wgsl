#import ssx::post_common::{PostUniform, Exposure, LUMA}

@group(0) @binding(0) var<uniform> u: PostUniform;
@group(0) @binding(1) var quarter: texture_2d<f32>;
@group(0) @binding(2) var<storage, read_write> adapted: array<f32>;
@group(0) @binding(3) var<storage, read_write> exposure: Exposure;

const ENC: f32 = 0.0433217;
const DEC: f32 = 23.08312;

fn store16(x: f32) -> f32 {
    let h = unpack2x16float(pack2x16float(vec2<f32>(x, 0.0))).x;
    return round(clamp(h, 0.0, 1.0) * 65535.0) / 65535.0;
}

@compute @workgroup_size(8, 8, 1)
fn adapt(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = u.sizes.zw;
    if (id.x >= size.x || id.y >= size.y) {
        return;
    }
    let q = textureLoad(quarter, vec2<i32>(id.xy), 0).rgb;
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
