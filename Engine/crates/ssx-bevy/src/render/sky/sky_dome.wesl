import bevy_render::view::View;
@if(SSX_HOOK_SURFACE || SSX_HOOK_VERTEX)
import super::hook;
@if(SSX_HOOK_SURFACE)
import package::render::sky::hook_types::{SurfaceIn, Surface};
@if(SSX_HOOK_VERTEX)
import package::render::sky::hook_types::VertexIn;

struct Dome {
    rotation: vec4<f32>,
    scale: vec4<f32>,
    tile: vec4<f32>,
    sun: vec4<f32>,
}

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<uniform> dome: Dome;
@group(0) @binding(2) var sky_texture: texture_2d<f32>;
@group(0) @binding(3) var sky_sampler: sampler;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) direction: vec3<f32>,
    @location(1) uv: vec2<f32>,
}

fn turned(p: vec3<f32>) -> vec3<f32> {
    let c = dome.rotation.x;
    let s = dome.rotation.y;
    return vec3<f32>(c * p.x + s * p.z, p.y, -s * p.x + c * p.z);
}

@vertex
fn vertex(v: VertexInput) -> VertexOutput {
    var p = v.position;
@if(SSX_HOOK_VERTEX)
    p = p + hook::vertex_offset(VertexIn(v.position, v.normal, v.uv, dome.rotation.w));
    let w = turned(p);
    let clip = view.clip_from_view * vec4<f32>((view.view_from_world * vec4<f32>(w, 0.0)).xyz, 1.0);
    var out: VertexOutput;
    out.position = vec4<f32>(clip.xy, 0.0, clip.w);
    out.direction = w;
    out.uv = v.uv;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let texel = textureSample(sky_texture, sky_sampler, in.uv * dome.tile.xy).rgb;
    var color = texel * texel;
@if(SSX_HOOK_SURFACE) {
    let hooked = hook::surface(
        SurfaceIn(normalize(in.direction), in.uv, normalize(dome.sun.xyz), dome.rotation.w),
        Surface(color),
        sky_texture,
        sky_sampler,
    );
    color = hooked.color;
}
    return vec4<f32>(color * dome.scale.rgb * dome.rotation.z, 1.0);
}
