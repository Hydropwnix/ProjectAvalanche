import bevy_render::view::View;

struct CubeSky {
    rotation: vec4<f32>,
    scale: vec4<f32>,
}

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<uniform> cube_sky: CubeSky;
@group(0) @binding(2) var sky_cube: texture_cube<f32>;
@group(0) @binding(3) var sky_sampler: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) ndc: vec2<f32>,
}

@vertex
fn vertex(@builtin(vertex_index) i: u32) -> VertexOutput {
    let uv = vec2<f32>(f32(i >> 1u), f32(i & 1u)) * 2.0;
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    return VertexOutput(vec4<f32>(ndc, 0.0, 1.0), ndc);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let vp = view.view_from_clip * vec4<f32>(in.ndc, 1.0, 1.0);
    let v = normalize(vp.xyz / vp.w);
    let w = normalize((view.world_from_view * vec4<f32>(v, 0.0)).xyz);
    let c = cube_sky.rotation.x;
    let s = cube_sky.rotation.y;
    let d = vec3<f32>(c * w.x - s * w.z, w.y, s * w.x + c * w.z);
    let texel = textureSample(sky_cube, sky_sampler, d).rgb;
    return vec4<f32>(texel * cube_sky.scale.rgb * cube_sky.rotation.z, 1.0);
}
