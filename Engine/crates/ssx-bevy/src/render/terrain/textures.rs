use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use anyhow::{Context, Result, bail, ensure};
use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageSampler};
use bevy::log::warn;
use bevy::render::render_resource::{
    Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
};
use ssx_assets::texture::{self, DxnExport};
use ssx_formats::xpr::XprFile;

pub struct MipChain {
    pub levels: Vec<texture::Image>,
    pub blocks: Option<BlockChain>,
}

pub struct BlockChain {
    pub format: ssx_formats::xenos::TextureFormat,
    pub levels: Vec<texture::BlockLevel>,
}

impl MipChain {
    pub fn from_levels(levels: Vec<texture::Image>) -> Self {
        Self { levels, blocks: None }
    }
}

pub static BLOCK_BYTES: AtomicU64 = AtomicU64::new(0);
pub static DECODED_BYTES: AtomicU64 = AtomicU64::new(0);

static BLOCK_TEXTURES: AtomicBool = AtomicBool::new(true);

fn block_textures() -> bool {
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
    use ssx_formats::xenos::TextureFormat as X;
    let desc = &image.texture_descriptor;
    let (xenos, format) = match desc.format {
        TextureFormat::Bc1RgbaUnorm => (X::Dxt1, TextureFormat::Rgba8Unorm),
        TextureFormat::Bc2RgbaUnorm => (X::Dxt23, TextureFormat::Rgba8Unorm),
        TextureFormat::Bc3RgbaUnorm => (X::Dxt45, TextureFormat::Rgba8Unorm),
        TextureFormat::Bc5RgUnorm => (X::Dxn, TextureFormat::Rgba8Unorm),
        TextureFormat::Bc1RgbaUnormSrgb => (X::Dxt1, TextureFormat::Rgba8UnormSrgb),
        TextureFormat::Bc2RgbaUnormSrgb => (X::Dxt23, TextureFormat::Rgba8UnormSrgb),
        TextureFormat::Bc3RgbaUnormSrgb => (X::Dxt45, TextureFormat::Rgba8UnormSrgb),
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
            let [bw, bh] = xenos.blocks(lw, lh);
            let n = (bw * bh * xenos.bytes_per_block()) as usize;
            let blocks = data.get(at..at + n).context("block data shorter than its levels")?.to_vec();
            at += n;
            let level = texture::BlockLevel { width: lw, height: lh, blocks };
            out.extend_from_slice(&texture::decode_block_level(xenos, &level)?.rgba);
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
fn block_format(format: ssx_formats::xenos::TextureFormat, texels: Texels) -> Option<TextureFormat> {
    use ssx_formats::xenos::TextureFormat as X;
    match (texels, format) {
        (Texels::Linear, X::Dxt1) => Some(TextureFormat::Bc1RgbaUnorm),
        (Texels::Linear, X::Dxt23) => Some(TextureFormat::Bc2RgbaUnorm),
        (Texels::Linear, X::Dxt45) => Some(TextureFormat::Bc3RgbaUnorm),
        (Texels::Linear | Texels::NormalXy, X::Dxn) => Some(TextureFormat::Bc5RgUnorm),
        (Texels::ColourSrgb, X::Dxt1) => Some(TextureFormat::Bc1RgbaUnormSrgb),
        (Texels::ColourSrgb, X::Dxt23) => Some(TextureFormat::Bc2RgbaUnormSrgb),
        (Texels::ColourSrgb, X::Dxt45) => Some(TextureFormat::Bc3RgbaUnormSrgb),
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
            data.extend_from_slice(&level.blocks);
        }
    }
    Some((format, data))
}

pub fn decode_xpr(path: &Path) -> Result<MipChain> {
    let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let xpr = XprFile::parse(&data).with_context(|| format!("parsing {}", path.display()))?;
    let res = xpr.resources.first().with_context(|| format!("{}: no resource", path.display()))?;
    let mut levels = texture::decode_resource(&data, &xpr, res, true, DxnExport::Raw)
        .with_context(|| format!("decoding {}", path.display()))?;
    levels.retain(|l| l.face == 0);
    levels.sort_by_key(|l| l.level);
    ensure!(!levels.is_empty(), "{}: no levels", path.display());
    let blocks = texture::resource_blocks(&data, &xpr, res)
        .with_context(|| format!("blocks of {}", path.display()))?
        .map(|(format, levels)| BlockChain { format, levels });
    Ok(MipChain { levels: levels.into_iter().map(|l| l.image).collect(), blocks })
}

pub fn decode_all<'a>(paths: impl IntoIterator<Item = &'a Path>) -> Result<HashMap<PathBuf, MipChain>> {
    let mut unique: Vec<&Path> = paths.into_iter().collect();
    unique.sort();
    unique.dedup();
    std::thread::scope(|scope| {
        let jobs: Vec<_> = unique.iter().map(|&p| (p, scope.spawn(move || decode_xpr(p)))).collect();
        jobs.into_iter()
            .map(|(p, job)| {
                let chain = job.join().map_err(|_| anyhow::anyhow!("decoder thread panicked on {}", p.display()))??;
                Ok((p.to_path_buf(), chain))
            })
            .collect()
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
