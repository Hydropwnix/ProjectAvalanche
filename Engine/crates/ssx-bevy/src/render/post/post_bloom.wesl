import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;
import package::render::post::post_common::{PostUniform, Exposure, hable};

@group(0) @binding(0) var<uniform> u: PostUniform;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;
@group(0) @binding(3) var<storage, read> exposure: Exposure;

@fragment
fn threshold(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let q = textureLoad(src, vec2<i32>(floor(in.position.xy)), 0).rgb;
    let x = q * q * exposure.bloom_exposure;
    let t = hable(x, u.hable_a, u.hable_b) * u.hable_b.z;
    return vec4<f32>(sqrt(max(t, vec3<f32>(0.0))), 1.0);
}

fn tap(uv: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(src, samp, uv, 0.0).rgb;
}

fn blur(uv: vec2<f32>, dir: vec2<f32>, s: vec4<f32>, o: vec4<f32>) -> vec4<f32> {
    let t = dir / vec2<f32>(u.sizes.zw);
    let c = s.x * tap(uv + o.x * t) + s.y * tap(uv + o.y * t) + s.z * tap(uv)
        + s.y * tap(uv + o.z * t) + s.x * tap(uv + o.w * t);
    return vec4<f32>(c, 1.0);
}

@fragment
fn blur_h(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    return blur(in.uv, vec2<f32>(1.0, 0.0), u.blur_h_samples, u.blur_h_offsets);
}

@fragment
fn blur_v(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    return blur(in.uv, vec2<f32>(0.0, 1.0), u.blur_v_samples, u.blur_v_offsets);
}
