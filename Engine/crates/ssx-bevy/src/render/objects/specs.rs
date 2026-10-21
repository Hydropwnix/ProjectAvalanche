use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use bevy::math::Affine2;
use bevy::prelude::*;
use bevy::render::render_resource::Face;
use ssx_assets::track_objects::asset_file;
use ssx_assets::vault_db::Material as VaultMaterial;

use super::load::parallel_map;
use super::{ALPHA_CUTOFF, EffectKind, FOREGROUND_EFFECT, SHADOW_EFFECT};
use crate::render::fx_states::{self, SamplerStates};
use crate::render::lighting::game::GameLightAssets;
use crate::render::objects::foreground::{ForegroundExtension, ForegroundMaterial, ForegroundParams};
use crate::render::objects::material::{
    ObjectExtension, ObjectMaterial, ObjectParams, ObjectShading, PLACEMENT_BUFFER,
};
use crate::render::shadows::ShadowPassKey;
use crate::render::terrain::textures::{MipChain, Texels, decode_xpr, texture_2d};
use crate::rider::material::{CharacterParams, CharacterShading};

#[derive(Clone, Debug)]
pub(super) struct MaterialSpec {
    kind: EffectKind,
    cull: Option<Face>,
    pub(super) shadow_pass: ShadowPassKey,
    pub(super) shadow_alpha: Vec4,
    base_color: Option<usize>,
    pub(super) normal: Option<usize>,
    pub(super) specular: Option<usize>,
    scatter: Option<usize>,
    sparkle: Option<usize>,
    pub(super) ambient: Option<usize>,
    pub(super) character: Option<CharacterParams>,
    pub(super) params: ObjectParams,
    uv_scale: Vec2,
    foreground: Option<ForegroundSpec>,
}

#[derive(Clone, Debug)]
struct ForegroundSpec {
    textures: [usize; 4],
    occlusion: Option<usize>,
    normal: Option<usize>,
    params: ForegroundParams,
}

#[derive(Clone, Debug)]
struct TexFile {
    path: PathBuf,
    sampler: SamplerStates,
}

struct MaterialFiles {
    kind: EffectKind,
    cull: Option<Face>,
    shadow_pass: ShadowPassKey,
    shadow_alpha: Vec4,
    base_color: Option<TexFile>,
    normal: Option<TexFile>,
    specular: Option<TexFile>,
    scatter: Option<TexFile>,
    sparkle: Option<TexFile>,
    ambient: Option<TexFile>,
    character: Option<CharacterParams>,
    foreground: [Option<TexFile>; 5],
    foreground_params: ForegroundParams,
    params: ObjectParams,
    uv_scale: Vec2,
}

fn visible_techniques(effect: &str) -> &'static [&'static str] {
    if effect == FOREGROUND_EFFECT { &["forward"] } else { &["prelight_gbuffer", "prelight_material", "forward"] }
}

impl MaterialFiles {
    fn new(big_root: &Path, m: &VaultMaterial, problems: &mut Vec<String>) -> MaterialFiles {
        let effect = m.effect.as_deref().unwrap_or("");
        let mut kind = EffectKind::of(effect);
        if (effect == "player_apparel.fx" || effect == "player_board.fx")
            && m.float("glowEnable").unwrap_or(0.0) >= ALPHA_CUTOFF
        {
            kind = EffectKind::Opaque;
        }
        let shading = ObjectShading::of(effect);
        let shadow_only = effect == SHADOW_EFFECT;
        let character_shading = CharacterShading::of(effect);
        let player = effect.starts_with("player_");
        let techniques = if shadow_only { &["render_bc_shadowbuffer"][..] } else { visible_techniques(effect) };
        let mut file = |param: &str| -> Option<TexFile> {
            let rel = m.textures.get(param)?;
            let sampler = match fx_states::sampler_in(effect, techniques, &format!("{param}Sampler")) {
                Ok(s) => s,
                Err(e) => {
                    problems.push(format!("{}: {param} not sampled by {effect}: {e:#}", m.name));
                    return None;
                }
            };
            let found = asset_file(big_root, &format!("{rel}.xpr"));
            if found.is_none() {
                problems.push(format!("{}: {param} {rel}.xpr missing", m.name));
            }
            found.map(|path| TexFile { path, sampler })
        };
        let tile = |name: &str| match m.vectors.get(name).map(Vec::as_slice) {
            Some(&[u, v, ..]) => Vec2::new(u, v),
            _ => Vec2::ZERO,
        };
        let mut unset = Vec::new();
        let mut float = |name: &str, default: f32| {
            m.float(name).unwrap_or_else(|| {
                unset.push(name.to_string());
                default
            })
        };
        let mut foreground: [Option<TexFile>; 5] = Default::default();
        let mut foreground_params = ForegroundParams::default();
        let mut params = ObjectParams::default();
        let mut character = None;
        let (base_color, uv_scale) = if kind == EffectKind::Foreground {
            let names = ["mask", "baseChannel", "redChannel", "greenChannel", "ambOcclusion"];
            foreground = names.map(|p| file(&format!("{p}Texture")));
            let [base, red, green, ao] = ["baseChannelTile", "redChannelTile", "greenChannelTile", "aoTile"].map(tile);
            foreground_params = ForegroundParams {
                tiles: Vec4::new(base.x, red.x, green.x, float("snowBias", 0.0)),
                ao_tile: Vec4::new(ao.x, ao.y, 0.0, 0.0),
            };
            (foreground[1].clone(), Vec2::ONE)
        } else {
            let fall = match m.vectors.get("snowFallDirection").map(Vec::as_slice) {
                Some(&[x, y, z, ..]) => Vec3::new(x, y, z),
                _ => Vec3::ZERO,
            };
            let (normal_tile, specular_tile) = (tile("normalTile"), tile("specularTile"));
            params = ObjectParams {
                shading: Vec4::new(
                    shading as u8 as f32,
                    float("specularIntensity", 0.0),
                    float("specularEccentricity", 0.0),
                    if shading == ObjectShading::Foliage || player {
                        0.0
                    } else {
                        float("glowEnable", 0.0) * float("glowIntensity", 0.0)
                    },
                ),
                tiles: Vec4::new(normal_tile.x, normal_tile.y, specular_tile.x, specular_tile.y),
                snow_fall: fall.extend(0.0),
                alpha_test: Vec4::new(if kind == EffectKind::Cutout { ALPHA_CUTOFF } else { 0.0 }, 0.0, 0.0, 0.0),
                ..default()
            };
            character = character_shading.map(|c| {
                let glow = if c == CharacterShading::Apparel {
                    let colour = m.vectors.get("glowColour").map_or(Vec3::ZERO, |v| {
                        Vec3::new(
                            v.first().copied().unwrap_or(0.0),
                            v.get(1).copied().unwrap_or(0.0),
                            v.get(2).copied().unwrap_or(0.0),
                        )
                    });
                    colour * float("glowEnable", 0.0) * float("glowIntensity", 0.0)
                } else {
                    Vec3::ZERO
                };
                let mut p = CharacterParams::new(c, params.shading.y, params.shading.z, glow);
                if matches!(c, CharacterShading::Board | CharacterShading::Secondary)
                    && let Some(fx) = m.effect.as_deref()
                    && let Ok(b) = crate::render::fx_states::param_default(fx, "hologlitchMaxBloomDiffuse")
                {
                    p.holo_bloom = Vec4::new(b[0], b[1], b[2], 0.0);
                }
                if c == CharacterShading::Board {
                    p.board = Vec4::new(
                        float("rimMult", 0.0),
                        float("glowEnable", 0.0) * float("glowIntensity", 0.0),
                        0.0,
                        0.0,
                    );
                }
                p
            });
            if shading == ObjectShading::Advanced {
                params.snow = Vec4::new(
                    float("snowBlendOffset", 0.0),
                    float("snowBlendPoint", 0.0),
                    float("snowBlendRange", 0.0),
                    float("snowSpecularIntensity", 0.0),
                );
                params.snow_fall.w = float("snowEccentricity", 0.0);
                params.scatter = Vec4::new(
                    float("snowScatterTile", 0.0),
                    float("snowScatterAmount", 0.0),
                    float("snowNormalBlendOffset", 0.0),
                    0.0,
                );
            }
            let has_tile = fx_states::effect(effect)
                .is_ok_and(|fx| fx.parameters.iter().any(|p| p.name.as_deref() == Some("diffuseTile")));
            (file("diffuseTexture"), if has_tile { tile("diffuseTile") } else { Vec2::ONE })
        };
        let advanced = shading == ObjectShading::Advanced && kind != EffectKind::Foreground;
        let cull = techniques.iter().find_map(|t| fx_states::cull(effect, t).ok());
        let shadow_technique =
            if character_shading.is_some() { "render_shadowbuffer_GpuSkinned4" } else { "render_bc_shadowbuffer" };
        let glow_enable = m.float("glowEnable").unwrap_or(0.0);
        let files = MaterialFiles {
            kind,
            cull: None,
            shadow_pass: ShadowPassKey::of(effect, shadow_technique),
            shadow_alpha: crate::render::shadows::shadow_alpha(effect, shadow_technique, glow_enable),
            base_color,
            normal: file("normalTexture"),
            specular: if kind == EffectKind::Foreground { None } else { file("specularTexture") },
            scatter: if advanced { file("scatterNormalMap") } else { None },
            sparkle: if advanced { file("sparkleNormalMap") } else { None },
            ambient: if character_shading.is_some() { file("ambOcclusionTexture") } else { None },
            character,
            foreground,
            foreground_params,
            params,
            uv_scale,
        };
        let cull = cull.unwrap_or_else(|| {
            problems.push(format!("{}: no render states for {effect:?}; back faces culled", m.name));
            Some(Face::Back)
        });
        let files = MaterialFiles { cull, ..files };
        if !unset.is_empty() {
            problems.push(format!("{}: unset, effect defaults (0) used: {}", m.name, unset.join(" ")));
        }
        files
    }
}

pub(super) fn prepare_materials(
    big_root: &Path,
    materials: &[&VaultMaterial],
    problems: &mut Vec<String>,
) -> Result<(Vec<Image>, Vec<MaterialSpec>)> {
    let files: Vec<MaterialFiles> = materials.iter().map(|m| MaterialFiles::new(big_root, m, problems)).collect();
    let mut paths: Vec<&Path> = files
        .iter()
        .flat_map(|f| {
            [&f.base_color, &f.normal, &f.specular, &f.scatter, &f.sparkle, &f.ambient].into_iter().chain(&f.foreground)
        })
        .flatten()
        .map(|t| t.path.as_path())
        .collect();
    paths.sort();
    paths.dedup();
    let decoded = parallel_map(&paths, |p| decode_xpr(p));
    let mut chains: HashMap<&Path, MipChain> = HashMap::new();
    for (path, result) in paths.iter().zip(decoded) {
        match result {
            Ok(chain) => {
                chains.insert(path, chain);
            }
            Err(e) => problems.push(format!("{e:#}")),
        }
    }

    let mut images = Vec::new();
    let mut image_ids: HashMap<(PathBuf, Texels, [u32; 8]), usize> = HashMap::new();
    let mut sampler_problems = Vec::new();
    let mut image = |tex: Option<&TexFile>, texels: Texels| -> Option<usize> {
        let tex = tex?;
        let chain = chains.get(tex.path.as_path())?;
        let sampler = match tex.sampler.image_sampler() {
            Ok(s) => s,
            Err(e) => {
                sampler_problems.push(format!("{}: {e:#}", tex.path.display()));
                return None;
            }
        };
        Some(*image_ids.entry((tex.path.clone(), texels, tex.sampler.key())).or_insert_with(|| {
            images.push(texture_2d(chain, texels, sampler));
            images.len() - 1
        }))
    };
    let mut specs = Vec::with_capacity(files.len());
    for f in &files {
        let [mask, base, red, green, occlusion] = &f.foreground;
        let channels = [mask, base, red, green].map(|p| image(p.as_ref(), Texels::Linear));
        let normal = image(f.normal.as_ref(), Texels::NormalXy);
        let foreground = match channels {
            [Some(mask), Some(base), Some(red), Some(green)] => Some(ForegroundSpec {
                textures: [mask, base, red, green],
                occlusion: image(occlusion.as_ref(), Texels::Linear),
                normal,
                params: ForegroundParams {
                    ao_tile: f.foreground_params.ao_tile.with_z(f32::from(u8::from(normal.is_some()))),
                    ..f.foreground_params
                },
            }),
            _ => None,
        };
        let mut params = f.params;
        params.scatter.w = f32::from(u8::from(normal.is_some()));
        let bias = |t: &Option<TexFile>| t.as_ref().map_or(0.0, |t| t.sampler.lod_bias);
        params.lod_bias = Vec4::new(bias(&f.base_color), bias(&f.normal), bias(&f.specular), bias(&f.scatter));
        let character_bias = Vec4::new(bias(&f.base_color), bias(&f.normal), bias(&f.specular), bias(&f.ambient));
        specs.push(MaterialSpec {
            kind: f.kind,
            cull: f.cull,
            shadow_pass: f.shadow_pass,
            shadow_alpha: f.shadow_alpha,
            base_color: image(f.base_color.as_ref(), Texels::Linear),
            normal,
            specular: image(f.specular.as_ref(), Texels::Linear),
            scatter: image(f.scatter.as_ref(), Texels::Linear),
            sparkle: image(f.sparkle.as_ref(), Texels::Linear),
            ambient: image(f.ambient.as_ref(), Texels::Linear),
            character: f.character.map(|c| CharacterParams {
                shading: c.shading.with_w(f32::from(u8::from(normal.is_some()))),
                lod_bias: character_bias,
                ..c
            }),
            params,
            uv_scale: if foreground.is_some() { Vec2::ONE } else { f.uv_scale },
            foreground,
        });
    }
    problems.extend(sampler_problems);
    Ok((images, specs))
}

impl MaterialSpec {
    pub(super) fn standard_material(&self, images: &[Handle<Image>]) -> StandardMaterial {
        let cutout = self.kind == EffectKind::Cutout;
        StandardMaterial {
            base_color: if self.base_color.is_some() { Color::WHITE } else { Color::linear_rgb(0.6, 0.6, 0.6) },
            base_color_texture: self.base_color.map(|i| images[i].clone()),
            alpha_mode: if cutout { AlphaMode::Mask(ALPHA_CUTOFF) } else { AlphaMode::Opaque },
            double_sided: self.cull.is_none(),
            cull_mode: self.cull,
            uv_transform: Affine2::from_scale(self.uv_scale),
            ..default()
        }
    }

    fn object_material(&self, images: &[Handle<Image>], light: &GameLightAssets) -> ObjectMaterial {
        let image_or =
            |i: Option<usize>, fallback: &Handle<Image>| i.map_or_else(|| fallback.clone(), |i| images[i].clone());
        ObjectMaterial {
            base: self.standard_material(images),
            extension: ObjectExtension {
                light: light.uniform,
                params: self.params,
                normal: image_or(self.normal, &light.flat_normal),
                specular: image_or(self.specular, &light.white),
                scatter: image_or(self.scatter, &light.neutral_rgb),
                sparkle: image_or(self.sparkle, &light.neutral_rgb),
                shadow_alpha: self.shadow_alpha,
                placements: PLACEMENT_BUFFER,
                local_lights: crate::render::lighting::local::LOCAL_LIGHT_BUFFER,
                local_light_textures: crate::render::lighting::local::LOCAL_LIGHT_TEXTURES,
                ldr_tonemap: crate::render::objects::material::LDR_TONEMAP_BUFFER,
                shadow_pass: self.shadow_pass,
                ldr: false,
            },
        }
    }

    pub(super) fn add_low(
        &self,
        images: &[Handle<Image>],
        light: &GameLightAssets,
        objects: &mut Assets<ObjectMaterial>,
        blend: bool,
        ldr: bool,
    ) -> Option<Handle<ObjectMaterial>> {
        if self.foreground.is_some() {
            return None;
        }
        let mut material = self.object_material(images, light);
        material.extension.params.alpha_test.y = 1.0;
        if blend {
            material.base.alpha_mode = AlphaMode::Blend;
            material.extension.ldr = ldr;
        }
        Some(objects.add(material))
    }

    pub(super) fn add(
        &self,
        images: &[Handle<Image>],
        light: &GameLightAssets,
        objects: &mut Assets<ObjectMaterial>,
        foreground: &mut Assets<ForegroundMaterial>,
    ) -> MaterialHandle {
        let image_or =
            |i: Option<usize>, fallback: &Handle<Image>| i.map_or_else(|| fallback.clone(), |i| images[i].clone());
        let Some(fg) = &self.foreground else {
            return MaterialHandle::Object(objects.add(self.object_material(images, light)));
        };
        let [mask, base, red, green] = fg.textures.map(|i| images[i].clone());
        let material = ForegroundMaterial {
            base: StandardMaterial { base_color_texture: None, ..self.standard_material(images) },
            extension: ForegroundExtension {
                params: fg.params,
                mask,
                base,
                red,
                green,
                light: light.uniform,
                occlusion: image_or(fg.occlusion, &light.white),
                normal: image_or(fg.normal, &light.flat_normal),
                shadow_pass: self.shadow_pass,
            },
        };
        MaterialHandle::Foreground(foreground.add(material))
    }
}

pub enum MaterialHandle {
    Object(Handle<ObjectMaterial>),
    Foreground(Handle<ForegroundMaterial>),
}
