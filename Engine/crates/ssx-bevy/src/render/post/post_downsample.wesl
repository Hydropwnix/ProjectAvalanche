import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;
import package::render::post::post_common::PostUniform;

@group(0) @binding(0) var<uniform> u: PostUniform;
@group(0) @binding(1) var scene: texture_2d<f32>;

@fragment
fn downsample(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let q = vec2<i32>(floor(in.position.xy));
    let last = vec2<i32>(u.sizes.xy) - vec2<i32>(1);
    var sum = vec3<f32>(0.0);
    for (var j = 0; j < 4; j++) {
        for (var i = 0; i < 4; i++) {
            let p = min(q * 4 + vec2<i32>(i, j), last);
            let c = max(textureLoad(scene, p, 0).rgb, vec3<f32>(0.0)) * u.exposure.x;
            sum += sqrt(c);
        }
    }
    return vec4<f32>(sum / 16.0, 1.0);
}
