fn surface(input: SurfaceIn, base: Surface, sky_texture: texture_2d<f32>, sky_sampler: sampler) -> Surface {
    var out = base;
    let toward_sun = max(dot(input.direction, input.sun_direction), 0.0);
    out.color = base.color * (1.0 + 0.5 * pow(toward_sun, 8.0));
    return out;
}
