#import bevy_pbr::forward_io::VertexOutput
#import bevy_pbr::mesh_view_bindings::{globals, view}

struct GeotagParams {
    base_colour: vec4<f32>,
    exterior: vec4<f32>,
    core: vec4<f32>,
    specular: vec4<f32>,
    pulse: vec4<f32>,
    tile: vec4<f32>,
    light: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: GeotagParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var diffuse: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var diffuse_sampler: sampler;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let time = globals.time;
    let n = normalize(in.world_normal);
    let v = normalize(view.world_position.xyz - in.world_position.xyz);
    let l = normalize(params.light.xyz);
    let ndv = dot(n, v);
    let ext = pow(saturate((1.0 - abs(ndv)) * (params.exterior.z + 1.0)), params.exterior.y) * params.exterior.x;
    let core = pow(saturate(abs(ndv) * (params.core.z + 1.0)), params.core.y) * params.core.x;
    let r = l - 2.0 * dot(l, n) * n;
    let spec = params.specular.x * pow(saturate(dot(r, -v)), params.specular.y) * saturate(dot(l, n));
    let phase = fract(time * 0.15915494 * params.pulse.x + 0.5) * 6.2831855 - 3.1415927;
    let pulse = 1.0 - 0.5 * params.pulse.y + 0.5 * params.pulse.y * sin(phase);
    var uv = vec2(0.0);
#ifdef VERTEX_UVS_A
    uv = in.uv;
#endif
    let st = vec2((uv.x + fract(time * params.pulse.z)) * params.tile.x, uv.y * params.tile.y);
    let t = textureSample(diffuse, diffuse_sampler, st);
    var c0 = vec4(0.0);
#ifdef VERTEX_COLORS
    c0 = in.color;
#endif
    let front = select(0.0, 1.0, ndv >= 0.0);
    let core_term = core * pulse * c0.x;
    let a = saturate(t.g * t.g * c0.y * front + core_term + (ext * pulse + saturate(spec + t.a)) * c0.z);
    let colour = saturate(core_term + params.base_colour.rgb) * a;
    return vec4(colour, a);
}
