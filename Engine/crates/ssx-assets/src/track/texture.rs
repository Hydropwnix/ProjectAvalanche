use std::path::Path;

use anyhow::{Context, Result, bail};
pub use ssx_content::texture::bc::{decode_bc1_block, decode_bc2_alpha, decode_bc4_block};
use ssx_formats::xenos::TextureFormat;
use ssx_formats::xpr::{Swizzle, XprFile, XprResource};

pub fn decode_texel(format: TextureFormat, texel: &[u8]) -> [u8; 4] {
    match format {
        TextureFormat::K8 => [texel[0]; 4],
        TextureFormat::K88 => [texel[0], texel[1], texel[1], texel[1]],
        TextureFormat::K8888 => [texel[0], texel[1], texel[2], texel[3]],
        _ => decode_block(format, texel)[0],
    }
}

pub fn decode_block(format: TextureFormat, block: &[u8]) -> [[u8; 4]; 16] {
    let with_alpha = |mut colour: [[u8; 4]; 16], alpha: [u8; 16]| {
        colour.iter_mut().zip(alpha).for_each(|(t, a)| t[3] = a);
        colour
    };
    let halves = block.as_chunks::<8>().0;
    match format {
        TextureFormat::Dxt1 => decode_bc1_block(&halves[0], true),
        TextureFormat::Dxt23 => with_alpha(decode_bc1_block(&halves[1], false), decode_bc2_alpha(&halves[0])),
        TextureFormat::Dxt45 => with_alpha(decode_bc1_block(&halves[1], false), decode_bc4_block(&halves[0])),
        TextureFormat::Dxn => {
            let (x, y) = (decode_bc4_block(&halves[0]), decode_bc4_block(&halves[1]));
            std::array::from_fn(|i| [x[i], y[i], y[i], y[i]])
        }
        TextureFormat::Dxt5a => decode_bc4_block(&halves[0]).map(|v| [v; 4]),
        TextureFormat::K8 | TextureFormat::K88 | TextureFormat::K8888 => [decode_texel(format, block); 16],
    }
}

pub fn decode_level(format: TextureFormat, blocks: &[u8], width: u32, height: u32) -> Result<Vec<[u8; 4]>> {
    let [bw, bh] = format.blocks(width, height);
    let bpb = format.bytes_per_block() as usize;
    if blocks.len() != bw as usize * bh as usize * bpb {
        bail!("{} level {width}×{height}: {} bytes of blocks, expected {}", format.name(), blocks.len(), bw * bh);
    }
    let (w, h) = (width as usize, height as usize);
    if !format.is_compressed() {
        return Ok(blocks.chunks_exact(bpb).map(|t| decode_texel(format, t)).collect());
    }
    let mut out = vec![[0u8; 4]; w * h];
    for (i, block) in blocks.chunks_exact(bpb).enumerate() {
        let (bx, by) = (i % bw as usize * 4, i / bw as usize * 4);
        for (j, texel) in decode_block(format, block).into_iter().enumerate() {
            let (x, y) = (bx + j % 4, by + j / 4);
            if x < w && y < h {
                out[y * w + x] = texel;
            }
        }
    }
    Ok(out)
}

pub fn apply_swizzle(components: [u8; 4], swizzle: [Swizzle; 4]) -> [u8; 4] {
    swizzle.map(|s| match s {
        Swizzle::X => components[0],
        Swizzle::Y => components[1],
        Swizzle::Z => components[2],
        Swizzle::W => components[3],
        Swizzle::Zero => 0,
        Swizzle::One => 255,
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DxnExport {
    #[default]
    Raw,
    NormalZ,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct TextureLevel {
    pub level: u32,
    pub face: u32,
    pub image: Image,
}

fn finish_texel(format: TextureFormat, swizzle: [Swizzle; 4], dxn: DxnExport, raw: [u8; 4]) -> [u8; 4] {
    let [r, g, b, a] = apply_swizzle(raw, swizzle);
    if format != TextureFormat::Dxn {
        return [r, g, b, a];
    }
    let blue = match dxn {
        DxnExport::Raw => 0,
        DxnExport::NormalZ => {
            let (x, y) = (r as f32 / 127.5 - 1.0, g as f32 / 127.5 - 1.0);
            let z = (1.0 - x * x - y * y).max(0.0).sqrt();
            (z * 127.5 + 127.5).round() as u8
        }
    };
    [r, g, blue, 255]
}

pub fn decode_resource(
    file: &[u8],
    xpr: &XprFile,
    res: &XprResource,
    all_mips: bool,
    dxn: DxnExport,
) -> Result<Vec<TextureLevel>> {
    let fetch = &res.fetch;
    let mut memory = xpr.data(file).to_vec();
    fetch.endian.apply(&mut memory);
    let finish = |texels: Vec<[u8; 4]>| -> Vec<u8> {
        texels.into_iter().flat_map(|t| finish_texel(fetch.format, fetch.swizzle, dxn, t)).collect()
    };
    if let Some(volume) = fetch.linear_volume() {
        let base = memory.get(fetch.base_address as usize..).context("texture address past the data section")?;
        let (w, h, d) = (volume.width as usize, volume.height as usize, volume.depth as usize);
        let mut rgba = vec![0u8; w * d * h * 4];
        for z in 0..volume.depth {
            let blocks = volume.extract_slice(base, z).with_context(|| format!("{} slice {z}", res.name))?;
            let slice = finish(decode_level(fetch.format, &blocks, volume.width, volume.height)?);
            for (y, row) in slice.chunks_exact(w * 4).enumerate() {
                let at = (y * w * d + z as usize * w) * 4;
                rgba[at..at + w * 4].copy_from_slice(row);
            }
        }
        let image = Image { width: (w * d) as u32, height: h as u32, rgba };
        return Ok(vec![TextureLevel { level: 0, face: 0, image }]);
    }
    let Some(surface) = fetch.surface_2d() else {
        bail!("{}: {:?} textures are not decoded", res.name, fetch.dimension);
    };
    let mut out = Vec::new();
    for placement in surface.levels().iter().take(if all_mips { usize::MAX } else { 1 }) {
        let start = if placement.level == 0 { fetch.base_address } else { fetch.mip_address } as usize;
        let region = memory.get(start..).context("texture address past the data section")?;
        for face in 0..surface.array_size {
            let blocks = surface
                .extract(region, placement, face)
                .with_context(|| format!("{} level {} face {face}", res.name, placement.level))?;
            let texels = decode_level(fetch.format, &blocks, placement.width, placement.height)?;
            let image = Image { width: placement.width, height: placement.height, rgba: finish(texels) };
            out.push(TextureLevel { level: placement.level, face, image });
        }
    }
    Ok(out)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockLevel {
    pub width: u32,
    pub height: u32,
    pub blocks: Vec<u8>,
}

pub fn resource_blocks(
    file: &[u8],
    xpr: &XprFile,
    res: &XprResource,
) -> Result<Option<(TextureFormat, Vec<BlockLevel>)>> {
    let fetch = &res.fetch;
    let identity = [Swizzle::X, Swizzle::Y, Swizzle::Z, Swizzle::W];
    let gpu_format =
        matches!(fetch.format, TextureFormat::Dxt1 | TextureFormat::Dxt23 | TextureFormat::Dxt45 | TextureFormat::Dxn);
    if !gpu_format || fetch.swizzle != identity || fetch.linear_volume().is_some() {
        return Ok(None);
    }
    let Some(surface) = fetch.surface_2d() else { return Ok(None) };
    if surface.array_size != 1 {
        return Ok(None);
    }
    let mut memory = xpr.data(file).to_vec();
    fetch.endian.apply(&mut memory);
    let mut out = Vec::new();
    for placement in surface.levels() {
        let start = if placement.level == 0 { fetch.base_address } else { fetch.mip_address } as usize;
        let region = memory.get(start..).context("texture address past the data section")?;
        let blocks = surface
            .extract(region, &placement, 0)
            .with_context(|| format!("{} level {}", res.name, placement.level))?;
        out.push(BlockLevel { width: placement.width, height: placement.height, blocks });
    }
    Ok(Some((fetch.format, out)))
}

pub fn decode_block_level(format: TextureFormat, level: &BlockLevel) -> Result<Image> {
    let identity = [Swizzle::X, Swizzle::Y, Swizzle::Z, Swizzle::W];
    let texels = decode_level(format, &level.blocks, level.width, level.height)?;
    let rgba = texels.into_iter().flat_map(|t| finish_texel(format, identity, DxnExport::Raw, t)).collect();
    Ok(Image { width: level.width, height: level.height, rgba })
}

pub fn write_png(path: &Path, image: &Image) -> Result<()> {
    let file = std::fs::File::create(path).with_context(|| format!("creating {}", path.display()))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&image.rgba)?;
    writer.finish()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssx_formats::xenos::tile_2d;

    #[test]
    fn bc2_alpha_nibbles_and_component_replication() {
        assert_eq!(&decode_bc2_alpha(&[0x21, 0xf0, 0, 0, 0, 0, 0, 0])[..4], &[17, 34, 0, 255]);
        let mut dxn = [0u8; 16];
        dxn[..2].copy_from_slice(&[10, 10]);
        dxn[8..10].copy_from_slice(&[20, 20]);
        assert_eq!(decode_block(TextureFormat::Dxn, &dxn)[0], [10, 20, 20, 20]);
        assert_eq!(decode_texel(TextureFormat::K8, &[7]), [7; 4]);
        assert_eq!(decode_texel(TextureFormat::K88, &[1, 2]), [1, 2, 2, 2]);
        let argb = apply_swizzle([1, 2, 3, 4], [Swizzle::Z, Swizzle::Y, Swizzle::X, Swizzle::W]);
        assert_eq!(argb, [3, 2, 1, 4]);
        assert_eq!(apply_swizzle([9; 4], [Swizzle::X, Swizzle::Zero, Swizzle::X, Swizzle::One]), [9, 0, 9, 255]);
    }

    #[test]
    fn decode_level_crops_partial_blocks() {
        let block = [0x00, 0xf8, 0x1f, 0x00, 0x04, 0, 0, 0];
        let texels = decode_level(TextureFormat::Dxt1, &block, 2, 2).unwrap();
        assert_eq!(texels, [[255, 0, 0, 255], [0, 0, 255, 255], [255, 0, 0, 255], [255, 0, 0, 255]]);
        assert!(decode_level(TextureFormat::Dxt1, &block, 8, 8).is_err());
    }

    #[test]
    fn decodes_a_synthetic_tiled_xpr() {
        let gradient: Vec<u8> = (0..32 * 32).map(|i| ((i % 32) * 8 + i / 32) as u8).collect();
        let mut memory = vec![0u8; 4096];
        tile_2d(&gradient, &mut memory, 32, 1, [0, 0], [32, 32]).unwrap();
        let fetch: [u32; 6] = [0x8040_0002, 0x0000_0002, 0x0003_e01f, 0x0000_1400, 0, 0x0000_0a00];
        let mut header = 1u32.to_be_bytes().to_vec();
        header.extend(b"TX2D");
        [32u32, 52, 24, 0].iter().for_each(|v| header.extend(v.to_be_bytes()));
        header.extend(b"g\0\0\0\0\0\0\0");
        ssx_formats::xpr::D3D_RESOURCE_HEADER.iter().chain(&fetch).for_each(|v| header.extend(v.to_be_bytes()));
        header.resize(header.len().next_multiple_of(16), 0);
        let mut file = b"XPR2".to_vec();
        file.extend((header.len() as u32).to_be_bytes());
        file.extend(4096u32.to_be_bytes());
        file.extend(header);
        file.extend(memory);

        let xpr = XprFile::parse(&file).unwrap();
        let levels = decode_resource(&file, &xpr, &xpr.resources[0], true, DxnExport::Raw).unwrap();
        assert_eq!(levels.len(), 1);
        let image = &levels[0].image;
        assert_eq!((image.width, image.height), (32, 32));
        let expected: Vec<u8> = gradient.iter().flat_map(|&v| [v, v, v, 255]).collect();
        assert_eq!(image.rgba, expected);
    }
}
