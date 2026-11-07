const GLOW: vec3<f32> = vec3<f32>(0.2, 0.6, 1.0);

fn surface(
    input: SurfaceIn,
    base: Surface,
    diffuse: texture_2d<f32>,
    diffuse_sampler: sampler,
    normal_map: texture_2d<f32>,
    specular_map: texture_2d<f32>,
    map_sampler: sampler,
) -> Surface {
    var out = base;
    let pulse = 0.5 + 0.5 * sin(input.time * 2.0 + input.world_position.x * 0.25);
    out.emission = GLOW * pulse * smoothstep(0.6, 0.9, base.base_color.g);
    return out;
}

fn vertex_offset(input: VertexIn) -> vec3<f32> {
    return vec3<f32>(0.0);
}
