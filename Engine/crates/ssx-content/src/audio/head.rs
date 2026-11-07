use super::{AudioError, AudioInfo, Format, Limits, ogg};

pub type ReadRange<'a> = dyn FnMut(u64, u64) -> Result<Vec<u8>, String> + 'a;

const MAX_PAGE: u64 = 27 + 255 + 255 * 255;

pub fn probe_head(name: &str, size: u64, read: &mut ReadRange<'_>, limits: &Limits) -> Result<AudioInfo, AudioError> {
    let format = Format::from_name(name)?;
    let fail = |m: String| AudioError::new(name, m);
    if size > limits.max_file_bytes {
        return Err(fail(format!("{size} bytes (at most {})", limits.max_file_bytes)));
    }
    if size < 12 {
        return Err(fail(format!("{size} bytes: no {} header", format.name())));
    }
    let head = read(0, size.min(4096)).map_err(|e| fail(e.clone()))?;
    if !head.starts_with(format.magic()) {
        return Err(fail(format!("named {}, but its content is {}", format.name(), super::sniff(&head))));
    }
    let (channels, rate, frames) = match format {
        Format::Wav => wav_head(size, &head, read),
        Format::Flac => flac_head(size, read),
        Format::OggVorbis => ogg_head(size, read),
    }
    .map_err(fail)?;
    let info = AudioInfo { format, channels, rate, frames, file_bytes: size };
    info.check(name, limits)?;
    Ok(info)
}

fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn wav_head(size: u64, head: &[u8], read: &mut ReadRange<'_>) -> Result<(u16, u32, u64), String> {
    if &head[8..12] != b"WAVE" || u64::from(u32le(head, 4)) + 8 != size {
        return Err("not a RIFF WAVE file of its RIFF size".into());
    }
    let (mut at, mut fmt, mut chunks) = (12u64, None::<(u16, u32, u16)>, 0);
    while at < size {
        chunks += 1;
        if chunks > 1024 || size - at < 8 {
            return Err(format!("a chunk header cut short at byte {at}, or more than 1024 chunks"));
        }
        let h = read(at, 8)?;
        let len = u64::from(u32le(&h, 4));
        let body = at + 8;
        if body + len > size {
            return Err(format!("a chunk at byte {at} states {len} bytes, past the file's end"));
        }
        match &h[..4] {
            b"fmt " if len >= 16 => {
                let f = read(body, 16)?;
                let (channels, rate, align) =
                    (u16::from_le_bytes([f[2], f[3]]), u32le(&f, 4), u16::from_le_bytes([f[12], f[13]]));
                if align == 0 {
                    return Err("block align 0".into());
                }
                fmt = Some((channels, rate, align));
            }
            b"fmt " => return Err("a `fmt ` chunk under 16 bytes".into()),
            b"data" => {
                let (channels, rate, align) = fmt.ok_or("`data` before `fmt `")?;
                return Ok((channels, rate, len / u64::from(align)));
            }
            _ => {}
        }
        at = body + len + (len & 1);
    }
    Err("no `data` chunk".into())
}

fn flac_head(size: u64, read: &mut ReadRange<'_>) -> Result<(u16, u32, u64), String> {
    if size < 42 {
        return Err("no STREAMINFO".into());
    }
    let b = read(0, 42)?;
    if b[4] & 0x7F != 0 || b[5..8] != [0, 0, 34] {
        return Err("the first metadata block is not a 34-byte STREAMINFO".into());
    }
    let s = &b[8..42];
    let packed = u64::from_be_bytes([s[10], s[11], s[12], s[13], s[14], s[15], s[16], s[17]]);
    let (rate, channels) = ((packed >> 44) as u32, ((packed >> 41) & 0x7) as u16 + 1);
    let frames = packed & 0xF_FFFF_FFFF;
    if frames == 0 || s[18..34] == [0; 16] {
        return Err("the total of samples or the MD5 of the audio is not stated: the file cannot be verified".into());
    }
    Ok((channels, rate, frames))
}

fn ogg_head(size: u64, read: &mut ReadRange<'_>) -> Result<(u16, u32, u64), String> {
    let first = read(0, size.min(MAX_PAGE))?;
    let n = usize::from(*first.get(26).ok_or("the first page is cut short")?);
    let id_len = usize::from(*first.get(27).ok_or("the first page is cut short")?);
    if n != 1 || id_len != 30 || first.len() < 28 + 30 || first[5] & 0x02 == 0 {
        return Err("the first page does not hold the Vorbis identification header alone".into());
    }
    let p = &first[28..58];
    if p[0] != 1 || &p[1..7] != b"vorbis" || u32le(p, 7) != 0 {
        return Err("no Vorbis identification header".into());
    }
    let (channels, rate) = (u16::from(p[11]), u32le(p, 12));
    let tail_from = size.saturating_sub(MAX_PAGE);
    let tail = read(tail_from, size - tail_from)?;
    if tail.len() < 27 {
        return Err("no whole last page at the file's end (cut short)".into());
    }
    let mut k = tail.len() - 27;
    loop {
        if &tail[k..k + 4] == b"OggS" {
            let segs = usize::from(tail[k + 26]);
            if k + 27 + segs <= tail.len() {
                let body: usize = tail[k + 27..k + 27 + segs].iter().map(|&l| usize::from(l)).sum();
                let end = k + 27 + segs + body;
                if end == tail.len() && ogg::page_crc(&tail[k..end]) == u32le(&tail, k + 22) {
                    if tail[k + 5] & 0x04 == 0 {
                        return Err("the last page is not the end of the stream (EOS)".into());
                    }
                    let granule = u64::from_le_bytes(tail[k + 6..k + 14].try_into().unwrap_or([0; 8]));
                    if granule == u64::MAX {
                        return Err("the last page states no granule position".into());
                    }
                    return Ok((channels, rate, granule));
                }
            }
        }
        if k == 0 {
            return Err("no whole last page at the file's end (cut short)".into());
        }
        k -= 1;
    }
}
