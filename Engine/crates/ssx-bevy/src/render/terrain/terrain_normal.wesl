import bevy_pbr::prepass::io as prepass_io;
import package::render::terrain::terrain_gbuffer::{inputs, gbuffer};

@if(PREPASS_FRAGMENT)
@fragment
fn fragment(in: prepass_io::VertexOutput) -> prepass_io::FragmentOutput {
    var out: prepass_io::FragmentOutput;
@if(NORMAL_PREPASS) {
    let n = gbuffer(inputs(in.world_position.xyz, in.color, in.uv, in.uv_b), in.world_normal).normal;
    out.normal = vec4(n * 0.5 + vec3(0.5), 1.0);
}
    return out;
}
@else
@fragment
fn fragment(in: prepass_io::VertexOutput) {}
