use std::io::Read;

use super::bc::decode_level;
use super::{BlockChain, BlockFormat, BlockLevel, Level, Limits, Shape, TextureError, chain_len, check_size};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Texels {
    Blocks { format: BlockFormat, rgb_only: bool },
    Bytes { bytes: usize, at: [Option<usize>; 4] },
}

fn texels(format: u32) -> Result<Texels, String> {
    let b = |format, rgb_only| Texels::Blocks { format, rgb_only };
    let bytes = |bytes, at| Texels::Bytes { bytes, at };
    Ok(match format {
        131 | 132 => b(BlockFormat::Bc1, true),
        133 | 134 => b(BlockFormat::Bc1, false),
        135 | 136 => b(BlockFormat::Bc2, false),
        137 | 138 => b(BlockFormat::Bc3, false),
        139 => b(BlockFormat::Bc4, false),
        141 => b(BlockFormat::Bc5, false),
        145 | 146 => b(BlockFormat::Bc7, false),
        9 => bytes(1, [Some(0), None, None, None]),
        16 => bytes(2, [Some(0), Some(1), None, None]),
        23 | 29 => bytes(3, [Some(0), Some(1), Some(2), None]),
        30 | 36 => bytes(3, [Some(2), Some(1), Some(0), None]),
        37 | 43 => bytes(4, [Some(0), Some(1), Some(2), Some(3)]),
        44 | 50 => bytes(4, [Some(2), Some(1), Some(0), Some(3)]),
        140 | 142 => return Err(format!("VkFormat {format} (signed BC4 / BC5)")),
        143 | 144 => return Err(format!("VkFormat {format} (BC6H: HDR texels; no texture of the game takes them)")),
        147..=184 | 1_000_054_000..=1_000_054_007 | 1_000_066_000..=1_000_066_013 => {
            return Err(format!("VkFormat {format} (a mobile block format: ETC, EAC, ASTC or PVRTC)"));
        }
        _ => return Err(format!("VkFormat {format}")),
    })
}

fn level_bytes(data: &[u8], scheme: Option<u32>, expected: u64, stated: u64) -> Result<Vec<u8>, String> {
    if stated != expected {
        return Err(format!("a level states {stated} bytes, its format and size need {expected}"));
    }
    let mut out = Vec::new();
    let mut read = |r: &mut dyn Read| -> Result<(), String> {
        r.take(expected + 1).read_to_end(&mut out).map_err(|e| format!("decompressing a level: {e}"))?;
        Ok(())
    };
    match scheme {
        None => out.extend_from_slice(data),
        Some(2) => {
            let mut z = ruzstd::decoding::StreamingDecoder::new(data).map_err(|e| format!("Zstandard: {e}"))?;
            read(&mut z)?;
        }
        Some(3) => read(&mut flate2::read::ZlibDecoder::new(data))?,
        Some(1) => return Err("BasisLZ supercompression (Basis Universal needs its C++ transcoder)".into()),
        Some(s) => return Err(format!("supercompression scheme {s}")),
    }
    if out.len() as u64 != expected {
        return Err(format!("a level holds {} bytes, its format and size need {expected}", out.len()));
    }
    Ok(out)
}

fn punches_through(block: &[u8; 8]) -> bool {
    let (c0, c1) = (u16::from_le_bytes([block[0], block[1]]), u16::from_le_bytes([block[2], block[3]]));
    let indices = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
    c0 <= c1 && (0..16).any(|i| indices >> (2 * i) & 3 == 3)
}

const MAGIC: &[u8; 12] = b"\xABKTX 20\xBB\r\n\x1A\n";
const HEADER: usize = 80;

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from(u32_at(b, at)) | u64::from(u32_at(b, at + 4)) << 32
}

fn section(offset: u64, length: u64, len: usize, what: &str) -> Result<std::ops::Range<usize>, String> {
    let end =
        offset.checked_add(length).filter(|&e| e <= len as u64).ok_or_else(|| format!("the {what} past the end"))?;
    Ok(offset as usize..end as usize)
}

pub(super) fn size(bytes: &[u8], file: &str) -> Result<(u32, u32, u32), TextureError> {
    let fail = |m: String| TextureError::new(file, format!("KTX2: {m}"));
    if bytes.get(..12) != Some(MAGIC) {
        return Err(fail("no KTX 20 identifier".into()));
    }
    let header = bytes.get(..HEADER).ok_or_else(|| fail("shorter than its header".into()))?;
    let field = |i: usize| u32_at(header, 12 + 4 * i);
    match field(6) {
        1 | 6 => Ok((field(2), field(3), field(6))),
        n => Err(fail(format!("{n} faces (1 or 6)"))),
    }
}

pub(super) fn decode(
    bytes: &[u8],
    file: &str,
    limits: &Limits,
) -> Result<(Shape, Vec<Level>, Option<BlockChain>), TextureError> {
    let fail = |m: String| TextureError::new(file, format!("KTX2: {m}"));
    if bytes.get(..12) != Some(MAGIC) {
        return Err(fail("no KTX 20 identifier".into()));
    }
    let header = bytes.get(..HEADER).ok_or_else(|| fail("shorter than its header".into()))?;
    let field = |i: usize| u32_at(header, 12 + 4 * i);
    let (format, type_size, width, height, depth) = (field(0), field(1), field(2), field(3), field(4));
    let (layers, face_count, level_count, scheme) = (field(5), field(6), field(7), field(8));
    if format == 0 {
        return Err(fail("VK_FORMAT_UNDEFINED (Basis Universal / UASTC) is not supported".into()));
    }
    let texels = texels(format).map_err(|m| fail(format!("{m} is not supported")))?;
    if type_size != 1 {
        return Err(fail(format!("typeSize {type_size} (1 for 8-bit and block formats)")));
    }
    if height == 0 {
        return Err(fail("a 1-D texture (only planes and cubes)".into()));
    }
    if depth != 0 {
        return Err(fail("a volume texture (only planes and cubes)".into()));
    }
    if layers > 1 {
        return Err(fail(format!("an array of {layers} (one texture or one cube per file)")));
    }
    let cube = match face_count {
        1 => false,
        6 => true,
        n => return Err(fail(format!("{n} faces (1 or 6)"))),
    };
    let faces = if cube { 6 } else { 1 };
    check_size(file, width, height, faces, limits)?;
    if cube && width != height {
        return Err(fail(format!("cube faces of {width}×{height} (faces are square)")));
    }
    let mips = level_count.max(1);
    if mips > chain_len(width, height) {
        return Err(fail(format!("{level_count} mip levels for {width}×{height}")));
    }
    let index_end = HEADER + 24 * mips as usize;
    let index = bytes.get(HEADER..index_end).ok_or_else(|| fail("shorter than its level index".into()))?;
    let len = bytes.len();
    let dfd = section(u64::from(u32_at(header, 48)), u64::from(u32_at(header, 52)), len, "data format descriptor")
        .map_err(fail)?;
    if dfd.start < index_end || dfd.len() < 4 || u32_at(bytes, dfd.start) as usize != dfd.len() {
        return Err(fail("a data format descriptor without its total size".into()));
    }
    section(u64::from(u32_at(header, 56)), u64::from(u32_at(header, 60)), len, "key/value data").map_err(fail)?;
    let sgd = section(u64_at(header, 64), u64_at(header, 72), len, "supercompression global data").map_err(fail)?;
    if !sgd.is_empty() {
        return Err(fail("supercompression global data (BasisLZ only, which is not supported)".into()));
    }
    let scheme = (scheme != 0).then_some(scheme);
    let mut levels = Vec::with_capacity(mips as usize);
    let mut blocks = Vec::new();
    let mut opaque_bc1 = false;
    for (l, entry) in index.as_chunks::<24>().0.iter().enumerate() {
        let (w, h) = ((width >> l).max(1), (height >> l).max(1));
        let range = section(u64_at(entry, 0), u64_at(entry, 8), len, "level data")
            .map_err(|m| fail(format!("level {l}: {m}")))?;
        if range.start < index_end {
            return Err(fail(format!("level {l} overlaps the header")));
        }
        let face_bytes = match texels {
            Texels::Blocks { format, .. } => format.level_bytes(w, h),
            Texels::Bytes { bytes, .. } => u64::from(w) * u64::from(h) * bytes as u64,
        };
        let data = level_bytes(&bytes[range], scheme, face_bytes * u64::from(faces), u64_at(entry, 16))
            .map_err(|m| fail(format!("level {l}: {m}")))?;
        let mut images = Vec::with_capacity(faces as usize);
        for face in data.chunks_exact(face_bytes as usize) {
            let rgba = match texels {
                Texels::Blocks { format, rgb_only } => {
                    let mut rgba = decode_level(format, face, w, h).map_err(|m| fail(format!("level {l}: {m}")))?;
                    if rgb_only {
                        opaque_bc1 |= face.as_chunks::<8>().0.iter().any(punches_through);
                        rgba.as_chunks_mut::<4>().0.iter_mut().for_each(|t| t[3] = 255);
                    }
                    if !cube {
                        blocks.push(BlockLevel { width: w, height: h, data: face.to_vec() });
                    }
                    rgba
                }
                Texels::Bytes { bytes, at } => face
                    .chunks_exact(bytes)
                    .flat_map(|t| {
                        let get = |c: usize, d: u8| at[c].map_or(d, |k| t[k]);
                        [get(0, 0), get(1, 0), get(2, 0), get(3, 255)]
                    })
                    .collect(),
            };
            images.push(rgba);
        }
        levels.push(Level { width: w, height: h, images });
    }
    let chain = match texels {
        Texels::Blocks { format, .. } if !cube && !opaque_bc1 => Some(BlockChain { format, levels: blocks }),
        _ => None,
    };
    Ok((if cube { Shape::Cube } else { Shape::Plane }, levels, chain))
}
