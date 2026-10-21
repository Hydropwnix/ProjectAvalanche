#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{alpha_discard, main_pass_post_lighting_processing},
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
    mesh_functions,
}
#import ssx_viewer::game_lighting as gl

struct ObjectParams {
    shading: vec4<f32>,
    tiles: vec4<f32>,
    snow: vec4<f32>,
    snow_fall: vec4<f32>,
    scatter: vec4<f32>,
    lod_bias: vec4<f32>,
    alpha_test: vec4<f32>,
}

#ifdef SSX_LOCAL_LIGHTS
var<private> light: gl::GameLight;
#else
@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> light: gl::GameLight;
#endif
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var<uniform> params: ObjectParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var normal_map: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var object_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var specular_map: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(105) var scatter_map: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(106) var sparkle_map: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(107) var sparkle_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(121) var<storage, read> placements: array<vec2<f32>>;
struct LdrTonemap {
    hable_a: vec4<f32>,
    hable_b: vec4<f32>,
    exposure: vec4<f32>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(122) var<storage, read> ldr_tonemap: LdrTonemap;

const FOLIAGE: u32 = 1u;
const ADVANCED: u32 = 2u;
const SHADOW_ONLY: u32 = 3u;
const NEUTRAL_COV: f32 = 2.0 * 128.0 / 255.0 - 1.0;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
#ifdef SSX_LOCAL_LIGHTS
    light = gl::frame_light();
#endif
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    let family = u32(round(params.shading.x));
    if family == SHADOW_ONLY {
        discard;
    }
    let p = in.world_position.xyz;

    let slot = min(mesh_functions::get_tag(in.instance_index), arrayLength(&placements) - 1u);
    let instance = placements[slot];

    var ao_vtx = 1.0;
    var cov = NEUTRAL_COV;
#ifdef VERTEX_UVS_B
    ao_vtx = in.uv_b.x * instance.x;
    cov = 2.0 * in.uv_b.y - 1.0;
#endif

    var tex_n = vec2(0.5);
    var spec_map = 1.0;
#ifdef VERTEX_UVS_A
    tex_n = textureSampleBias(normal_map, object_sampler, in.uv * params.tiles.xy, params.lod_bias.y).xy;
    spec_map = textureSampleBias(specular_map, object_sampler, in.uv * params.tiles.zw, params.lod_bias.z).y;
    pbr_input.material.base_color = gl::diffuse_texel(in.uv, params.lod_bias.x);
#endif
    let scatter = textureSampleBias(scatter_map, object_sampler, p.xz * params.scatter.x, params.lod_bias.w).xyz;
    let sparkle = textureSample(sparkle_map, sparkle_sampler, p.xz * light.sparkle.x).xyz;

    let glow_mask = pbr_input.material.base_color.a;
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);
    if pbr_input.material.base_color.a < params.alpha_test.x {
        discard;
    }
    pbr_input.material.base_color.a = 1.0;
    let d = pbr_input.material.base_color;

    let n_vtx = normalize(in.world_normal);
    var n = n_vtx;
#ifdef VERTEX_TANGENTS
#ifdef VERTEX_UVS_A
    if params.scatter.w > 0.0 {
        let nt = normalize(vec3(tex_n * 2.0 - 1.0, 1.0));
        n = gl::game_tbn(in.world_normal, in.world_tangent, nt);
    }
#endif
#endif
    var ecc = params.shading.z;
    if family == ADVANCED {
        let b = saturate(params.scatter.z + cov);
        let n_snow = normalize(n_vtx + params.scatter.y * (scatter * 2.0 - 1.0));
        n = normalize(mix(n, n_snow, b));
        ecc = mix(ecc, params.snow_fall.w, b);
    }

    let v = gl::to_eye(p);
    let ao = 1.0 + light.specular.w * (ao_vtx - 1.0);
    let ao_tint = mix(light.ramp_shade.rgb, vec3(1.0), ao);
    if params.alpha_test.y > 0.0 {
        let k = saturate(dot(n, light.light_dir.xyz));
        let k_rim = mix(k, gl::rim_term(light, n, v), mix(light.rim_shadow.y, light.rim_shadow.x, k));
        var light_term = light.ramp_lit.w * mix(light.ramp_shade.rgb, light.ramp_lit.rgb, k_rim);
        var albedo_low = d.rgb * d.rgb;
        if family == FOLIAGE {
            light_term += 0.2 * saturate(dot(-n, light.light_dir.xyz)) * light.ramp_lit.rgb;
        } else if family == ADVANCED {
            let x = saturate(cov + params.snow.x + dot(n, -normalize(params.snow_fall.xyz)));
            let t = saturate((x - (params.snow.y - params.snow.z)) / max(2.0 * params.snow.z, 1e-6));
            albedo_low = mix(albedo_low, vec3(0.5), t * t * (3.0 - 2.0 * t));
        }
        let low = gl::apply_fog(light, light_term * ao_tint * albedo_low, in.world_position.w);
        var out_low: FragmentOutput;
#ifdef SSX_LDR_TRANSLUCENT
        let t = ldr_tonemap;
        let x = low * t.exposure.x * t.exposure.y;
        let h = (x * (t.hable_a.x * x + t.hable_a.z) + t.hable_a.w) / (x * (t.hable_a.x * x + t.hable_a.y) + t.hable_b.x)
            - t.hable_b.y;
        out_low.color = vec4(sqrt(abs(h * t.hable_b.z)), instance.y);
#else
        out_low.color = main_pass_post_lighting_processing(pbr_input, vec4(low, instance.y));
#endif
        return out_low;
    }

    let receiver = (pbr_input.flags & MESH_FLAGS_SHADOW_RECEIVER_BIT) != 0u;
    let wp = vec4(p, 1.0);
    let shadow = gl::sun_shadow(wp, n_vtx, gl::view_z(wp), in.position.xy, receiver);
    let lit = gl::key_light(light, n, v, ecc, shadow, gl::contact_shadow(p), 1.0);

    let spec_light = light.specular.rgb * lit.a * ao;
    var albedo = d.rgb * d.rgb;
    let si = params.shading.y * spec_map;
    var spec = si * si;
    var col: vec3<f32>;
    if family == FOLIAGE {
        let trans = 0.2 * saturate(dot(-n, light.light_dir.xyz)) * light.ramp_lit.rgb;
        col = albedo * ao_tint * (lit.rgb + trans) + spec_light * spec;
    } else {
        var glint = 0.0;
        if family == ADVANCED {
            let x = saturate(cov + params.snow.x + dot(n, -normalize(params.snow_fall.xyz)));
            let t = saturate((x - (params.snow.y - params.snow.z)) / max(2.0 * params.snow.z, 1e-6));
            let s = t * t * (3.0 - 2.0 * t);
            albedo = mix(albedo, vec3(0.5), s);
            spec = mix(spec, params.snow.w, s);
            glint = s * gl::snow_glint(light, sparkle * 2.0 - 1.0, n, v, lit.rgb);
        }
        col = lit.rgb * ao_tint * albedo + spec_light * spec + glint;
        col += params.shading.w * glow_mask * albedo / max(light.misc.y, 1e-6);
    }

    let fogged = gl::apply_fog(light, col, in.world_position.w);
    var out: FragmentOutput;
    out.color = main_pass_post_lighting_processing(pbr_input, vec4(fogged, instance.y));
    return out;
}
