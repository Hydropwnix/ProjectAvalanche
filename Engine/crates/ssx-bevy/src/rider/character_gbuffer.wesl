import bevy_pbr::prepass::io as prepass_io;
import bevy_pbr::render::pbr_bindings;
import package::render::lighting::game_lighting as gl;
import package::render::objects::shadow_alpha::shadow_alpha_test;
import package::rider::character_normal::{
    params, normal_map, character_sampler, ps5_alpha, ps5_normal, holo_on, holo_nz, holo_kill,
};

@if(NORMAL_PREPASS)
fn gbuffer(in: prepass_io::VertexOutput) -> vec3<f32> {
    let effect = u32(round(params.shading.x));
    let m = pbr_bindings::material;
    var base = m.base_color;
@if(VERTEX_COLORS)
    base = in.color * base;
    var tex_n = vec2(0.5);
@if(VERTEX_UVS_A) {
    tex_n = textureSampleBias(normal_map, character_sampler, in.uv, params.lod_bias.y).xy;
    base = gl::diffuse_texel(in.uv, params.lod_bias.x);
}
    _ = ps5_alpha(effect, base, m.flags, m.alpha_cutoff);
    var tangent = vec4(0.0);
    var mapped = false;
@if(VERTEX_TANGENTS && VERTEX_UVS_A) {
    tangent = in.world_tangent;
    mapped = params.shading.w > 0.0;
}
    let n = ps5_normal(in.world_normal, tangent, mapped, tex_n);
    if holo_on(effect) {
        holo_kill(holo_nz(n));
    }
    return n;
}

@if(PREPASS_FRAGMENT)
@fragment
fn fragment(in: prepass_io::VertexOutput) -> prepass_io::FragmentOutput {
    var out: prepass_io::FragmentOutput;
@if(NORMAL_PREPASS)
    out.normal = vec4(gbuffer(in) * 0.5 + vec3(0.5), 1.0);
@else
    shadow_alpha_test(in);
    return out;
}
@else
@fragment
fn fragment(in: prepass_io::VertexOutput) {
    shadow_alpha_test(in);
}
