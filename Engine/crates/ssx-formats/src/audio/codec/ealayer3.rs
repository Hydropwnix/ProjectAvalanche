use crate::{Error, Result};

pub const SAMPLES_PER_GRANULE: u32 = 576;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub flag: u8,
    pub mpeg_version: u8,
    pub sample_rate: u32,
    pub channel_mode: u8,
    pub mode_extension: u8,
    pub channels: u8,
    pub granule: u8,
    pub main_data_bits: u32,
    pub pcm_samples: u32,
    pub pcm_offset: u32,
    pub size: usize,
}

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Bits<'_> {
    fn get(&mut self, n: usize) -> Result<u32> {
        let mut v = 0u32;
        for _ in 0..n {
            let byte = *self.data.get(self.pos / 8).ok_or_else(|| Error::invalid("EALayer3 frame past end"))?;
            v = (v << 1) | ((byte >> (7 - self.pos % 8)) & 1) as u32;
            self.pos += 1;
        }
        Ok(v)
    }
}

impl Frame {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut b = Bits { data, pos: 0 };
        let flag = b.get(8)? as u8;
        if flag != 0x00 && flag != 0xEE {
            return Err(Error::invalid(format!("EALayer3 frame flag {flag:#04x}")));
        }
        let version_index = b.get(2)?;
        let rate_index = b.get(2)?;
        let channel_mode = b.get(2)? as u8;
        let mode_extension = b.get(2)? as u8;
        let mpeg_version: u8 = match version_index {
            3 => 1,
            2 => 2,
            0 => 3,
            _ => return Err(Error::invalid("EALayer3 reserved MPEG version")),
        };
        let base = [44100, 48000, 32000];
        if rate_index == 3 {
            return Err(Error::invalid("EALayer3 reserved sample-rate index"));
        }
        let sample_rate = base[rate_index as usize] >> (mpeg_version - 1);
        let channels = if channel_mode == 3 { 1 } else { 2 };
        let granule = b.get(1)? as u8;
        if mpeg_version == 1 && granule == 1 {
            b.get(4 * channels as usize)?;
        }
        let rest_bits = if mpeg_version == 1 { 47 } else { 51 };
        let mut main_data_bits = 0;
        for _ in 0..channels {
            main_data_bits += b.get(12)?;
            b.get(32)?;
            b.get(rest_bits - 32)?;
        }
        let common_bits = b.pos - 8 + main_data_bits as usize;
        let mut size = 1 + common_bits.div_ceil(8);
        let (mut pcm_samples, mut pcm_offset) = (0, 0);
        if flag == 0xEE {
            let p = data.get(size..size + 8).ok_or_else(|| Error::invalid("EALayer3 PCM header past end"))?;
            pcm_samples = u32::from_be_bytes([p[0], p[1], p[2], p[3]]);
            pcm_offset = u32::from_be_bytes([p[4], p[5], p[6], p[7]]);
            let bytes = usize::try_from(pcm_samples)
                .ok()
                .and_then(|n| n.checked_mul(2 * channels as usize))
                .and_then(|n| n.checked_add(8))
                .ok_or_else(|| Error::invalid("EALayer3 PCM byte count overflow"))?;
            size = size.checked_add(bytes).ok_or_else(|| Error::invalid("EALayer3 frame byte count overflow"))?;
        }
        if size > data.len() {
            return Err(Error::invalid(format!("EALayer3 frame of {size} bytes, {} left", data.len())));
        }
        Ok(Self {
            flag,
            mpeg_version,
            sample_rate,
            channel_mode,
            mode_extension,
            channels,
            granule,
            main_data_bits,
            pcm_offset,
            pcm_samples,
            size,
        })
    }
}

pub fn frames(payload: &[u8]) -> Result<(Vec<Frame>, usize)> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < payload.len() {
        if payload[pos..].iter().all(|&b| b == 0) {
            return Ok((out, payload.len() - pos));
        }
        let f = Frame::parse(&payload[pos..]).map_err(|e| Error::invalid(format!("at payload byte {pos}: {e}")))?;
        pos += f.size;
        out.push(f);
    }
    Ok((out, 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_granule_and_padding() {
        let mut p = vec![0x00, 0xCC, 0, 0, 0, 0, 0, 0, 0, 0];
        let f = Frame::parse(&p).unwrap();
        assert_eq!((f.mpeg_version, f.sample_rate, f.channels, f.size), (1, 44100, 1, 10));
        p.extend([0, 0, 0]);
        let (frames, pad) = frames(&p).unwrap();
        assert_eq!((frames.len(), pad), (1, 3));
        assert!(Frame::parse(&[0x12, 0xCC]).is_err());
    }

    fn pcm_frame(count: u32, offset: u32, samples: &[i16]) -> Vec<u8> {
        let mut data = vec![0xEE, 0xCC, 0, 0, 0, 0, 0, 0, 0, 0];
        data.extend(count.to_be_bytes());
        data.extend(offset.to_be_bytes());
        data.extend(samples.iter().flat_map(|s| s.to_be_bytes()));
        data
    }

    #[test]
    fn pcm_header_is_two_big_endian_words() {
        let data = pcm_frame(2, 0x0001_0203, &[i16::MIN, i16::MAX]);
        let f = Frame::parse(&data).unwrap();
        assert_eq!((f.pcm_samples, f.pcm_offset, f.size), (2, 0x0001_0203, 22));
        assert!(Frame::parse(&data[..data.len() - 1]).is_err());
        let zero = Frame::parse(&pcm_frame(0, 0x1234_5678, &[])).unwrap();
        assert_eq!((zero.pcm_samples, zero.pcm_offset, zero.size), (0, 0x1234_5678, 18));
    }

    #[test]
    fn high_count_bits_are_not_a_fictitious_offset() {
        assert!(Frame::parse(&pcm_frame(0x0001_0000, 0, &[])).is_err());
        let large = pcm_frame(65_536, 7, &vec![0; 65_536]);
        let f = Frame::parse(&large).unwrap();
        assert_eq!((f.pcm_samples, f.pcm_offset, f.size), (65_536, 7, 18 + 2 * 65_536));
        assert!(Frame::parse(&pcm_frame(u32::MAX, 0, &[])).is_err());
        for len in 10..18 {
            assert!(Frame::parse(&pcm_frame(2, 0, &[1, 2])[..len]).is_err());
        }
    }
}
