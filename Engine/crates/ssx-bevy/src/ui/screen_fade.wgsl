struct Fade {
    v: vec4<f32>,
}

@group(0) @binding(0) var<uniform> f: Fade;
@group(0) @binding(1) var view_texture: texture_2d<f32>;

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

fn to_gamma(c: vec3<f32>) -> vec3<f32> {
    return select(1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, c * 12.92, c <= vec3(0.0031308));
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3(2.4)), c / 12.92, c <= vec3(0.04045));
}

@fragment
fn fs_fade(in: FullOut) -> @location(0) vec4<f32> {
    let c = to_gamma(clamp(textureLoad(view_texture, vec2<i32>(floor(in.position.xy)), 0).rgb, vec3(0.0), vec3(1.0)));
    let a = clamp(f.v.x, 0.0, 1.0);
    let out = mix(c, vec3<f32>(clamp(f.v.y, 0.0, 1.0)), a);
    return vec4<f32>(to_linear(out), 1.0);
}
