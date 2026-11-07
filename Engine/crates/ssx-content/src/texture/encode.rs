use super::{BlockChain, BlockFormat, BlockLevel, Shape, Texture, TextureKind};

pub const ENCODER: &str = "texpresso 2.0.2 cluster-fit";

fn params(kind: TextureKind) -> texpresso::Params {
    texpresso::Params {
        algorithm: texpresso::Algorithm::ClusterFit,
        weights: match kind {
            TextureKind::Color => texpresso::COLOUR_WEIGHTS_PERCEPTUAL,
            TextureKind::Normal | TextureKind::Data => texpresso::COLOUR_WEIGHTS_UNIFORM,
        },
        weigh_colour_by_alpha: false,
    }
}

fn compress(format: texpresso::Format, image: &[u8], w: usize, h: usize, p: texpresso::Params) -> Vec<u8> {
    let block_rows = h.div_ceil(4);
    let row_bytes = format.compressed_size(w, 4);
    let mut data = vec![0u8; row_bytes * block_rows];
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(block_rows).max(1);
    let per_band = block_rows.div_ceil(threads);
    std::thread::scope(|scope| {
        for (b, out) in data.chunks_mut(row_bytes * per_band).enumerate() {
            let y0 = b * per_band * 4;
            let rows = (h - y0).min(per_band * 4);
            let band = &image[y0 * w * 4..(y0 + rows) * w * 4];
            scope.spawn(move || format.compress(band, w, rows, p, out));
        }
    });
    data
}

pub fn encode_chain(texture: &Texture, format: BlockFormat, kind: TextureKind) -> Result<BlockChain, String> {
    if texture.shape != Shape::Plane {
        return Err(format!("only plane textures are block-encoded, not a {:?}", texture.shape));
    }
    let tex_format = match format {
        BlockFormat::Bc1 => texpresso::Format::Bc1,
        BlockFormat::Bc2 => texpresso::Format::Bc2,
        BlockFormat::Bc3 => texpresso::Format::Bc3,
        BlockFormat::Bc4 => texpresso::Format::Bc4,
        BlockFormat::Bc5 => texpresso::Format::Bc5,
        BlockFormat::Bc7 => return Err("BC7 is not encoded (the disc's formats are BC1 / BC2 / BC3 / BC5)".into()),
    };
    let p = params(kind);
    let mut levels = Vec::with_capacity(texture.levels.len());
    for level in &texture.levels {
        let image = level.images.first().ok_or("a level without an image")?;
        let (w, h) = (level.width as usize, level.height as usize);
        if image.len() != w * h * 4 {
            return Err(format!("a {w}×{h} level of {} bytes", image.len()));
        }
        levels.push(BlockLevel {
            width: level.width,
            height: level.height,
            data: compress(tex_format, image, w, h, p),
        });
    }
    Ok(BlockChain { format, levels })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::texture::bc::decode_level;
    use crate::texture::{Level, mips};

    fn gradient(w: u32, h: u32) -> Texture {
        let image: Vec<u8> =
            (0..w * h).flat_map(|i| [(i % w * 255 / w) as u8, (i / w * 255 / h) as u8, 128, 255]).collect();
        let levels = mips::build_chain(Level { width: w, height: h, images: vec![image] });
        Texture { shape: Shape::Plane, levels, blocks: None, kind: None, source: None }
    }

    #[test]
    fn bands_encode_as_one_pass() {
        let t = gradient(64, 52);
        let image = &t.levels[0].images[0];
        let p = params(TextureKind::Color);
        let mut one = vec![0u8; texpresso::Format::Bc3.compressed_size(64, 52)];
        texpresso::Format::Bc3.compress(image, 64, 52, p, &mut one);
        assert_eq!(compress(texpresso::Format::Bc3, image, 64, 52, p), one);
    }

    #[test]
    fn encoded_levels_decode_close_to_the_source() {
        let t = gradient(64, 32);
        for format in [BlockFormat::Bc1, BlockFormat::Bc3, BlockFormat::Bc5] {
            let chain = encode_chain(&t, format, TextureKind::Data).unwrap();
            assert_eq!(chain.levels.len(), t.levels.len());
            let (l0, b0) = (&t.levels[0], &chain.levels[0]);
            assert_eq!(b0.data.len() as u64, format.level_bytes(64, 32));
            let rgba = decode_level(format, &b0.data, 64, 32).unwrap();
            let channels = if format == BlockFormat::Bc5 { 2 } else { 3 };
            let errors: Vec<i32> = rgba
                .as_chunks::<4>()
                .0
                .iter()
                .zip(l0.images[0].as_chunks::<4>().0)
                .flat_map(|(a, b)| (0..channels).map(move |c| (a[c] as i32 - b[c] as i32).abs()))
                .collect();
            let (max, mean) = (errors.iter().max().unwrap(), errors.iter().sum::<i32>() as f64 / errors.len() as f64);
            assert!(*max <= 16 && mean <= 4.0, "{format:?}: max error {max}, mean {mean:.2}");
        }
        assert!(encode_chain(&t, BlockFormat::Bc7, TextureKind::Color).is_err());
    }
}
