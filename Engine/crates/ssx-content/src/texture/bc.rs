use super::BlockFormat;

fn rgb565(c: u16) -> [u8; 3] {
    let (r, g, b) = ((c >> 11) & 31, (c >> 5) & 63, c & 31);
    [((r << 3) | (r >> 2)) as u8, ((g << 2) | (g >> 4)) as u8, ((b << 3) | (b >> 2)) as u8]
}

fn mix(a: u8, b: u8, wa: u32, wb: u32) -> u8 {
    ((a as u32 * wa + b as u32 * wb) / (wa + wb)) as u8
}

pub fn decode_bc1_block(block: &[u8; 8], three_colour: bool) -> [[u8; 4]; 16] {
    let c0 = u16::from_le_bytes([block[0], block[1]]);
    let c1 = u16::from_le_bytes([block[2], block[3]]);
    let (a, b) = (rgb565(c0), rgb565(c1));
    let lerp = |wa, wb| [mix(a[0], b[0], wa, wb), mix(a[1], b[1], wa, wb), mix(a[2], b[2], wa, wb), 255];
    let palette = if c0 > c1 || !three_colour {
        [[a[0], a[1], a[2], 255], [b[0], b[1], b[2], 255], lerp(2, 1), lerp(1, 2)]
    } else {
        [[a[0], a[1], a[2], 255], [b[0], b[1], b[2], 255], lerp(1, 1), [0, 0, 0, 0]]
    };
    let indices = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
    std::array::from_fn(|i| palette[(indices >> (2 * i) & 3) as usize])
}

pub fn decode_bc4_block(block: &[u8; 8]) -> [u8; 16] {
    let (e0, e1) = (block[0], block[1]);
    let palette: [u8; 8] = if e0 > e1 {
        std::array::from_fn(|i| match i {
            0 => e0,
            1 => e1,
            _ => mix(e0, e1, 8 - i as u32, i as u32 - 1),
        })
    } else {
        std::array::from_fn(|i| match i {
            0 => e0,
            1 => e1,
            6 => 0,
            7 => 255,
            _ => mix(e0, e1, 6 - i as u32, i as u32 - 1),
        })
    };
    let mut bits = [0u8; 8];
    bits[..6].copy_from_slice(&block[2..8]);
    let indices = u64::from_le_bytes(bits);
    std::array::from_fn(|i| palette[(indices >> (3 * i) & 7) as usize])
}

pub fn decode_bc2_alpha(block: &[u8; 8]) -> [u8; 16] {
    std::array::from_fn(|i| ((block[i / 2] >> (4 * (i % 2))) & 15) * 17)
}

fn with_alpha(mut colour: [[u8; 4]; 16], alpha: [u8; 16]) -> [[u8; 4]; 16] {
    colour.iter_mut().zip(alpha).for_each(|(t, a)| t[3] = a);
    colour
}

pub fn decode_block(format: BlockFormat, block: &[u8; 16]) -> [[u8; 4]; 16] {
    let (halves, _) = block.as_chunks::<8>();
    let (lo, hi) = (&halves[0], &halves[1]);
    match format {
        BlockFormat::Bc1 => decode_bc1_block(lo, true),
        BlockFormat::Bc2 => with_alpha(decode_bc1_block(hi, false), decode_bc2_alpha(lo)),
        BlockFormat::Bc3 => with_alpha(decode_bc1_block(hi, false), decode_bc4_block(lo)),
        BlockFormat::Bc4 => decode_bc4_block(lo).map(|r| [r, 0, 0, 255]),
        BlockFormat::Bc5 => {
            let (x, y) = (decode_bc4_block(lo), decode_bc4_block(hi));
            std::array::from_fn(|i| [x[i], y[i], 0, 255])
        }
        BlockFormat::Bc7 => {
            let mut out = [0u8; 64];
            bcdec_rs::bc7(block, &mut out, 16);
            std::array::from_fn(|i| [out[4 * i], out[4 * i + 1], out[4 * i + 2], out[4 * i + 3]])
        }
    }
}

pub fn decode_level(format: BlockFormat, data: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let expected = format.level_bytes(width, height);
    if data.len() as u64 != expected {
        let name = format.name();
        return Err(format!("{name} level {width}×{height}: {} bytes of blocks, expected {expected}", data.len()));
    }
    let (w, h) = (width as usize, height as usize);
    let bw = w.div_ceil(4);
    let bpb = format.bytes_per_block();
    let mut out = vec![0u8; w * h * 4];
    for (i, chunk) in data.chunks_exact(bpb).enumerate() {
        let mut block = [0u8; 16];
        block[..bpb].copy_from_slice(chunk);
        let (bx, by) = (i % bw * 4, i / bw * 4);
        for (j, texel) in decode_block(format, &block).into_iter().enumerate() {
            let (x, y) = (bx + j % 4, by + j / 4);
            if x < w && y < h {
                let at = (y * w + x) * 4;
                out[at..at + 4].copy_from_slice(&texel);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bc1_four_and_three_colour_modes() {
        let four = decode_bc1_block(&[0x00, 0xf8, 0x1f, 0x00, 0xe4, 0, 0, 0], true);
        assert_eq!(&four[..4], &[[255, 0, 0, 255], [0, 0, 255, 255], [170, 0, 85, 255], [85, 0, 170, 255]]);
        let three = decode_bc1_block(&[0x1f, 0x00, 0x00, 0xf8, 0xe4, 0, 0, 0], true);
        assert_eq!(&three[2..4], &[[127, 0, 127, 255], [0, 0, 0, 0]]);
        let forced = decode_bc1_block(&[0x1f, 0x00, 0x00, 0xf8, 0xe4, 0, 0, 0], false);
        assert_eq!(forced[3], [170, 0, 85, 255]);
    }

    #[test]
    fn bc4_eight_and_six_value_modes() {
        let eight = decode_bc4_block(&[255, 0, 0x92, 0x24, 0x49, 0x92, 0x24, 0x49]);
        assert_eq!(eight, [218; 16]);
        let idx: u64 = 6 | 7 << 3 | 2 << 6 | 1 << 9;
        let mut block = [0u8, 255, 0, 0, 0, 0, 0, 0];
        block[2..].copy_from_slice(&idx.to_le_bytes()[..6]);
        assert_eq!(&decode_bc4_block(&block)[..4], &[0, 255, 51, 255]);
    }

    #[test]
    fn bc4_bc5_follow_gpu_sampling() {
        let mut block = [0u8; 16];
        block[..2].copy_from_slice(&[10, 10]);
        block[8..10].copy_from_slice(&[20, 20]);
        assert_eq!(decode_block(BlockFormat::Bc5, &block)[0], [10, 20, 0, 255]);
        assert_eq!(decode_block(BlockFormat::Bc4, &block)[0], [10, 0, 0, 255]);
    }

    #[test]
    fn bc7_mode6_solid_and_the_reserved_mode() {
        let mut block = [0u8; 16];
        block[0] = 0x40 | 0x80;
        block[1] = 0xFF;
        block[2] = 0x1F;
        let texels = decode_block(BlockFormat::Bc7, &block);
        assert_eq!(texels, [[0xFE, 0, 0, 0]; 16]);
        assert_eq!(decode_block(BlockFormat::Bc7, &[0u8; 16]), [[0; 4]; 16]);
    }

    #[test]
    fn levels_are_checked_and_cropped() {
        let block = [0x00, 0xf8, 0x1f, 0x00, 0x04, 0, 0, 0];
        let rgba = decode_level(BlockFormat::Bc1, &block, 2, 2).unwrap();
        assert_eq!(rgba, [255, 0, 0, 255, 0, 0, 255, 255, 255, 0, 0, 255, 255, 0, 0, 255]);
        assert!(decode_level(BlockFormat::Bc1, &block, 8, 8).is_err());
        assert!(decode_level(BlockFormat::Bc7, &block, 4, 4).is_err());
    }
}
