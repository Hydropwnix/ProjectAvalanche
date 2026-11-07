#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct WavInfo {
    pub channels: u16,
    pub rate: u32,
    pub frames: u64,
    pub bits: u16,
    pub float: bool,
}

const MAX_CHUNKS: usize = 1024;
const PCM: u16 = 1;
const FLOAT: u16 = 3;
const EXTENSIBLE: u16 = 0xFFFE;
const GUID_TAIL: [u8; 14] = [0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71];

fn u16le(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

pub(super) fn walk(b: &[u8]) -> Result<WavInfo, String> {
    if b.len() < 12 || &b[8..12] != b"WAVE" {
        return Err("not a RIFF WAVE file".into());
    }
    let stated = u64::from(u32le(b, 4)) + 8;
    if stated != b.len() as u64 {
        return Err(format!("the RIFF header states {stated} bytes, the file has {} (cut short or padded)", b.len()));
    }
    let (mut at, mut chunks) = (12usize, 0usize);
    let mut fmt: Option<(u16, u16, u32, u16, u16)> = None;
    let mut data: Option<u64> = None;
    while at < b.len() {
        chunks += 1;
        if chunks > MAX_CHUNKS {
            return Err(format!("more than {MAX_CHUNKS} chunks"));
        }
        if b.len() - at < 8 {
            return Err(format!("a chunk header cut short at byte {at}"));
        }
        let id = &b[at..at + 4];
        let size = u32le(b, at + 4) as usize;
        let body = at + 8;
        let end = body.checked_add(size).filter(|&e| e <= b.len()).ok_or_else(|| {
            format!(
                "chunk {} at byte {at} states {size} bytes, past the file's end",
                ssx_core::text::quoted(&String::from_utf8_lossy(id))
            )
        })?;
        match id {
            b"fmt " => {
                if fmt.is_some() || data.is_some() {
                    return Err("a second `fmt ` chunk, or one after `data`".into());
                }
                fmt = Some(format_chunk(&b[body..end])?);
            }
            b"data" => {
                if data.is_some() {
                    return Err("a second `data` chunk".into());
                }
                let Some((_, _, _, align, _)) = fmt else { return Err("`data` before `fmt `".into()) };
                if !size.is_multiple_of(usize::from(align)) {
                    return Err(format!("{size} bytes of data are not whole frames of {align} bytes"));
                }
                data = Some((size / usize::from(align)) as u64);
            }
            _ => {}
        }
        at = end + (size & 1);
        if at > b.len() {
            return Err("the last chunk's pad byte is missing".into());
        }
    }
    let (tag, channels, rate, _, bits) = fmt.ok_or("no `fmt ` chunk")?;
    let frames = data.ok_or("no `data` chunk")?;
    Ok(WavInfo { channels, rate, frames, bits, float: tag == FLOAT })
}

fn format_chunk(c: &[u8]) -> Result<(u16, u16, u32, u16, u16), String> {
    if c.len() < 16 {
        return Err(format!("a `fmt ` chunk of {} bytes (at least 16)", c.len()));
    }
    let (mut tag, channels, rate) = (u16le(c, 0), u16le(c, 2), u32le(c, 4));
    let (byte_rate, align, bits) = (u32le(c, 8), u16le(c, 12), u16le(c, 14));
    if tag == EXTENSIBLE {
        if c.len() < 40 || u16le(c, 16) < 22 {
            return Err("an extensible `fmt ` chunk without its 22 extension bytes".into());
        }
        let valid = u16le(c, 18);
        if valid != bits {
            return Err(format!("{valid} valid bits in {bits}-bit containers (taken only when equal)"));
        }
        if c[26..40] != GUID_TAIL {
            return Err("an extensible format whose subformat is not PCM or float".into());
        }
        tag = u16le(c, 24);
    }
    match (tag, bits) {
        (PCM, 8 | 16 | 24 | 32) | (FLOAT, 32) => {}
        (PCM | FLOAT, _) => {
            return Err(format!("{bits}-bit {} (taken: 8, 16, 24, 32-bit PCM, 32-bit float)", kind(tag)));
        }
        _ => return Err(format!("format {tag:#06x} (taken: PCM and 32-bit float)")),
    }
    if channels == 0 {
        return Err("0 channels".into());
    }
    let want_align = u32::from(channels) * u32::from(bits / 8);
    if u32::from(align) != want_align {
        return Err(format!("block align {align}, but {channels} channels of {bits} bits make {want_align}"));
    }
    let want_rate = u64::from(rate) * u64::from(align);
    if u64::from(byte_rate) != want_rate {
        return Err(format!("{byte_rate} bytes per second, but {rate} Hz × {align} bytes make {want_rate}"));
    }
    Ok((tag, channels, rate, align, bits))
}

fn kind(tag: u16) -> &'static str {
    if tag == FLOAT { "float" } else { "PCM" }
}
