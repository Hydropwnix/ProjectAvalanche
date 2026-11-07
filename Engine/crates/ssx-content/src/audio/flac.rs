#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FlacInfo {
    pub channels: u16,
    pub rate: u32,
    pub frames: u64,
    pub bits: u16,
    pub md5: [u8; 16],
    pub audio_start: usize,
}

const MAX_BLOCKS: usize = 1024;
const STREAMINFO: u8 = 0;
const STREAMINFO_BYTES: usize = 34;

pub(super) fn walk(b: &[u8]) -> Result<FlacInfo, String> {
    let mut at = 4usize;
    let mut info: Option<FlacInfo> = None;
    for k in 0..=MAX_BLOCKS {
        if k == MAX_BLOCKS {
            return Err(format!("more than {MAX_BLOCKS} metadata blocks"));
        }
        if b.len() - at < 4 {
            return Err(format!("a metadata block header cut short at byte {at}"));
        }
        let (last, kind) = (b[at] & 0x80 != 0, b[at] & 0x7F);
        let size = usize::from(b[at + 1]) << 16 | usize::from(b[at + 2]) << 8 | usize::from(b[at + 3]);
        let body = at + 4;
        let end = body
            .checked_add(size)
            .filter(|&e| e <= b.len())
            .ok_or_else(|| format!("metadata block {k} (type {kind}) states {size} bytes, past the file's end"))?;
        if kind == 127 {
            return Err(format!("metadata block {k} has the invalid type 127"));
        }
        if k == 0 {
            if kind != STREAMINFO || size != STREAMINFO_BYTES {
                return Err("the first metadata block is not a 34-byte STREAMINFO".into());
            }
            info = Some(stream_info(&b[body..end])?);
        } else if kind == STREAMINFO {
            return Err(format!("a second STREAMINFO (block {k})"));
        }
        at = end;
        if last {
            break;
        }
    }
    let mut info = info.ok_or("no STREAMINFO")?;
    if at >= b.len() {
        return Err("no audio frames after the metadata".into());
    }
    if b.len() - at < 2 || b[at] != 0xFF || b[at + 1] & 0xFC != 0xF8 {
        return Err(format!("no frame sync code after the metadata (byte {at})"));
    }
    info.audio_start = at;
    Ok(info)
}

fn stream_info(s: &[u8]) -> Result<FlacInfo, String> {
    let min_block = u16::from_be_bytes([s[0], s[1]]);
    let max_block = u16::from_be_bytes([s[2], s[3]]);
    if min_block < 16 || max_block < min_block {
        return Err(format!("block sizes {min_block}…{max_block} (at least 16, the minimum not above the maximum)"));
    }
    let packed = u64::from_be_bytes([s[10], s[11], s[12], s[13], s[14], s[15], s[16], s[17]]);
    let rate = (packed >> 44) as u32;
    let channels = ((packed >> 41) & 0x7) as u16 + 1;
    let bits = ((packed >> 36) & 0x1F) as u16 + 1;
    let frames = packed & 0xF_FFFF_FFFF;
    let mut md5 = [0u8; 16];
    md5.copy_from_slice(&s[18..34]);
    if rate == 0 {
        return Err("a sample rate of 0".into());
    }
    if bits < 4 {
        return Err(format!("{bits} bits per sample (at least 4)"));
    }
    if frames == 0 {
        return Err("the total of samples is not stated (0): the file cannot be verified".into());
    }
    if md5 == [0; 16] {
        return Err("the MD5 of the audio is not stated: the file cannot be verified".into());
    }
    Ok(FlacInfo { channels, rate, frames, bits, md5, audio_start: 0 })
}
