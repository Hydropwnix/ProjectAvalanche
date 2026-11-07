struct SurfaceIn {
    direction: vec3<f32>,
    uv: vec2<f32>,
    sun_direction: vec3<f32>,
    time: f32,
}

struct Surface {
    color: vec3<f32>,
}

struct VertexIn {
    position: vec3<f32>,
    normal: vec3<f32>,
    uv: vec2<f32>,
    time: f32,
}
