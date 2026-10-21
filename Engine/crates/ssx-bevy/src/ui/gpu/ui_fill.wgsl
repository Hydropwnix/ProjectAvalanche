struct Draw {
    xf0: vec4<f32>,
    xf1: vec4<f32>,
    mult: vec4<f32>,
    add: vec4<f32>,
    paint0: vec4<f32>,
    paint1: vec4<f32>,
    rect: vec4<f32>,
    tex: vec4<f32>,
};

@group(0) @binding(0) var<storage, read> draws: array<Draw>;
@group(1) @binding(0) var paint_texture: texture_2d<f32>;
@group(1) @binding(1) var paint_sampler: sampler;

struct VOut {
    @builtin(position) position: vec4<f32>,
    @location(0) paint: vec2<f32>,
    @location(1) @interpolate(flat) draw: u32,
};

@vertex
fn vs(@location(0) p: vec2<f32>, @location(1) q: vec2<f32>, @location(2) i: u32) -> VOut {
    let d = draws[i];
    let px = vec2(d.xf0.x * p.x + d.xf0.z * p.y + d.xf1.x, d.xf0.y * p.x + d.xf0.w * p.y + d.xf1.y);
    var out: VOut;
    out.position = vec4(px.x / d.xf1.z * 2.0 - 1.0, 1.0 - px.y / d.xf1.w * 2.0, 0.0, 1.0);
    out.paint = vec2(d.paint0.x * q.x + d.paint0.z * q.y + d.paint1.x, d.paint0.y * q.x + d.paint0.w * q.y + d.paint1.y);
    out.draw = i;
    return out;
}

fn focal_t(q: vec2<f32>, f: f32) -> f32 {
    let dq = q - vec2(f, 0.0);
    let a = dot(dq, dq);
    if a <= 0.0 {
        return 0.0;
    }
    let b = 2.0 * f * dq.x;
    let c = f * f - 1.0;
    let k = (-b + sqrt(max(b * b - 4.0 * a * c, 0.0))) / (2.0 * a);
    return select(1.0 / k, 1.0, k <= 0.0);
}

fn spread(t: f32, mode: f32) -> f32 {
    if mode > 1.5 {
        return fract(t);
    }
    if mode > 0.5 {
        let m = t - 2.0 * floor(t / 2.0);
        return select(m, 2.0 - m, m > 1.0);
    }
    return clamp(t, 0.0, 1.0);
}

fn paint(d: Draw, q: vec2<f32>) -> vec4<f32> {
    let kind = u32(d.paint1.z + 0.5);
    switch kind {
        case 0u: {
            return d.rect;
        }
        case 1u, 2u, 3u: {
            var t: f32;
            if kind == 1u {
                t = (q.x + 16384.0) / 32768.0;
            } else if kind == 2u {
                t = length(q) / 16384.0;
            } else {
                t = focal_t(q / 16384.0, d.rect.y);
            }
            let u = (spread(t, d.rect.z) * 255.0 + 0.5) / 256.0;
            return textureSampleLevel(paint_texture, paint_sampler, vec2(u, d.rect.x), 0.0);
        }
        case 4u, 5u: {
            let size = d.rect.zw - d.rect.xy;
            var texel: vec2<f32>;
            if kind == 5u {
                texel = q - size * floor(q / size);
            } else {
                texel = clamp(q, vec2(0.5), max(size - vec2(0.5), vec2(0.5)));
            }
            return textureSampleLevel(paint_texture, paint_sampler, (d.rect.xy + texel) / d.tex.xy, 0.0);
        }
        case 7u: {
            let size = d.rect.zw - d.rect.xy;
            let texel = clamp(q, vec2(0.5), max(size - vec2(0.5), vec2(0.5)));
            let coverage = textureSampleLevel(paint_texture, paint_sampler, (d.rect.xy + texel) / d.tex.xy, 0.0).r;
            return vec4(1.0, 1.0, 1.0, coverage);
        }
        default: {
            return textureSampleLevel(paint_texture, paint_sampler, q / d.tex.xy, 0.0);
        }
    }
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let d = draws[in.draw];
    let c = paint(d, in.paint);
    if u32(d.paint1.z + 0.5) == 6u {
        return c;
    }
    var s = clamp(c * d.mult + d.add, vec4(0.0), vec4(1.0));
    if u32(d.paint1.z + 0.5) == 7u {
        s = vec4(clamp(d.mult + d.add, vec4(0.0), vec4(1.0)).rgb, clamp(d.mult.a + d.add.a, 0.0, 1.0) * c.a);
    }
    let pm = vec4(s.rgb * s.a, s.a);
    if d.paint1.w > 0.5 {
        return vec4(pm.rgb + vec3(1.0 - s.a), pm.a);
    }
    return pm;
}

@fragment
fn fs_mask(in: VOut) -> @location(0) vec4<f32> {
    return vec4(0.0);
}
