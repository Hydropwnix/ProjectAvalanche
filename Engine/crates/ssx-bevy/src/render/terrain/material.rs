use std::collections::HashMap;
use std::path::PathBuf;

use bevy::asset::embedded_asset;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError};
use bevy::shader::ShaderRef;

use crate::render::shadows::{ShadowPassKey, specialize_shadow_pass};

use crate::render::lighting::game::{GameLightAssets, GameLightUniform, GameLightingPlugin};
use crate::render::objects::material::ObjectMaterialPlugin;
use crate::render::snow_tracks::{SnowTrackParams, SnowTrackTextures};
use crate::render::terrain::look::TerrainLook;
use crate::rider::material::CharacterMaterialPlugin;

const SHADER_PATH: &str = "embedded://ssx_bevy/render/terrain/terrain.wgsl";
const PREPASS_SHADER_PATH: &str = "embedded://ssx_bevy/render/terrain/terrain_prepass.wgsl";

pub const SHADER_STATUS: &str = "\
terrain (microcode port): tri-planar max(|n|*7-1.4,0), UDN base normals, <=4 top-down snow types + scatter, \
coverage ramp + smoothstep(N.-fall),\n  albedo tex^2 / snow 0.5, Phong exp+intensity, fades 75-125/440-480 m, \
material pass dl*lerp(rampColor1,1,AO)*albedo + spec*(gsc*sl*AO + env) + glint,\n  \
env = pow(fresnel*cube(R_view), reflectEnvExponent), sparkle (snowSparkle*), AO = COLOR1.z via maxAOAttenuation; \
COLOR0/1 packed as the exe (0x82A04520)\n\
terrain: technique set per face group (unk_a bits 1/2), effect samplers, fog per vertex; \
game vertex normals (unit face normals averaged per mesh, not renormalised); ride snow tracks (DispTilesMap, CPU port)";

pub type TerrainMaterial = ExtendedMaterial<StandardMaterial, TerrainExtension>;

pub struct TerrainMaterialPlugin;

impl Plugin for TerrainMaterialPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "terrain.wgsl");
        embedded_asset!(app, "terrain_prepass.wgsl");
        bevy::shader::load_shader_library!(app, "terrain_ripple.wgsl");
        app.add_plugins((
            MaterialPlugin::<TerrainMaterial>::default(),
            GameLightingPlugin,
            ObjectMaterialPlugin,
            CharacterMaterialPlugin,
        ))
        .add_systems(Update, toggle_fades)
        .add_systems(PostUpdate, terrain_ripple);
    }

    fn finish(&self, app: &mut App) {
        crate::render::terrain::textures::init_block_textures(app);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect, ShaderType)]
pub struct TerrainRippleUniform {
    pub a: Vec4,
    pub b: Vec4,
}

fn ripple_override() -> Option<TerrainRippleUniform> {
    static FIXED: std::sync::OnceLock<Option<TerrainRippleUniform>> = std::sync::OnceLock::new();
    *FIXED.get_or_init(|| {
        let v: Vec<f32> =
            std::env::var("SSX_TERRAIN_RIPPLE").ok()?.split(',').filter_map(|x| x.trim().parse().ok()).collect();
        let v: [f32; 8] = v.try_into().ok()?;
        Some(TerrainRippleUniform { a: Vec4::new(v[0], v[1], v[2], v[3]), b: Vec4::new(v[4], v[5], v[6], v[7]) })
    })
}

fn terrain_ripple(
    ripple: Option<Res<crate::ride::TerrainRipple>>,
    mut materials: ResMut<Assets<TerrainMaterial>>,
    mut last: Local<TerrainRippleUniform>,
) {
    let u = ripple_override().unwrap_or_else(|| {
        ripple.map_or_else(Default::default, |r| {
            let p = r.params;
            TerrainRippleUniform {
                a: Vec4::new(p.amplitude, p.falloff, p.frequency, p.phase),
                b: p.position.extend(p.start),
            }
        })
    });
    if *last == u {
        return;
    }
    if last.a.x == 0.0 && u.a.x > 0.0 {
        info!("terrain ripple at {:.1}: amplitude {:.3}, frequency {:.3}", u.b.truncate(), u.a.x, u.a.z);
    }
    *last = u;
    for (_, m) in materials.iter_mut() {
        m.extension.ripple = u;
    }
}

#[derive(Resource, Clone, Copy, Debug)]
pub struct TerrainFades(pub bool);

fn toggle_fades(
    keys: Res<ButtonInput<KeyCode>>,
    fades: Option<ResMut<TerrainFades>>,
    mut materials: ResMut<Assets<TerrainMaterial>>,
) {
    let Some(mut fades) = fades else { return };
    if !keys.just_pressed(KeyCode::KeyG) {
        return;
    }
    fades.0 = !fades.0;
    for (_, material) in materials.iter_mut() {
        material.extension.params.snow_blend.w = f32::from(u8::from(fades.0));
    }
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
#[bind_group_data(ShadowPassKey)]
pub struct TerrainExtension {
    #[uniform(100)]
    pub params: TerrainParams,
    #[texture(101)]
    pub rock_diffuse_1: Handle<Image>,
    #[texture(102)]
    pub rock_normal_1: Handle<Image>,
    #[texture(111)]
    pub rock_diffuse_2: Handle<Image>,
    #[texture(112)]
    pub rock_normal_2: Handle<Image>,
    #[texture(103, dimension = "2d_array")]
    #[sampler(104)]
    pub snow_normal: Handle<Image>,
    #[texture(105)]
    pub scatter_normal: Handle<Image>,
    #[uniform(106)]
    pub light: GameLightUniform,
    #[storage(140, read_only)]
    pub local_lights: Handle<bevy::render::storage::ShaderBuffer>,
    #[texture(141, dimension = "2d_array")]
    #[sampler(142)]
    pub local_light_textures: Handle<Image>,
    #[texture(107, dimension = "cube")]
    #[sampler(108)]
    pub reflect_cube: Handle<Image>,
    #[texture(109)]
    #[sampler(110)]
    pub sparkle_normal: Handle<Image>,
    #[uniform(113)]
    pub tracks: SnowTrackParams,
    #[texture(114, sample_type = "u_int")]
    pub track_lookup: Handle<Image>,
    #[texture(115, dimension = "2d_array")]
    #[sampler(116)]
    pub track_tiles: Handle<Image>,
    #[uniform(117)]
    pub ripple: TerrainRippleUniform,
    pub shadow_pass: ShadowPassKey,
}

impl From<&TerrainExtension> for ShadowPassKey {
    fn from(e: &TerrainExtension) -> Self {
        e.shadow_pass
    }
}

impl MaterialExtension for TerrainExtension {
    fn vertex_shader() -> ShaderRef {
        crate::render::lighting::game::GAME_VERTEX_PATH.into()
    }

    fn fragment_shader() -> ShaderRef {
        SHADER_PATH.into()
    }

    fn prepass_vertex_shader() -> ShaderRef {
        PREPASS_SHADER_PATH.into()
    }

    fn specialize(
        _pipeline: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        specialize_shadow_pass(descriptor, key.mesh_key, key.bind_group_data);
        crate::render::lighting::game::specialize_game_vertex(descriptor, 106);
        descriptor.vertex.shader_defs.push("SSX_RAW_NORMAL".into());
        descriptor.vertex.shader_defs.push("SSX_TERRAIN_RIPPLE".into());
        Ok(())
    }
}

pub fn shadow_pass() -> ShadowPassKey {
    ShadowPassKey::of("terrain_default", "render_shadowbuffer")
}

#[derive(Clone, Copy, Debug, Default, Reflect, ShaderType)]
pub struct TerrainParams {
    pub base: [Vec4; 2],
    pub snow_tile: [Vec4; 2],
    pub snow_eccentricity: [Vec4; 2],
    pub snow_specular: [Vec4; 2],
    pub planar_scatter: Vec4,
    pub snow_blend: Vec4,
    pub snow_fall: Vec4,
    pub reflect: Vec4,
}

impl TerrainParams {
    pub fn from_look(look: &TerrainLook, fades: bool) -> Self {
        let base = |i: usize| {
            let s = &look.base[i];
            Vec4::new(s.tile, s.normal_scale, s.eccentricity, s.specular_intensity)
        };
        let layers = |f: &dyn Fn(usize) -> f32| [Vec4::new(f(0), f(1), f(2), f(3)), Vec4::new(f(4), f(5), 0.0, 0.0)];
        TerrainParams {
            base: [base(0), base(1)],
            snow_tile: layers(&|i| look.snow[i].tile),
            snow_eccentricity: layers(&|i| look.snow[i].eccentricity),
            snow_specular: layers(&|i| look.snow[i].specular_intensity),
            planar_scatter: Vec4::new(
                look.planar_blend_scale,
                look.planar_blend_scale * look.planar_blend_offset,
                look.scatter_tile,
                look.scatter_amount,
            ),
            snow_blend: Vec4::new(
                look.snow_blend_offset,
                look.snow_blend_point,
                look.snow_blend_range,
                f32::from(u8::from(fades)),
            ),
            snow_fall: Vec3::from_array(look.snow_fall_direction).extend(0.0),
            reflect: Vec4::new(look.reflect_env_exponent, look.reflect_env_fresnel, 0.0, 0.0),
        }
    }
}

const GBUFFER: &str = "prelight_gbuffer_4SnowTypes_2BaseSurfaces";
const MATERIAL: &str = "prelight_material_2BaseSurfaces_Snow";

fn terrain_samplers() -> anyhow::Result<[crate::render::fx_states::SamplerStates; 5]> {
    use crate::render::fx_states::sampler;
    let fx = "terrain_default";
    Ok([
        sampler(fx, MATERIAL, "remapBaseSurface1DiffuseMapSampler")?,
        sampler(fx, GBUFFER, "remapBaseSurface1NormalMapSampler")?,
        sampler(fx, GBUFFER, "snowType1NormalMapSampler")?,
        sampler(fx, GBUFFER, "scatterNormalMapSampler")?,
        sampler(fx, MATERIAL, "sparkleNormalMapSampler")?,
    ])
}

pub struct PreparedLooks {
    looks: Vec<(TerrainParams, LookImages)>,
    images: Vec<Image>,
}

struct LookImages {
    diffuse: [usize; 2],
    normal: [usize; 2],
    snow: usize,
    scatter: usize,
    sparkle: usize,
}

impl PreparedLooks {
    pub fn decode(looks: &[TerrainLook]) -> anyhow::Result<PreparedLooks> {
        use crate::render::terrain::textures::{Texels, array_texture, decode_all, texture_2d};
        let paths = looks.iter().flat_map(|look| {
            let base = look.base.iter().flat_map(|s| [s.diffuse.as_path(), s.normal.as_path()]);
            base.chain(look.snow.iter().map(|l| l.normal.as_path()))
                .chain([look.scatter.as_path(), look.sparkle.as_path()])
        });
        let chains = decode_all(paths)?;
        let states = terrain_samplers()?;
        let [diffuse_s, normal_s, snow_s, scatter_s, _] = states;
        let shared = snow_s.descriptor()?;
        anyhow::ensure!(
            [diffuse_s, normal_s, scatter_s].iter().all(|s| s.descriptor().ok().as_ref() == Some(&shared)),
            "terrain_default.fxo: the base-surface / snow / scatter samplers differ"
        );
        let sampler_of = |kind: usize| -> anyhow::Result<bevy::image::ImageSampler> { states[kind].image_sampler() };
        let mut images = Vec::new();
        let mut index: HashMap<(Vec<PathBuf>, Texels, usize), usize> = HashMap::new();
        let mut image = |layers: Vec<PathBuf>, texels: Texels, array: bool, kind: usize| -> anyhow::Result<usize> {
            if let Some(&i) = index.get(&(layers.clone(), texels, kind)) {
                return Ok(i);
            }
            let layer_chains: Vec<_> = layers.iter().map(|p| &chains[p]).collect();
            images.push(if array {
                array_texture(&layer_chains, texels, sampler_of(kind)?)?
            } else {
                texture_2d(layer_chains[0], texels, sampler_of(kind)?)
            });
            index.insert((layers, texels, kind), images.len() - 1);
            Ok(images.len() - 1)
        };
        let mut out = Vec::with_capacity(looks.len());
        for look in looks {
            let ids = LookImages {
                diffuse: [
                    image(vec![look.base[0].diffuse.clone()], Texels::Linear, false, 0)?,
                    image(vec![look.base[1].diffuse.clone()], Texels::Linear, false, 0)?,
                ],
                normal: [
                    image(vec![look.base[0].normal.clone()], Texels::NormalXy, false, 1)?,
                    image(vec![look.base[1].normal.clone()], Texels::NormalXy, false, 1)?,
                ],
                snow: image(look.snow.iter().map(|l| l.normal.clone()).collect(), Texels::NormalXy, true, 2)?,
                scatter: image(vec![look.scatter.clone()], Texels::Linear, false, 3)?,
                sparkle: image(vec![look.sparkle.clone()], Texels::Linear, false, 4)?,
            };
            let mut params = TerrainParams::from_look(look, true);
            params.reflect.z = scatter_s.lod_bias;
            out.push((params, ids));
        }
        Ok(PreparedLooks { looks: out, images })
    }

    pub fn into_materials(
        self,
        images: &mut Assets<Image>,
        light: &GameLightAssets,
        tracks: &SnowTrackTextures,
    ) -> Vec<[TerrainMaterial; 3]> {
        let handles: Vec<Handle<Image>> = self.images.into_iter().map(|image| images.add(image)).collect();
        let shadow_pass = shadow_pass();
        self.looks
            .into_iter()
            .map(|(params, LookImages { diffuse, normal, snow, scatter, sparkle })| {
                std::array::from_fn(|group| TerrainMaterial {
                    base: StandardMaterial { base_color: Color::WHITE, ..default() },
                    extension: TerrainExtension {
                        params: TerrainParams { snow_fall: params.snow_fall.with_w(group as f32), ..params },
                        rock_diffuse_1: handles[diffuse[0]].clone(),
                        rock_normal_1: handles[normal[0]].clone(),
                        rock_diffuse_2: handles[diffuse[1]].clone(),
                        rock_normal_2: handles[normal[1]].clone(),
                        snow_normal: handles[snow].clone(),
                        scatter_normal: handles[scatter].clone(),
                        light: light.uniform,
                        local_lights: crate::render::lighting::local::LOCAL_LIGHT_BUFFER,
                        local_light_textures: crate::render::lighting::local::LOCAL_LIGHT_TEXTURES,
                        reflect_cube: light.reflect_cube.clone(),
                        sparkle_normal: handles[sparkle].clone(),
                        tracks: tracks.params,
                        track_lookup: tracks.lookup.clone(),
                        track_tiles: tracks.tiles.clone(),
                        ripple: TerrainRippleUniform::default(),
                        shadow_pass,
                    },
                })
            })
            .collect()
    }
}
