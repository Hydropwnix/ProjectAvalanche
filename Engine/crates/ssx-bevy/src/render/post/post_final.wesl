import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;
import package::render::post::post_common::{PostUniform, Exposure, hable};

@group(0) @binding(0) var<uniform> u: PostUniform;
@group(0) @binding(1) var scene: texture_2d<f32>;
@group(0) @binding(2) var bloom: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;
@group(0) @binding(4) var lut: texture_3d<f32>;
@group(0) @binding(5) var<storage, read> exposure: Exposure;

struct OverlayUniform {
    tint0: vec4<f32>,
    tint1: vec4<f32>,
    fadeoffbounds: vec4<f32>,
    uvscale: vec4<f32>,
}
@group(0) @binding(6) var<uniform> overlay: OverlayUniform;
@group(0) @binding(7) var overlay_map0: texture_2d<f32>;
@group(0) @binding(8) var overlay_map1: texture_2d<f32>;
@group(0) @binding(9) var overlay_sampler: sampler;

fn apply_overlay(c: vec3<f32>, uv: vec2<f32>) -> vec3<f32> {
    let q = uv * overlay.uvscale.xy - overlay.uvscale.zw;
    let fob = overlay.fadeoffbounds;
    let span = fob.y - fob.x;
    var t = select(0.0, 1.0, length(q) > fob.x);
    if span != 0.0 {
        t = saturate((length(q) - fob.x) / span);
    }
    let m0 = textureSampleLevel(overlay_map0, overlay_sampler, uv, 0.0);
    let m1 = textureSampleLevel(overlay_map1, overlay_sampler, uv, 0.0);
    let a0 = saturate(m0.a * overlay.tint0.w - 1.0);
    let a1 = saturate(m1.a * overlay.tint1.w - 1.0);
    let x = mix(c, m1.rgb * overlay.tint1.rgb, a1);
    let k = max(t * t * (3.0 - 2.0 * t), fob.z);
    return mix(x * k, m0.rgb * overlay.tint0.rgb, a0);
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

@fragment
fn tonemap_pass(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let p = vec2<i32>(floor(in.position.xy));
    let c = max(textureLoad(scene, p, 0).rgb, vec3<f32>(0.0)) * u.exposure.x;
    let t = hable(c * exposure.exposure, u.hable_a, u.hable_b) * u.hable_b.z;
    return vec4<f32>(sqrt(clamp(t, vec3<f32>(0.0), vec3<f32>(1.0))), 1.0);
}

@fragment
fn composite_pass(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let base = textureLoad(scene, vec2<i32>(floor(in.position.xy)), 0).rgb;
    var lin = base * base;
    if (u.bloom.z != 0.0) {
        let b = textureSampleLevel(bloom, samp, in.uv, 0.0).rgb;
        lin += b * b * u.bloom.y;
    }
    let g = sqrt(lin);
    let l = textureSampleLevel(lut, samp, g * (31.0 / 32.0) + 1.0 / 64.0, 0.0).rgb;
    var v = clamp(l, vec3<f32>(0.0), vec3<f32>(1.0));
    if overlay.fadeoffbounds.w > 0.0 {
        v = clamp(sqrt(max(apply_overlay(l * l, in.uv), vec3<f32>(0.0))), vec3<f32>(0.0), vec3<f32>(1.0));
    }
    return vec4<f32>(srgb_to_linear(v), 1.0);
}
