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

@group(0) @binding(0) var full_texture: texture_2d<f32>;
@group(0) @binding(1) var full_sampler: sampler;

fn to_gamma(c: vec3<f32>) -> vec3<f32> {
    return select(1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, c * 12.92, c <= vec3(0.0031308));
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3(2.4)), c / 12.92, c <= vec3(0.04045));
}

@fragment
fn fs_scene_in(in: FullOut) -> @location(0) vec4<f32> {
    let c = textureSampleLevel(full_texture, full_sampler, in.uv, 0.0);
    return vec4(to_gamma(clamp(c.rgb, vec3(0.0), vec3(1.0))), 1.0);
}

@fragment
fn fs_result_out(in: FullOut) -> @location(0) vec4<f32> {
    let c = textureSampleLevel(full_texture, full_sampler, in.uv, 0.0);
    return vec4(to_linear(c.rgb), 1.0);
}

struct FilterParams {
    blur: vec4<f32>,
    color: vec4<f32>,
    shadow: vec4<f32>,
};

@group(1) @binding(0) var<uniform> filter_params: FilterParams;
@group(1) @binding(1) var second_texture: texture_2d<f32>;

@fragment
fn fs_blur(in: FullOut) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(full_texture));
    let half_width = filter_params.blur.z;
    let n = i32(ceil(half_width));
    var sum = vec4(0.0);
    var weight = 0.0;
    for (var k = -n; k <= n; k += 1) {
        let w = select(1.0, half_width - f32(n - 1), abs(k) == n && n > 0);
        let uv = in.uv + filter_params.blur.xy * f32(k) / size;
        sum += textureSampleLevel(full_texture, full_sampler, uv, 0.0) * w;
        weight += w;
    }
    let c = sum / max(weight, 1e-4);
    if filter_params.blur.w > 0.5 {
        return vec4(0.0, 0.0, 0.0, c.a);
    }
    return c;
}

@fragment
fn fs_compose(in: FullOut) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(full_texture));
    let text = textureSampleLevel(full_texture, full_sampler, in.uv, 0.0);
    let p = filter_params.shadow;
    let a = clamp(textureSampleLevel(second_texture, full_sampler, in.uv - p.xy / size, 0.0).a * p.z, 0.0, 1.0);
    let flags = u32(p.w + 0.5);
    var shadow = vec4(filter_params.color.rgb, 1.0) * filter_params.color.a * a;
    if (flags & 1u) != 0u {
        return shadow * (1.0 - text.a);
    }
    if (flags & 2u) != 0u {
        return shadow;
    }
    return text + shadow * (1.0 - text.a);
}
