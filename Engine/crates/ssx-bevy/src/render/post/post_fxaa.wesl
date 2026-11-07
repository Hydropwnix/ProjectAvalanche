import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;
import package::render::post::post_common::PostUniform;

@group(0) @binding(0) var<uniform> post: PostUniform;
@group(0) @binding(1) var frame_buffer: texture_2d<f32>;
@group(0) @binding(2) var linear_clamp: sampler;

const EDGE_SHARPNESS: f32 = 8.0;
const EDGE_THRESHOLD: f32 = 0.125;
const EDGE_THRESHOLD_MIN: f32 = 0.05;
const CONST_DIR: vec4<f32> = vec4<f32>(1.0, -1.0, 0.25, -0.25);
const NE_BIAS: f32 = 0.0026041667;

fn rcp_frame() -> vec2<f32> {
    return 1.0 / vec2<f32>(post.sizes.xy);
}

fn rcp_frame_opt() -> vec4<f32> {
    return vec4<f32>(-0.5 * rcp_frame(), 0.5 * rcp_frame());
}
fn rcp_frame_opt2_360() -> vec4<f32> {
    return vec4<f32>(8.0 * rcp_frame(), -4.0 * rcp_frame());
}

fn tap(uv: vec2<f32>, off: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(frame_buffer, linear_clamp, uv + off * rcp_frame(), 0.0);
}

@fragment
fn fxaa_pass(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let pos = in.uv;
    let m = tap(pos, vec2<f32>(0.0, 0.0));
    let nw = tap(pos, vec2<f32>(-0.5, -0.5)).y;
    let se = tap(pos, vec2<f32>(0.5, 0.5)).y;
    let sw = tap(pos, vec2<f32>(-0.5, 0.5)).y;
    let ne = tap(pos, vec2<f32>(0.5, -0.5)).y + NE_BIAS;

    let luma_max = max(max(nw, sw), max(ne, se));
    let luma_min = min(min(nw, sw), min(ne, se));
    let luma_max_m = max(luma_max, m.y);
    let luma_min_m = min(luma_min, m.y);
    let range = luma_max_m - luma_min_m;
    let threshold = max(EDGE_THRESHOLD * luma_max, EDGE_THRESHOLD_MIN);
    if (threshold > range) {
        return m;
    }

    var dir = vec2<f32>(
        sw * CONST_DIR.x + nw * CONST_DIR.y + ne * CONST_DIR.y + se * CONST_DIR.x,
        se * CONST_DIR.y + sw * CONST_DIR.x + nw * CONST_DIR.x + ne * CONST_DIR.y,
    );
    dir = dir * inverseSqrt(abs(dot(dir, dir)));

    let dir1 = pos.xyxy + dir.xyxy * rcp_frame_opt();
    let inv_min = 1.0 / (EDGE_SHARPNESS * min(abs(dir.x), abs(dir.y)));
    let k = saturate(dir.xyxy * CONST_DIR.zzww * inv_min + 0.5);
    let o2 = rcp_frame_opt2_360();
    let dir2 = k * o2.xyxy + o2.zwzw + pos.xyxy;

    let n2 = textureSampleLevel(frame_buffer, linear_clamp, dir2.xy, 0.0) * 0.25;
    let p2 = textureSampleLevel(frame_buffer, linear_clamp, dir2.zw, 0.0) * 0.25;
    let n1 = textureSampleLevel(frame_buffer, linear_clamp, dir1.xy, 0.0) * 0.5;
    let p1 = textureSampleLevel(frame_buffer, linear_clamp, dir1.zw, 0.0) * 0.5;

    let rgby_a = n1 + p1;
    let rgby_b = rgby_a * 0.5 + (n2 + p2);
    var r = select(rgby_b, rgby_a, rgby_b.y - luma_max > 0.0);
    r = select(rgby_a, r, rgby_b.y - luma_min > 0.0);
    return r;
}
