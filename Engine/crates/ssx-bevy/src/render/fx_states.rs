use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Context, Result, bail};
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerBorderColor, ImageSamplerDescriptor};
use bevy::render::render_resource::{CompareFunction, Face};
use ssx_formats::fxo::{Effect, Pass, RegisterSet, state};

const SHADER_DIRS: [&str; 2] = ["cache1/data/xenon/shaders", "bootcache/data/xenon/shaders"];

struct Library {
    dirs: Vec<PathBuf>,
    effects: Mutex<HashMap<String, Arc<Effect>>>,
}

static LIBRARY: OnceLock<Library> = OnceLock::new();

pub fn init(big_root: &Path) {
    let dirs = SHADER_DIRS.iter().map(|d| big_root.join(d)).filter(|d| d.is_dir()).collect();
    let _ = LIBRARY.set(Library { dirs, effects: Mutex::new(HashMap::new()) });
}

pub fn effect(name: &str) -> Result<Arc<Effect>> {
    let lib = LIBRARY.get().context("effect states unavailable: no extracted archive root")?;
    let stem = name.trim_end_matches(".fxo").trim_end_matches(".fx").to_string();
    let mut cache = lib.effects.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(fx) = cache.get(&stem) {
        return Ok(fx.clone());
    }
    let path = lib
        .dirs
        .iter()
        .map(|d| d.join(format!("{stem}.fxo")))
        .find(|p| p.is_file())
        .with_context(|| format!("{stem}.fxo not found in the shader folders"))?;
    let data = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let fx = Arc::new(Effect::parse(&data).with_context(|| format!("parsing {}", path.display()))?);
    cache.insert(stem, fx.clone());
    Ok(fx)
}

pub fn param_default(name: &str, param: &str) -> Result<[f32; 4]> {
    let fx = effect(name)?;
    let p = fx
        .parameters
        .iter()
        .find(|p| p.name.as_deref() == Some(param))
        .with_context(|| format!("{name}: no parameter {param}"))?;
    let lib = LIBRARY.get().context("effect states unavailable: no extracted archive root")?;
    let stem = name.trim_end_matches(".fxo").trim_end_matches(".fx");
    let path = lib
        .dirs
        .iter()
        .map(|d| d.join(format!("{stem}.fxo")))
        .find(|p| p.is_file())
        .with_context(|| format!("{stem}.fxo not found in the shader folders"))?;
    let data = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let img = data.get(ssx_formats::fxo::IMAGE_BASE..).context("effect file too short")?;
    let word = |at: usize| -> Result<u32> {
        let b = img.get(at..at + 4).with_context(|| format!("{stem}.fxo: value buffer out of range"))?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    };
    let at = word(0x128)? as usize + 16 * usize::from(p.value_offset);
    let mut v = [0.0; 4];
    for (k, x) in v.iter_mut().enumerate() {
        *x = f32::from_bits(word(at + 4 * k)?);
    }
    Ok(v)
}

fn pass<'a>(fx: &'a Effect, effect: &str, technique: &str) -> Result<&'a Pass> {
    let t = fx.technique(technique).with_context(|| format!("{effect}: no technique {technique}"))?;
    t.passes.first().with_context(|| format!("{effect} {technique}: no pass"))
}

fn sampler_slot(fx: &Effect, p: &Pass, name: &str) -> Option<u16> {
    let shaders = [
        p.pixel_shader.and_then(|i| fx.pixel_shaders[i].as_ref()),
        p.vertex_shader.and_then(|i| fx.vertex_shaders[i].as_ref()),
    ];
    shaders
        .into_iter()
        .flatten()
        .flat_map(|s| &s.constants)
        .find_map(|c| (c.register_set == RegisterSet::Sampler && c.name == name).then_some(c.register_index))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SamplerStates {
    pub address: [u32; 3],
    pub mag: u32,
    pub min: u32,
    pub mip: u32,
    pub lod_bias: f32,
    pub max_mip_level: u32,
    pub max_anisotropy: u32,
}

pub fn sampler(effect: &str, technique: &str, sampler: &str) -> Result<SamplerStates> {
    let fx = self::effect(effect)?;
    let p = pass(&fx, effect, technique)?;
    let slot = sampler_slot(&fx, p, sampler).with_context(|| format!("{effect} {technique}: no sampler {sampler}"))?;
    let get =
        |s: u16| p.sampler_state(slot, s).with_context(|| format!("{effect} {technique} {sampler}: state {s:#x}"));
    Ok(SamplerStates {
        address: [get(state::ADDRESSU)?, get(state::ADDRESSV)?, get(state::ADDRESSW)?],
        mag: get(state::MAGFILTER)?,
        min: get(state::MINFILTER)?,
        mip: get(state::MIPFILTER)?,
        lod_bias: f32::from_bits(get(state::MIPMAPLODBIAS)?),
        max_mip_level: get(state::MAXMIPLEVEL)?,
        max_anisotropy: get(state::MAXANISOTROPY)?,
    })
}

pub fn sampler_in(effect: &str, techniques: &[&str], name: &str) -> Result<SamplerStates> {
    let mut last = None;
    for t in techniques {
        match sampler(effect, t, name) {
            Ok(s) => return Ok(s),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("{effect}: no technique given")))
}

impl SamplerStates {
    pub fn key(&self) -> [u32; 8] {
        let [u, v, w] = self.address;
        [u, v, w, self.mag, self.min, self.mip, self.max_mip_level, self.max_anisotropy]
    }

    pub fn descriptor(&self) -> Result<ImageSamplerDescriptor> {
        let address = |v: u32| -> Result<ImageAddressMode> {
            Ok(match v {
                0 => ImageAddressMode::Repeat,
                1 => ImageAddressMode::MirrorRepeat,
                2 => ImageAddressMode::ClampToEdge,
                6 => ImageAddressMode::ClampToBorder,
                _ => bail!("texture address mode {v} not translated"),
            })
        };
        let filter = |v: u32| -> Result<ImageFilterMode> {
            Ok(match v {
                state::FILTER_POINT => ImageFilterMode::Nearest,
                state::FILTER_LINEAR | state::FILTER_ANISOTROPIC => ImageFilterMode::Linear,
                _ => bail!("texture filter {v} not translated"),
            })
        };
        let aniso = [self.mag, self.min].contains(&state::FILTER_ANISOTROPIC);
        let border = self.address.contains(&6);
        Ok(ImageSamplerDescriptor {
            label: None,
            address_mode_u: address(self.address[0])?,
            address_mode_v: address(self.address[1])?,
            address_mode_w: address(self.address[2])?,
            mag_filter: filter(self.mag)?,
            min_filter: filter(self.min)?,
            mipmap_filter: if self.mip == state::FILTER_LINEAR {
                ImageFilterMode::Linear
            } else {
                ImageFilterMode::Nearest
            },
            lod_min_clamp: self.max_mip_level as f32,
            lod_max_clamp: if self.mip == state::FILTER_NONE { self.max_mip_level as f32 } else { 32.0 },
            compare: None,
            anisotropy_clamp: if aniso { self.max_anisotropy.clamp(1, 16) as u16 } else { 1 },
            border_color: border.then_some(ImageSamplerBorderColor::TransparentBlack),
        })
    }

    pub fn image_sampler(&self) -> Result<ImageSampler> {
        Ok(ImageSampler::Descriptor(self.descriptor()?))
    }
}

pub fn cull(effect: &str, technique: &str) -> Result<Option<Face>> {
    let fx = self::effect(effect)?;
    let p = pass(&fx, effect, technique)?;
    match p.render_state(state::CULLMODE) {
        Some(state::CULL_NONE) => Ok(None),
        Some(state::CULL_CW) => Ok(Some(Face::Back)),
        Some(state::CULL_CCW) => Ok(Some(Face::Front)),
        other => bail!("{effect} {technique}: CULLMODE {other:?}"),
    }
}

pub fn render_state(effect: &str, technique: &str, s: u16) -> Result<u32> {
    let fx = self::effect(effect)?;
    pass(&fx, effect, technique)?.render_state(s).with_context(|| format!("{effect} {technique}: state {s:#x}"))
}

pub fn depth_test(effect: &str, technique: &str) -> Result<(CompareFunction, bool)> {
    let get = |s| render_state(effect, technique, s);
    let compare = if get(state::ZENABLE)? == 0 {
        CompareFunction::Always
    } else {
        match get(state::ZFUNC)? {
            0 => CompareFunction::Never,
            1 => CompareFunction::Less,
            2 => CompareFunction::Equal,
            3 => CompareFunction::LessEqual,
            4 => CompareFunction::Greater,
            5 => CompareFunction::NotEqual,
            6 => CompareFunction::GreaterEqual,
            _ => CompareFunction::Always,
        }
    };
    Ok((compare, get(state::ZWRITEENABLE)? != 0))
}

pub fn pixel_shader_reads(effect: &str, technique: &str, name: &str) -> Result<bool> {
    let fx = self::effect(effect)?;
    let p = pass(&fx, effect, technique)?;
    Ok(p.pixel_shader
        .and_then(|i| fx.pixel_shaders[i].as_ref())
        .is_some_and(|s| s.constants.iter().any(|c| c.name == name)))
}
