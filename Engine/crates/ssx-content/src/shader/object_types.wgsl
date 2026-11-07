struct SurfaceIn {
    uv: vec2<f32>,
    uv_b: vec2<f32>,
    world_position: vec3<f32>,
    world_normal: vec3<f32>,
    to_eye: vec3<f32>,
    time: f32,
}

struct Surface {
    base_color: vec4<f32>,
    normal_xy: vec2<f32>,
    specular: f32,
    emission: vec3<f32>,
}

struct VertexIn {
    position: vec3<f32>,
    normal: vec3<f32>,
    uv: vec2<f32>,
    time: f32,
}
