fn surface(
    input: SurfaceIn,
    base: Surface,
    diffuse: texture_2d<f32>,
    diffuse_sampler: sampler,
    normal_map: texture_2d<f32>,
    specular_map: texture_2d<f32>,
    map_sampler: sampler,
) -> Surface {
    return base;
}

fn vertex_offset(input: VertexIn) -> vec3<f32> {
    return vec3<f32>(0.0);
}
