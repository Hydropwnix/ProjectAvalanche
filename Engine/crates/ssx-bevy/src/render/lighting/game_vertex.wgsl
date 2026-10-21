#import bevy_pbr::{
    mesh_bindings::mesh,
    mesh_functions,
    skinning,
    morph::{morph_position, morph_normal, morph_tangent},
    forward_io::{Vertex, VertexOutput},
    view_transformations::position_world_to_clip,
}
#import ssx_viewer::game_lighting as gl
#ifdef SSX_TERRAIN_RIPPLE
#import ssx_viewer::terrain_ripple
#endif

#ifdef SSX_LIGHT_BUFFER
@group(#{MATERIAL_BIND_GROUP}) @binding(140) var<storage, read> light_buffer: array<vec4<f32>>;
#else
@group(#{MATERIAL_BIND_GROUP}) @binding(#{SSX_LIGHT_BINDING}) var<uniform> fog_light: gl::GameLight;
#endif

#ifdef MORPH_TARGETS
fn morph_vertex(vertex_in: Vertex, instance_index: u32) -> Vertex {
    var vertex = vertex_in;
    let first_vertex = mesh[instance_index].first_vertex_index;
    let vertex_index = vertex.index - first_vertex;

    let weight_count = bevy_pbr::morph::layer_count(instance_index);
    for (var i: u32 = 0u; i < weight_count; i ++) {
        let weight = bevy_pbr::morph::weight_at(i, instance_index);
        if weight == 0.0 {
            continue;
        }
        vertex.position += weight * morph_position(vertex_index, i, instance_index);
#ifdef VERTEX_NORMALS
        vertex.normal += weight * morph_normal(vertex_index, i, instance_index);
#endif
#ifdef VERTEX_TANGENTS
        vertex.tangent += vec4(weight * morph_tangent(vertex_index, i, instance_index), 0.0);
#endif
    }
    return vertex;
}
#endif

@vertex
fn vertex(vertex_no_morph: Vertex) -> VertexOutput {
    var out: VertexOutput;

#ifdef MORPH_TARGETS
    var vertex = morph_vertex(vertex_no_morph, vertex_no_morph.instance_index);
#else
    var vertex = vertex_no_morph;
#endif

    let mesh_world_from_local = mesh_functions::get_world_from_local(vertex_no_morph.instance_index);

#ifdef SKINNED
    var world_from_local = skinning::skin_model(
        vertex.joint_indices,
        vertex.joint_weights,
        vertex_no_morph.instance_index
    );
#else
    var world_from_local = mesh_world_from_local;
#endif

#ifdef VERTEX_NORMALS
#ifdef SKINNED
    out.world_normal = skinning::skin_normals(world_from_local, vertex.normal);
#else
#ifdef SSX_RAW_NORMAL
    out.world_normal = (world_from_local * vec4<f32>(vertex.normal, 0.0)).xyz;
#else
    out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex_no_morph.instance_index);
#endif
#endif
#endif

#ifdef VERTEX_POSITIONS
    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));
#ifdef SSX_TERRAIN_RIPPLE
#ifdef VERTEX_NORMALS
    let rippled = terrain_ripple::apply(out.world_position.xyz, out.world_normal);
    out.world_normal = rippled[1];
#else
    let rippled = terrain_ripple::apply(out.world_position.xyz, vec3(0.0, 1.0, 0.0));
#endif
    out.world_position = vec4(rippled[0], out.world_position.w);
#endif
    out.position = position_world_to_clip(out.world_position.xyz);
#ifdef SSX_LIGHT_BUFFER
    let at = bitcast<u32>(light_buffer[3].w);
    let fog_light = gl::GameLight(
        light_buffer[at],
        light_buffer[at + 1u],
        light_buffer[at + 2u],
        light_buffer[at + 3u],
        light_buffer[at + 4u],
        light_buffer[at + 5u],
        light_buffer[at + 6u],
        light_buffer[at + 7u],
        light_buffer[at + 8u],
        light_buffer[at + 9u],
        light_buffer[at + 10u],
    );
#endif
    out.world_position.w = gl::fog_amount(fog_light, out.world_position.xyz);
#endif

#ifdef VERTEX_UVS_A
    out.uv = vertex.uv;
#endif
#ifdef VERTEX_UVS_B
    out.uv_b = vertex.uv_b;
#ifdef SSX_INSTANCE_AO
    let tag = mesh_functions::get_tag(vertex_no_morph.instance_index);
    out.uv_b.x *= select(bitcast<f32>(tag - 1u), 1.0, tag == 0u);
#endif
#endif

#ifdef VERTEX_TANGENTS
    out.world_tangent = mesh_functions::mesh_tangent_local_to_world(
        world_from_local,
        vertex.tangent,
        vertex_no_morph.instance_index
    );
#endif

#ifdef VERTEX_COLORS
    out.color = vertex.color;
#endif

#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = vertex_no_morph.instance_index;
#endif

#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(
        vertex_no_morph.instance_index, mesh_world_from_local[3]);
#endif

    return out;
}
