use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use anyhow::{Context, Result, bail, ensure};
use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageSampler};
use bevy::log::warn;
use bevy::render::render_resource::{
    Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
};
use ssx_assets::content::GameContent;
use ssx_assets::texture;
use ssx_content::texture::{BlockChain, BlockFormat, Want};
use ssx_content::{ContentId, Layer};

use crate::render::reload::Recipe;

pub struct MipChain {
    pub levels: Vec<texture::Image>,
    pub blocks: Option<BlockChain>,
    pub origin: Option<(ContentId, Layer)>,
}

impl MipChain {
    pub fn from_levels(levels: Vec<texture::Image>) -> Self {
        Self { levels, blocks: None, origin: None }
    }

    pub fn is_mod(&self) -> bool {
        self.origin.as_ref().is_some_and(|(_, l)| *l > Layer::Port)
    }
}

pub static BLOCK_BYTES: AtomicU64 = AtomicU64::new(0);
pub static DECODED_BYTES: AtomicU64 = AtomicU64::new(0);

static BLOCK_TEXTURES: AtomicBool = AtomicBool::new(true);

pub(crate) fn block_textures() -> bool {
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let off = *OFF
        .get_or_init(|| std::env::var("SSX_BC_TEXTURES").is_ok_and(|v| matches!(v.as_str(), "off" | "0" | "false")));
    !off && BLOCK_TEXTURES.load(Ordering::Relaxed)
}

pub fn init_block_textures(app: &mut bevy::app::App) {
    let bc = app
        .world()
        .get_resource::<bevy::image::CompressedImageFormatSupport>()
        .is_some_and(|s| s.0.contains(bevy::image::CompressedImageFormats::BC));
    let fallback = std::env::var("SSX_BC_TEXTURES").is_ok_and(|v| v == "fallback");
    if !bc || fallback {
        BLOCK_TEXTURES.store(false, Ordering::Relaxed);
        use bevy::ecs::schedule::IntoScheduleConfigs;
        app.add_systems(bevy::app::PostUpdate, decode_blocks.after(bevy::asset::AssetEventSystems));
    }
    bevy::log::info!(
        "textures: {}",
        if block_textures() { "the disc's BC blocks on the GPU" } else { "decoded to RGBA8" }
    );
}

fn decode_blocks(
    mut events: bevy::ecs::message::MessageReader<bevy::asset::AssetEvent<Image>>,
    mut images: bevy::ecs::system::ResMut<bevy::asset::Assets<Image>>,
) {
    for event in events.read() {
        let bevy::asset::AssetEvent::Added { id } = event else { continue };
        let Some(mut image) = images.get_mut(*id) else { continue };
        if let Err(e) = image_to_rgba8(&mut image) {
            warn!("texture {id:?}: block image not decoded ({e:#})");
        }
    }
}

fn image_to_rgba8(image: &mut Image) -> Result<()> {
    let desc = &image.texture_descriptor;
    let (blocks, format) = match desc.format {
        TextureFormat::Bc1RgbaUnorm => (BlockFormat::Bc1, TextureFormat::Rgba8Unorm),
        TextureFormat::Bc2RgbaUnorm => (BlockFormat::Bc2, TextureFormat::Rgba8Unorm),
        TextureFormat::Bc3RgbaUnorm => (BlockFormat::Bc3, TextureFormat::Rgba8Unorm),
        TextureFormat::Bc4RUnorm => (BlockFormat::Bc4, TextureFormat::Rgba8Unorm),
        TextureFormat::Bc5RgUnorm => (BlockFormat::Bc5, TextureFormat::Rgba8Unorm),
        TextureFormat::Bc7RgbaUnorm => (BlockFormat::Bc7, TextureFormat::Rgba8Unorm),
        TextureFormat::Bc1RgbaUnormSrgb => (BlockFormat::Bc1, TextureFormat::Rgba8UnormSrgb),
        TextureFormat::Bc2RgbaUnormSrgb => (BlockFormat::Bc2, TextureFormat::Rgba8UnormSrgb),
        TextureFormat::Bc3RgbaUnormSrgb => (BlockFormat::Bc3, TextureFormat::Rgba8UnormSrgb),
        _ => return Ok(()),
    };
    let (w, h, layers, mips) =
        (desc.size.width, desc.size.height, desc.size.depth_or_array_layers, desc.mip_level_count);
    let data = image.data.take().context("no data")?;
    let mut out = Vec::new();
    let mut at = 0;
    for _ in 0..layers {
        for m in 0..mips {
            let (lw, lh) = ((w >> m).max(1), (h >> m).max(1));
            let n = blocks.level_bytes(lw, lh) as usize;
            let level = data.get(at..at + n).context("block data shorter than its levels")?;
            at += n;
            out.extend(ssx_content::texture::bc::decode_level(blocks, level, lw, lh).map_err(anyhow::Error::msg)?);
        }
    }
    image.data = Some(out);
    image.texture_descriptor.format = format;
    Ok(())
}

fn count_bytes(format: TextureFormat, bytes: usize) {
    let counter = if format.is_compressed() { &BLOCK_BYTES } else { &DECODED_BYTES };
    counter.fetch_add(bytes as u64, Ordering::Relaxed);
}

// TODO(guess): the host GPU's block decode stands in for Xenos' texture unit (the disc's DXT1 / DXT2-3 / DXT4-5 / DXN blocks are BC1 / BC2 / BC3 / BC5 byte for byte; palette rounding at most one code value apart, `bc_check`: on the development machine's GPU 75–85 % of the disc's channel values equal the CPU decoder's, the rest one code value off); settle: a console capture of decoded DXT texels.
fn block_format(format: BlockFormat, texels: Texels) -> Option<TextureFormat> {
    match (texels, format) {
        (Texels::Linear, BlockFormat::Bc1) => Some(TextureFormat::Bc1RgbaUnorm),
        (Texels::Linear, BlockFormat::Bc2) => Some(TextureFormat::Bc2RgbaUnorm),
        (Texels::Linear, BlockFormat::Bc3) => Some(TextureFormat::Bc3RgbaUnorm),
        (Texels::Linear, BlockFormat::Bc4) => Some(TextureFormat::Bc4RUnorm),
        (Texels::Linear | Texels::NormalXy, BlockFormat::Bc5) => Some(TextureFormat::Bc5RgUnorm),
        (Texels::Linear, BlockFormat::Bc7) => Some(TextureFormat::Bc7RgbaUnorm),
        (Texels::ColourSrgb, BlockFormat::Bc1) => Some(TextureFormat::Bc1RgbaUnormSrgb),
        (Texels::ColourSrgb, BlockFormat::Bc2) => Some(TextureFormat::Bc2RgbaUnormSrgb),
        (Texels::ColourSrgb, BlockFormat::Bc3) => Some(TextureFormat::Bc3RgbaUnormSrgb),
        _ => None,
    }
}

fn block_upload(chains: &[(&MipChain, usize)], mips: usize, texels: Texels) -> Option<(TextureFormat, Vec<u8>)> {
    if !block_textures() {
        return None;
    }
    let first = chains.first()?.0.blocks.as_ref()?;
    let format = block_format(first.format, texels)?;
    let mut data = Vec::new();
    for &(chain, skip) in chains {
        let b = chain.blocks.as_ref()?;
        if b.format != first.format || b.levels.len() < skip + mips {
            return None;
        }
        let base = &b.levels[skip];
        if base.width % 4 != 0 || base.height % 4 != 0 {
            return None;
        }
        for (k, level) in b.levels[skip..skip + mips].iter().enumerate() {
            let rgba = &chain.levels[skip + k];
            if (level.width, level.height) != (rgba.width, rgba.height) {
                return None;
            }
            data.extend_from_slice(&level.data);
        }
    }
    Some((format, data))
}

pub fn load_chain(big_root: &Path, id: &ContentId, want: &Want) -> Result<MipChain> {
    let found = GameContent::of(big_root).texture_for_gpu(id, want, block_textures())?;
    let t = found.texture;
    let levels: Vec<texture::Image> = t
        .levels
        .into_iter()
        .filter_map(|l| {
            let (width, height) = (l.width, l.height);
            l.images.into_iter().next().map(|rgba| texture::Image { width, height, rgba })
        })
        .collect();
    ensure!(!levels.is_empty(), "{id}: no levels");
    Ok(MipChain { levels, blocks: t.blocks, origin: Some((id.clone(), found.layer)) })
}

pub fn model_image_chain(
    big_root: &Path,
    model: &ContentId,
    image: usize,
    kind: ssx_content::texture::TextureKind,
    want: &Want,
) -> Result<MipChain> {
    let found = GameContent::of(big_root).model_image_for_gpu(model, image, kind, want, block_textures())?;
    let t = found.texture;
    let levels: Vec<texture::Image> = t
        .levels
        .into_iter()
        .filter_map(|l| {
            let (width, height) = (l.width, l.height);
            l.images.into_iter().next().map(|rgba| texture::Image { width, height, rgba })
        })
        .collect();
    ensure!(!levels.is_empty(), "{model} image {image}: no levels");
    Ok(MipChain { levels, blocks: t.blocks, origin: Some((model.clone(), found.layer)) })
}

pub fn rgba_chain(big_root: &Path, id: &ContentId, want: &Want) -> Result<MipChain> {
    let found = GameContent::of(big_root).texture(id, want)?;
    let t = found.texture;
    let levels: Vec<texture::Image> = t
        .levels
        .into_iter()
        .filter_map(|l| {
            let (width, height) = (l.width, l.height);
            l.images.into_iter().next().map(|rgba| texture::Image { width, height, rgba })
        })
        .collect();
    ensure!(!levels.is_empty(), "{id}: no levels");
    Ok(MipChain { levels, blocks: t.blocks, origin: Some((id.clone(), found.layer)) })
}

pub fn decode_all(
    big_root: &Path,
    items: impl IntoIterator<Item = (ContentId, Want)>,
) -> Result<HashMap<(ContentId, Want), MipChain>> {
    let mut unique: Vec<(ContentId, Want)> = items.into_iter().collect();
    unique.sort();
    unique.dedup();
    std::thread::scope(|scope| {
        let jobs: Vec<_> =
            unique.iter().map(|item| (item, scope.spawn(move || load_chain(big_root, &item.0, &item.1)))).collect();
        jobs.into_iter()
            .map(|(item, job)| {
                let chain = job.join().map_err(|_| anyhow::anyhow!("decoder thread panicked on {}", item.0))??;
                Ok((item.clone(), chain))
            })
            .collect()
    })
}

pub fn fit_layers(big_root: &Path, layers: &[&MipChain], want: &Want) -> Vec<Option<MipChain>> {
    let base = layers.iter().filter(|c| !c.is_mod()).map(|c| c.levels[0].width).min();
    let content = GameContent::of(big_root);
    layers
        .iter()
        .map(|chain| {
            let base = base?;
            if !chain.is_mod() || chain.levels.iter().any(|l| l.width == base) {
                return None;
            }
            let (id, _) = chain.origin.clone()?;
            let w = chain.levels[0].width;
            match content.base_texture(&id, want) {
                Ok(found) => {
                    warn!("content: {id} from a mod is {w} wide, without the {base}-wide level its array stacks at: the disc's is used");
                    let levels = found
                        .texture
                        .levels
                        .into_iter()
                        .filter_map(|l| {
                            let (width, height) = (l.width, l.height);
                            l.images.into_iter().next().map(|rgba| texture::Image { width, height, rgba })
                        })
                        .collect();
                    Some(MipChain { levels, blocks: found.texture.blocks, origin: Some((id, Layer::Base)) })
                }
                Err(e) => {
                    warn!("content: {id}: the disc's texture for its array: {e}");
                    None
                }
            }
        })
        .collect()
}

pub fn plane_recipe(big_root: &Path, id: &ContentId, want: Want, texels: Texels, sampler: ImageSampler) -> Recipe {
    let (root, one) = (big_root.to_path_buf(), id.clone());
    Recipe::new(vec![id.clone()], move || Ok(texture_2d(&load_chain(&root, &one, &want)?, texels, sampler.clone())))
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Origin {
    Registry,
    Package(String, ssx_content::Layer),
    Base,
}

pub fn origin_chain(big_root: &Path, id: &ContentId, origin: &Origin, want: &Want) -> Result<MipChain> {
    let content = GameContent::of(big_root);
    let found = match origin {
        Origin::Registry => return load_chain(big_root, id, want),
        Origin::Package(source, layer) => content.texture_for_gpu_from(id, source, *layer, want, block_textures())?,
        Origin::Base => content.base_texture(id, want)?,
    };
    let t = found.texture;
    let levels: Vec<texture::Image> = t
        .levels
        .into_iter()
        .filter_map(|l| {
            let (width, height) = (l.width, l.height);
            l.images.into_iter().next().map(|rgba| texture::Image { width, height, rgba })
        })
        .collect();
    ensure!(!levels.is_empty(), "{id}: no levels");
    Ok(MipChain { levels, blocks: t.blocks, origin: Some((id.clone(), found.layer)) })
}

pub fn source_recipe(
    big_root: &Path,
    src: &ssx_content::material::TextureRef,
    origin: &Origin,
    want: Want,
    texels: Texels,
    sampler: ImageSampler,
) -> Option<Recipe> {
    use ssx_content::material::TextureRef;
    match src {
        TextureRef::Id(id) if *origin == Origin::Registry => Some(plane_recipe(big_root, id, want, texels, sampler)),
        TextureRef::Id(id) => {
            let (root, one, origin) = (big_root.to_path_buf(), id.clone(), origin.clone());
            Some(Recipe::new(vec![id.clone()], move || {
                Ok(texture_2d(&origin_chain(&root, &one, &origin, &want)?, texels, sampler.clone()))
            }))
        }
        TextureRef::ModelImage { model, image, kind } => {
            let (root, model, image, kind) = (big_root.to_path_buf(), model.clone(), *image, *kind);
            let ids = vec![model.clone()];
            Some(Recipe::new(ids, move || {
                Ok(texture_2d(&model_image_chain(&root, &model, image, kind, &want)?, texels, sampler.clone()))
            }))
        }
        TextureRef::Path(_) => None,
    }
}

pub fn source_chain(
    big_root: &Path,
    src: &ssx_content::material::TextureRef,
    origin: &Origin,
    want: &Want,
) -> Result<MipChain> {
    use ssx_content::material::TextureRef;
    match src {
        TextureRef::Id(id) => origin_chain(big_root, id, origin, want),
        TextureRef::ModelImage { model, image, kind } => model_image_chain(big_root, model, *image, *kind, want),
        TextureRef::Path(p) => anyhow::bail!("{p:?} is not a texture id"),
    }
}

pub fn array_recipe(big_root: &Path, ids: &[ContentId], want: Want, texels: Texels, sampler: ImageSampler) -> Recipe {
    let (root, all) = (big_root.to_path_buf(), ids.to_vec());
    Recipe::new(ids.to_vec(), move || {
        let chains = all.iter().map(|id| load_chain(&root, id, &want)).collect::<Result<Vec<_>>>()?;
        let layers: Vec<&MipChain> = chains.iter().collect();
        let fitted = fit_layers(&root, &layers, &want);
        let layers: Vec<&MipChain> = layers.iter().zip(&fitted).map(|(c, f)| f.as_ref().unwrap_or(c)).collect();
        array_texture(&layers, texels, sampler.clone())
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Texels {
    #[allow(dead_code)]
    ColourSrgb,
    NormalXy,
    Linear,
}

impl Texels {
    fn format(self) -> TextureFormat {
        match self {
            Texels::ColourSrgb => TextureFormat::Rgba8UnormSrgb,
            Texels::NormalXy => TextureFormat::Rg8Unorm,
            Texels::Linear => TextureFormat::Rgba8Unorm,
        }
    }

    fn extend(self, data: &mut Vec<u8>, level: &texture::Image) {
        match self {
            Texels::ColourSrgb | Texels::Linear => data.extend_from_slice(&level.rgba),
            Texels::NormalXy => data.extend(level.rgba.as_chunks::<4>().0.iter().flat_map(|t| [t[0], t[1]])),
        }
    }
}

pub fn texture_2d(chain: &MipChain, texels: Texels, sampler: ImageSampler) -> Image {
    let (width, height) = (chain.levels[0].width, chain.levels[0].height);
    let mips = chain
        .levels
        .iter()
        .enumerate()
        .take_while(|(i, l)| l.width == (width >> i).max(1) && l.height == (height >> i).max(1))
        .count();
    let (format, data) = block_upload(&[(chain, 0)], mips, texels).unwrap_or_else(|| {
        let mut data = Vec::new();
        for level in &chain.levels[..mips] {
            texels.extend(&mut data, level);
        }
        (texels.format(), data)
    });
    count_bytes(format, data.len());
    let size = Extent3d { width, height, depth_or_array_layers: 1 };
    let mut image = Image::new_uninit(size, TextureDimension::D2, format, RenderAssetUsages::RENDER_WORLD);
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = mips as u32;
    image.sampler = sampler;
    image
}

pub fn array_texture(layers: &[&MipChain], texels: Texels, sampler: ImageSampler) -> Result<Image> {
    let first = layers.first().context("no texture layers")?;
    let base = layers.iter().map(|c| c.levels[0].width).min().unwrap_or(0);
    if layers.iter().any(|c| c.levels[0].width != base) {
        warn!("texture array layers of different sizes: larger ones lose their top mips");
    }
    let mut trimmed = Vec::with_capacity(layers.len());
    let mut skips = Vec::with_capacity(layers.len());
    for chain in layers {
        let skip = chain.levels.iter().position(|l| l.width == base).with_context(|| {
            format!("cannot stack: no {base}-wide level in a chain starting at {}", chain.levels[0].width)
        })?;
        trimmed.push(&chain.levels[skip..]);
        skips.push((*chain, skip));
    }
    let mips = trimmed.iter().map(|l| l.len()).min().unwrap_or(0);
    let shape = |levels: &[texture::Image]| levels[..mips].iter().map(|l| (l.width, l.height)).collect::<Vec<_>>();
    let expected = shape(trimmed[0]);
    if trimmed.iter().any(|l| shape(l) != expected) {
        bail!("texture layers differ in shape (first layer {}x{})", first.levels[0].width, first.levels[0].height);
    }

    let (format, data) = block_upload(&skips, mips, texels).unwrap_or_else(|| {
        let mut data = Vec::new();
        for levels in &trimmed {
            for level in &levels[..mips] {
                texels.extend(&mut data, level);
            }
        }
        (texels.format(), data)
    });
    count_bytes(format, data.len());
    let size = Extent3d { width: expected[0].0, height: expected[0].1, depth_or_array_layers: layers.len() as u32 };
    let mut image = Image::new_uninit(size, TextureDimension::D2, format, RenderAssetUsages::RENDER_WORLD);
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = mips as u32;
    image.texture_view_descriptor =
        Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::D2Array), ..Default::default() });
    image.sampler = sampler;
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_images_decode_like_the_cpu_path() {
        let block = |k: u8| [0x1F ^ k, 0xF8, 0xE0 | k, 0x07, 0x1B, 0xE4, 0x4E ^ k, 0xB1];
        let shapes = [(8u32, 8u32, 4usize), (4, 4, 1), (2, 2, 1), (1, 1, 1)];
        let levels: Vec<texture::BlockLevel> = shapes
            .iter()
            .enumerate()
            .map(|(m, &(w, h, n))| texture::BlockLevel {
                width: w,
                height: h,
                blocks: (0..n).flat_map(|b| block((m * 4 + b) as u8)).collect(),
            })
            .collect();
        let rgba: Vec<texture::Image> = levels
            .iter()
            .map(|l| texture::decode_block_level(ssx_formats::xenos::TextureFormat::Dxt1, l).unwrap())
            .collect();
        let size = Extent3d { width: 8, height: 8, depth_or_array_layers: 1 };
        let usage = RenderAssetUsages::RENDER_WORLD;
        let mut image = Image::new_uninit(size, TextureDimension::D2, TextureFormat::Bc1RgbaUnorm, usage);
        image.data = Some(levels.iter().flat_map(|l| l.blocks.iter().copied()).collect());
        image.texture_descriptor.mip_level_count = 4;
        image_to_rgba8(&mut image).unwrap();
        assert_eq!(image.texture_descriptor.format, TextureFormat::Rgba8Unorm);
        let want: Vec<u8> = rgba.iter().flat_map(|l| l.rgba.iter().copied()).collect();
        assert_eq!(image.data.unwrap(), want);
    }
}
