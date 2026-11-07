use super::bc::decode_level;
use super::{BlockChain, BlockFormat, BlockLevel, Level, Limits, Shape, TextureError, chain_len, check_size};

const MAGIC: &[u8; 4] = b"DDS ";
const HEADER_SIZE: u32 = 124;
const PIXEL_FORMAT_SIZE: u32 = 32;
const DDSD_MIPMAPCOUNT: u32 = 0x2_0000;
const DDPF_ALPHAPIXELS: u32 = 0x1;
const DDPF_ALPHA: u32 = 0x2;
const DDPF_FOURCC: u32 = 0x4;
const DDPF_RGB: u32 = 0x40;
const DDPF_LUMINANCE: u32 = 0x2_0000;
const DDSCAPS2_CUBEMAP: u32 = 0x200;
const DDSCAPS2_ALL_FACES: u32 = 0xFC00;
const DDSCAPS2_VOLUME: u32 = 0x20_0000;
const DX10_TEXTURE2D: u32 = 3;
const DX10_TEXTURECUBE: u32 = 0x4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Texels {
    Blocks(BlockFormat),
    Bytes { bytes: usize, at: [Option<usize>; 4], grey: bool },
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn mask_byte(mask: u32, bytes: usize) -> Result<Option<usize>, String> {
    if mask == 0 {
        return Ok(None);
    }
    (0..bytes).find(|&k| mask == 0xFF << (8 * k)).map(Some).ok_or_else(|| format!("the channel mask {mask:#010x}"))
}

fn legacy(pf: &[u8]) -> Result<Texels, String> {
    let (flags, fourcc, bits) = (u32_at(pf, 4), &pf[8..12], u32_at(pf, 12));
    let masks = [u32_at(pf, 16), u32_at(pf, 20), u32_at(pf, 24), u32_at(pf, 28)];
    if flags & DDPF_FOURCC != 0 {
        return Ok(Texels::Blocks(match fourcc {
            b"DXT1" => BlockFormat::Bc1,
            b"DXT2" | b"DXT3" => BlockFormat::Bc2,
            b"DXT4" | b"DXT5" => BlockFormat::Bc3,
            b"ATI1" | b"BC4U" => BlockFormat::Bc4,
            b"ATI2" | b"BC5U" => BlockFormat::Bc5,
            other => return Err(format!("the FourCC {}", ssx_core::text::quoted(&String::from_utf8_lossy(other)))),
        }));
    }
    let bytes = match bits {
        8 | 16 | 24 | 32 => bits as usize / 8,
        _ => return Err(format!("{bits}-bit texels")),
    };
    let alpha = if flags & (DDPF_ALPHAPIXELS | DDPF_ALPHA) != 0 { masks[3] } else { 0 };
    if flags & DDPF_RGB != 0 {
        let at = [mask_byte(masks[0], bytes)?, mask_byte(masks[1], bytes)?, mask_byte(masks[2], bytes)?];
        if at.iter().any(Option::is_none) {
            return Err(format!("RGB masks {:#x} {:#x} {:#x}", masks[0], masks[1], masks[2]));
        }
        return Ok(Texels::Bytes { bytes, at: [at[0], at[1], at[2], mask_byte(alpha, bytes)?], grey: false });
    }
    if flags & DDPF_LUMINANCE != 0 {
        let l = mask_byte(masks[0], bytes)?.ok_or("a luminance format without a mask")?;
        return Ok(Texels::Bytes { bytes, at: [Some(l), None, None, mask_byte(alpha, bytes)?], grey: true });
    }
    if flags & DDPF_ALPHA != 0 {
        return Ok(Texels::Bytes { bytes, at: [None, None, None, mask_byte(masks[3], bytes)?], grey: false });
    }
    Err(format!("the pixel format flags {flags:#x}"))
}

fn dxgi(format: u32) -> Result<Texels, String> {
    let rgba = |at: [Option<usize>; 4]| Texels::Bytes { bytes: 4, at, grey: false };
    Ok(match format {
        71 | 72 => Texels::Blocks(BlockFormat::Bc1),
        74 | 75 => Texels::Blocks(BlockFormat::Bc2),
        77 | 78 => Texels::Blocks(BlockFormat::Bc3),
        80 => Texels::Blocks(BlockFormat::Bc4),
        83 => Texels::Blocks(BlockFormat::Bc5),
        98 | 99 => Texels::Blocks(BlockFormat::Bc7),
        28 | 29 => rgba([Some(0), Some(1), Some(2), Some(3)]),
        87 | 91 => rgba([Some(2), Some(1), Some(0), Some(3)]),
        88 | 93 => rgba([Some(2), Some(1), Some(0), None]),
        49 => Texels::Bytes { bytes: 2, at: [Some(0), Some(1), None, None], grey: false },
        61 => Texels::Bytes { bytes: 1, at: [Some(0), None, None, None], grey: false },
        65 => Texels::Bytes { bytes: 1, at: [None, None, None, Some(0)], grey: false },
        95 | 96 => return Err(format!("DXGI format {format} (BC6H: HDR texels; no texture of the game takes them)")),
        81 | 84 => return Err(format!("DXGI format {format} (signed BC4 / BC5)")),
        70 | 73 | 76 | 79 | 82 | 94 | 97 => return Err(format!("DXGI format {format} (typeless: save it as UNORM)")),
        _ => return Err(format!("DXGI format {format}")),
    })
}

fn expand(data: &[u8], bytes: usize, at: [Option<usize>; 4], grey: bool) -> Vec<u8> {
    data.chunks_exact(bytes)
        .flat_map(|t| {
            let get = |c: usize, default: u8| at[c].map_or(default, |k| t[k]);
            let r = get(0, 0);
            if grey { [r, r, r, get(3, 255)] } else { [r, get(1, 0), get(2, 0), get(3, 255)] }
        })
        .collect()
}

pub(super) fn size(bytes: &[u8], file: &str) -> Result<(u32, u32, u32), TextureError> {
    let fail = |m: String| TextureError::new(file, format!("DDS: {m}"));
    if bytes.get(..4) != Some(MAGIC) {
        return Err(fail("no `DDS ` magic".into()));
    }
    let header = bytes.get(4..128).ok_or_else(|| fail("shorter than its header".into()))?;
    let (height, width, pf) = (u32_at(header, 8), u32_at(header, 12), &header[72..104]);
    let mut cube = u32_at(header, 108) & DDSCAPS2_CUBEMAP != 0;
    if u32_at(pf, 4) & DDPF_FOURCC != 0 && &pf[8..12] == b"DX10" {
        let dx10 = bytes.get(128..148).ok_or_else(|| fail("shorter than its DX10 header".into()))?;
        cube = u32_at(dx10, 8) & DX10_TEXTURECUBE != 0;
    }
    Ok((width, height, if cube { 6 } else { 1 }))
}

pub(super) fn decode(
    bytes: &[u8],
    file: &str,
    limits: &Limits,
) -> Result<(Shape, Vec<Level>, Option<BlockChain>), TextureError> {
    let fail = |m: String| TextureError::new(file, format!("DDS: {m}"));
    if bytes.get(..4) != Some(MAGIC) {
        return Err(fail("no `DDS ` magic".into()));
    }
    let header = bytes.get(4..128).ok_or_else(|| fail("shorter than its header".into()))?;
    if u32_at(header, 0) != HEADER_SIZE || u32_at(header, 72) != PIXEL_FORMAT_SIZE {
        return Err(fail("a header or pixel-format size other than 124 / 32".into()));
    }
    let (flags, height, width) = (u32_at(header, 4), u32_at(header, 8), u32_at(header, 12));
    let mips = if flags & DDSD_MIPMAPCOUNT != 0 { u32_at(header, 24).max(1) } else { 1 };
    let pf = &header[72..104];
    let caps2 = u32_at(header, 108);
    if caps2 & DDSCAPS2_VOLUME != 0 {
        return Err(fail("a volume texture (only planes and cubes)".into()));
    }
    let mut cube = caps2 & DDSCAPS2_CUBEMAP != 0;
    if cube && caps2 & DDSCAPS2_ALL_FACES != DDSCAPS2_ALL_FACES {
        return Err(fail(format!("a cube without all six faces (caps2 {caps2:#x})")));
    }
    let mut at = 128;
    let texels = if u32_at(pf, 4) & DDPF_FOURCC != 0 && &pf[8..12] == b"DX10" {
        let dx10 = bytes.get(128..148).ok_or_else(|| fail("shorter than its DX10 header".into()))?;
        at = 148;
        let (dimension, misc, array) = (u32_at(dx10, 4), u32_at(dx10, 8), u32_at(dx10, 12));
        if dimension != DX10_TEXTURE2D {
            return Err(fail(format!("resource dimension {dimension} (only 2-D textures)")));
        }
        if array != 1 {
            return Err(fail(format!("an array of {array} (one texture or one cube per file)")));
        }
        cube = misc & DX10_TEXTURECUBE != 0;
        dxgi(u32_at(dx10, 0)).map_err(|m| fail(format!("{m} is not supported")))?
    } else {
        legacy(pf).map_err(|m| fail(format!("{m} is not supported")))?
    };
    let faces = if cube { 6 } else { 1 };
    check_size(file, width, height, faces, limits)?;
    if cube && width != height {
        return Err(fail(format!("cube faces of {width}×{height} (faces are square)")));
    }
    if mips > chain_len(width, height) {
        return Err(fail(format!("{mips} mip levels for {width}×{height}")));
    }
    let size = |l: u32| ((width >> l).max(1), (height >> l).max(1));
    let level_bytes = |l: u32| -> u64 {
        let (w, h) = size(l);
        match texels {
            Texels::Blocks(f) => f.level_bytes(w, h),
            Texels::Bytes { bytes, .. } => u64::from(w) * u64::from(h) * bytes as u64,
        }
    };
    let total: u64 = (0..mips).map(level_bytes).sum::<u64>() * u64::from(faces);
    let rest = (bytes.len() - at) as u64;
    if rest != total {
        return Err(fail(format!("{rest} bytes of texels, the header describes {total}")));
    }
    let mut levels: Vec<Level> =
        (0..mips).map(|l| Level { width: size(l).0, height: size(l).1, images: Vec::new() }).collect();
    let mut blocks = Vec::new();
    for face in 0..faces {
        for l in 0..mips {
            let n = level_bytes(l) as usize;
            let data = &bytes[at..at + n];
            at += n;
            let (w, h) = size(l);
            let rgba = match texels {
                Texels::Blocks(f) => {
                    if face == 0 && !cube {
                        blocks.push(BlockLevel { width: w, height: h, data: data.to_vec() });
                    }
                    decode_level(f, data, w, h).map_err(fail)?
                }
                Texels::Bytes { bytes, at, grey } => expand(data, bytes, at, grey),
            };
            levels[l as usize].images.push(rgba);
        }
    }
    let chain = match texels {
        Texels::Blocks(format) if !cube => Some(BlockChain { format, levels: blocks }),
        _ => None,
    };
    Ok((if cube { Shape::Cube } else { Shape::Plane }, levels, chain))
}
