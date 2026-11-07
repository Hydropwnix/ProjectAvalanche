fn surface(input: SurfaceIn, base: Surface, sky_texture: texture_2d<f32>, sky_sampler: sampler) -> Surface {
    return base;
}

fn vertex_offset(input: VertexIn) -> vec3<f32> {
    return vec3<f32>(0.0);
}
