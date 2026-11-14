import package::render::lighting::game_lighting as gl;

struct ObjectParams {
    shading: vec4<f32>,
    tiles: vec4<f32>,
    snow: vec4<f32>,
    snow_fall: vec4<f32>,
    scatter: vec4<f32>,
    lod_bias: vec4<f32>,
    alpha_test: vec4<f32>,
}

const FOLIAGE: u32 = 1u;
const ADVANCED: u32 = 2u;
const SHADOW_ONLY: u32 = 3u;
const NEUTRAL_COV: f32 = 2.0 * 128.0 / 255.0 - 1.0;

struct GbufferNormal {
    n: vec3<f32>,
    snow: f32,
}

fn gbuffer_normal(
    world_normal: vec3<f32>,
    world_tangent: vec4<f32>,
    mapped: bool,
    tex_n: vec2<f32>,
    family: u32,
    params: ObjectParams,
    cov: f32,
    scatter: vec3<f32>,
) -> GbufferNormal {
    let n_vtx = normalize(world_normal);
    var n = n_vtx;
    if mapped {
        let nt = normalize(vec3(tex_n * 2.0 - 1.0, 1.0));
@if(SSX_TANGENT_SIGN)
        n = gl::signed_tbn(world_normal, world_tangent, nt);
@else
        n = gl::game_tbn(world_normal, world_tangent, nt);
    }
    var b = 0.0;
    if family == ADVANCED {
        b = saturate(params.scatter.z + cov);
        let n_snow = normalize(n_vtx + params.scatter.y * (scatter * 2.0 - 1.0));
        n = normalize(mix(n, n_snow, b));
    }
    return GbufferNormal(n, b);
}
