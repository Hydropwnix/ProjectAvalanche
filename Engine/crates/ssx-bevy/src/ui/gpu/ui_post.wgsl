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
