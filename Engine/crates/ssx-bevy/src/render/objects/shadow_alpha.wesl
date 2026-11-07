import bevy_pbr::prepass::io as prepass_io;
import bevy_pbr::render::{
    pbr_bindings,
    pbr_types,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(120) var<uniform> shadow_alpha: vec4<f32>;

fn shadow_alpha_test(in: prepass_io::VertexOutput) {
@if(VERTEX_UVS_A) {
    if shadow_alpha.x >= 0.0 {
        let m = pbr_bindings::material;
        var a = m.base_color.a;
        if (m.flags & pbr_types::STANDARD_MATERIAL_FLAGS_BASE_COLOR_TEXTURE_BIT) != 0u {
            let uv = (m.uv_transform * vec3(in.uv, 1.0)).xy;
            a *= textureSample(pbr_bindings::base_color_texture, pbr_bindings::base_color_sampler, uv).a;
        }
        if !(max(a, shadow_alpha.y) > shadow_alpha.x) {
            discard;
        }
    }
}
}

@if(PREPASS_FRAGMENT)
@fragment
fn fragment(in: prepass_io::VertexOutput) -> prepass_io::FragmentOutput {
    shadow_alpha_test(in);
    var out: prepass_io::FragmentOutput;
    return out;
}
@else
@fragment
fn fragment(in: prepass_io::VertexOutput) {
    shadow_alpha_test(in);
}
