use crate::eaac::{Codec, SoundBlocks, SoundHeader};
use crate::ealayer3::Frame;
use crate::mp3_layer3::{BitReader, ChannelState, GRANULE_SAMPLES, Granule, GranuleChannel, decode_granule};
use crate::{Error, Result};

pub const WARM_UP_SAMPLES: usize = 576 + 529;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DecodeStats {
    pub granules: u64,
    pub huffman_exact: u64,
    pub huffman_overrun: u64,
    pub huffman_leftover: u64,
    pub block_types: [u64; 4],
    pub mixed_blocks: u64,
    pub mode_extensions: [u64; 4],
    pub scfsi_used: u64,
    pub ms_block_mismatch: u64,
    pub pcm_blocks: u64,
}

impl DecodeStats {
    pub fn add(&mut self, o: &DecodeStats) {
        self.granules += o.granules;
        self.huffman_exact += o.huffman_exact;
        self.huffman_overrun += o.huffman_overrun;
        self.huffman_leftover += o.huffman_leftover;
        for i in 0..4 {
            self.block_types[i] += o.block_types[i];
            self.mode_extensions[i] += o.mode_extensions[i];
        }
        self.mixed_blocks += o.mixed_blocks;
        self.scfsi_used += o.scfsi_used;
        self.ms_block_mismatch += o.ms_block_mismatch;
        self.pcm_blocks += o.pcm_blocks;
    }
}

pub fn frame_granule(f: &Frame, data: &[u8]) -> Result<(Granule, usize)> {
    if f.mpeg_version != 1 {
        return Err(Error::invalid("EALayer3: MPEG-2/2.5 granules are not used on this disc"));
    }
    let mut r = BitReader::new(data, 8);
    let digest = r.bits(8);
    let mut g = Granule {
        sample_rate_index: ((digest >> 4) & 3) as u8,
        channel_mode: f.channel_mode,
        mode_extension: f.mode_extension,
        channels: f.channels as usize,
        granule: r.bits(1) as u8,
        ..Default::default()
    };
    if g.granule == 1 {
        for s in &mut g.scfsi[..g.channels] {
            *s = r.bits(4) as u8;
        }
    }
    for c in &mut g.ch[..g.channels] {
        *c = GranuleChannel::read(&mut r)?;
    }
    let bits: u32 = g.ch[..g.channels].iter().map(|c| u32::from(c.part2_3_length)).sum();
    if bits != f.main_data_bits {
        return Err(Error::invalid("EALayer3: side info disagrees with the frame parser"));
    }
    Ok((g, r.pos()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PcmLayout {
    Planar,
    Interleaved,
}

pub struct SectionDecoder {
    channels: usize,
    states: Vec<ChannelState>,
    skip: usize,
    pub pcm_layout: PcmLayout,
    pub stats: DecodeStats,
}

impl SectionDecoder {
    pub fn new(channels: u8) -> Result<Self> {
        if !(1..=2).contains(&channels) {
            return Err(Error::invalid(format!("EALayer3: {channels} channels (only 1 and 2 occur on the disc)")));
        }
        Ok(Self {
            channels: channels as usize,
            states: vec![ChannelState::default(); channels as usize],
            skip: WARM_UP_SAMPLES,
            pcm_layout: PcmLayout::Planar,
            stats: DecodeStats::default(),
        })
    }

    pub fn decode_payload(&mut self, payload: &[u8], out: &mut Vec<f32>) -> Result<usize> {
        let (frames, _padding) = crate::ealayer3::frames(payload)?;
        let start = out.len();
        let mut pcm = [[0f32; GRANULE_SAMPLES]; 2];
        let mut pos = 0;
        for f in &frames {
            let data = &payload[pos..pos + f.size];
            pos += f.size;
            if f.channels as usize != self.channels {
                return Err(Error::invalid(format!(
                    "EALayer3: {}-channel frame in a {}-channel sound",
                    f.channels, self.channels
                )));
            }
            let (g, bit) = frame_granule(f, data)?;
            let reports = decode_granule(&mut self.states, &g, data, bit, &mut pcm)?;
            let s = &mut self.stats;
            s.granules += 1;
            for (c, rep) in reports.iter().enumerate().take(self.channels) {
                match rep.end_delta {
                    0 => s.huffman_exact += 1,
                    d if d > 0 => s.huffman_overrun += 1,
                    _ => s.huffman_leftover += 1,
                }
                let gc = &g.ch[c];
                s.block_types[gc.block_type as usize] += 1;
                s.mixed_blocks += u64::from(gc.window_switching && gc.mixed_block);
                s.scfsi_used += u64::from(g.granule == 1 && g.scfsi[c] != 0);
            }
            if g.channel_mode == 1 {
                s.mode_extensions[g.mode_extension as usize] += 1;
                let shape = |c: &GranuleChannel| (c.window_switching, c.block_type, c.mixed_block);
                s.ms_block_mismatch += u64::from(g.mode_extension & 2 != 0 && shape(&g.ch[0]) != shape(&g.ch[1]));
            }
            let drop = self.skip.min(GRANULE_SAMPLES);
            self.skip -= drop;
            if f.flag == 0xEE {
                self.pcm_block(f, data, &mut pcm, drop)?;
            }
            for i in drop..GRANULE_SAMPLES {
                for ch in &pcm[..self.channels] {
                    out.push(ch[i]);
                }
            }
        }
        Ok((out.len() - start) / self.channels)
    }

    fn pcm_block(
        &mut self,
        f: &Frame,
        data: &[u8],
        decoded: &mut [[f32; GRANULE_SAMPLES]; 2],
        drop: usize,
    ) -> Result<()> {
        let (n, ch) = (f.pcm_samples as usize, self.channels);
        let start = usize::try_from(f.pcm_offset)
            .ok()
            .and_then(|offset| drop.checked_add(offset))
            .ok_or_else(|| Error::invalid("EALayer3 PCM destination offset overflow"))?;
        let end = start.checked_add(n).ok_or_else(|| Error::invalid("EALayer3 PCM destination range overflow"))?;
        if end > GRANULE_SAMPLES {
            return Err(Error::invalid(format!(
                "EALayer3: PCM range {start}..{end} outside the {GRANULE_SAMPLES}-sample granule"
            )));
        }
        let pcm = &data[f.size - 2 * n * ch..f.size];
        let sample = |i: usize| f32::from(i16::from_be_bytes([pcm[2 * i], pcm[2 * i + 1]])) / 32768.0;
        for i in 0..n {
            for (c, channel) in decoded[..ch].iter_mut().enumerate() {
                channel[start + i] = sample(match self.pcm_layout {
                    PcmLayout::Planar => c * n + i,
                    PcmLayout::Interleaved => i * ch + c,
                });
            }
        }
        self.stats.pcm_blocks += 1;
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct DecodedSound {
    pub channels: u8,
    pub sample_rate: u32,
    pub samples: Vec<f32>,
    pub stats: DecodeStats,
}

pub fn decode_sound(h: &SoundHeader, blocks: &SoundBlocks) -> Result<DecodedSound> {
    decode_sound_with(h, blocks, PcmLayout::Planar)
}

pub fn decode_sound_with(h: &SoundHeader, blocks: &SoundBlocks, layout: PcmLayout) -> Result<DecodedSound> {
    if h.codec != Codec::EaLayer3V1 {
        return Err(Error::invalid(format!("decode EALayer3: codec {:?}", h.codec)));
    }
    let ch = h.channels as usize;
    let mut samples = Vec::with_capacity(h.num_samples as usize * ch);
    let mut stats = DecodeStats::default();
    let mut tmp = Vec::new();
    for section in &blocks.sections {
        let mut dec = SectionDecoder::new(h.channels)?;
        dec.pcm_layout = layout;
        for (i, b) in section.iter().enumerate() {
            tmp.clear();
            let n = dec.decode_payload(b.payload, &mut tmp)?;
            let want = b.samples as usize;
            if n < want || (n > want && i + 1 != section.len()) {
                return Err(Error::invalid(format!("EALayer3 block {i}: decodes to {n} samples, header says {want}")));
            }
            samples.extend_from_slice(&tmp[..want * ch]);
        }
        stats.add(&dec.stats);
    }
    if samples.len() != h.num_samples as usize * ch {
        return Err(Error::invalid(format!(
            "EALayer3: {} samples decoded, header says {}",
            samples.len() / ch,
            h.num_samples
        )));
    }
    Ok(DecodedSound { channels: h.channels, sample_rate: h.sample_rate, samples, stats })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn silent_frame(digest: u8, granule: u8, channels: usize, pcm: &[i16]) -> Vec<u8> {
        let bits = 8 + 1 + if granule == 1 { 4 * channels } else { 0 } + 59 * channels;
        let mut f = vec![if pcm.is_empty() { 0x00 } else { 0xEE }, digest];
        f.resize(1 + bits.div_ceil(8), 0);
        f[2] = granule << 7;
        if !pcm.is_empty() {
            f.extend(((pcm.len() / channels) as u32).to_be_bytes());
            f.extend(0u32.to_be_bytes());
            f.extend(pcm.iter().flat_map(|s| s.to_be_bytes()));
        }
        f
    }

    #[test]
    fn pcm_block_and_warm_up() {
        let mut p = silent_frame(0xCC, 0, 1, &[]);
        p.extend(silent_frame(0xCC, 1, 1, &[0x4000, -0x4000, 0x0100]));
        p.extend(silent_frame(0xCC, 0, 1, &[]));
        p.extend(silent_frame(0xCC, 1, 1, &[]));
        let mut dec = SectionDecoder::new(1).unwrap();
        let mut out = Vec::new();
        assert_eq!(dec.decode_payload(&p, &mut out).unwrap(), 4 * 576 - WARM_UP_SAMPLES);
        assert_eq!(&out[..3], &[0.5, -0.5, 1.0 / 128.0]);
        assert!(out[3..].iter().all(|&v| v == 0.0));
        assert_eq!((dec.stats.granules, dec.stats.pcm_blocks, dec.stats.huffman_exact), (4, 1, 4));
        out.clear();
        assert_eq!(dec.decode_payload(&silent_frame(0xCC, 0, 1, &[]), &mut out).unwrap(), 576);
    }

    #[test]
    fn stereo_pcm_block_is_planar() {
        let mut p = silent_frame(0xC4, 0, 2, &[]);
        p.extend(silent_frame(0xC4, 1, 2, &[1, 2, 3, 4]));
        let mut dec = SectionDecoder::new(2).unwrap();
        let mut out = Vec::new();
        assert_eq!(dec.decode_payload(&p, &mut out).unwrap(), 2 * 576 - WARM_UP_SAMPLES);
        let s = |v: i16| f32::from(v) / 32768.0;
        assert_eq!(&out[..4], &[s(1), s(3), s(2), s(4)]);
        assert!(out[4..].iter().all(|&v| v == 0.0));
        assert!(SectionDecoder::new(4).is_err());
    }

    #[test]
    fn pcm_extrema_use_el31_scale_without_clamping() {
        let values = [i16::MIN, i16::MAX, -1, 0, 1];
        let mut payload = silent_frame(0xCC, 0, 1, &[]);
        payload.extend(silent_frame(0xCC, 1, 1, &values));
        let mut dec = SectionDecoder::new(1).unwrap();
        let mut out = Vec::new();
        dec.decode_payload(&payload, &mut out).unwrap();
        assert_eq!(&out[..5], &[-1.0, 32767.0 / 32768.0, -1.0 / 32768.0, 0.0, 1.0 / 32768.0]);
        assert!(out[5..].iter().all(|&v| v == 0.0));
    }

    #[test]
    fn each_section_has_its_own_warm_up_and_pcm() {
        use crate::eaac::{Block, Storage};

        let section = |value: i16| {
            let mut payload = silent_frame(0xCC, 0, 1, &[]);
            payload.extend(silent_frame(0xCC, 1, 1, &[value]));
            payload
        };
        let intro = section(i16::MIN);
        let loop_payload = section(i16::MAX);
        let samples_per_section = (2 * GRANULE_SAMPLES - WARM_UP_SAMPLES) as u32;
        let header = SoundHeader {
            version: 0,
            codec: Codec::EaLayer3V1,
            channels: 1,
            sample_rate: 44100,
            storage: Storage::Stream,
            looped: true,
            num_samples: 2 * samples_per_section,
            loop_start: Some(samples_per_section),
            loop_offset: Some((intro.len() + 8) as u32),
            prefetch_samples: None,
            size: 16,
        };
        let block = |offset, payload| Block { offset, flag: 0x80, samples: samples_per_section, payload };
        let blocks = SoundBlocks {
            sections: vec![vec![block(0, intro.as_slice())], vec![block(intro.len() + 8, loop_payload.as_slice())]],
        };
        let decoded = decode_sound(&header, &blocks).unwrap();
        let n = samples_per_section as usize;
        assert_eq!(decoded.samples.len(), 2 * n);
        assert_eq!(decoded.samples[0], -1.0);
        assert_eq!(decoded.samples[n], 32767.0 / 32768.0);
        assert!(decoded.samples[1..n].iter().chain(&decoded.samples[n + 1..]).all(|&v| v == 0.0));
        assert_eq!((decoded.stats.granules, decoded.stats.pcm_blocks), (4, 2));
    }

    fn pcm_at_offset(digest: u8, channels: usize, values: &[i16], offset: u32) -> Vec<u8> {
        let mut frame = silent_frame(digest, 1, channels, values);
        let pos = frame.len() - values.len() * 2 - 4;
        frame[pos..pos + 4].copy_from_slice(&offset.to_be_bytes());
        let mut payload = silent_frame(digest, 0, channels, &[]);
        payload.extend(frame);
        payload
    }

    #[test]
    fn pcm_offset_overwrites_after_the_skipped_prefix() {
        let payload = pcm_at_offset(0xC4, 2, &[i16::MIN, i16::MAX, 16384, -16384], 3);
        let mut dec = SectionDecoder::new(2).unwrap();
        let mut out = Vec::new();
        assert_eq!(dec.decode_payload(&payload, &mut out).unwrap(), 47);
        assert_eq!(&out[..6], &[0.0; 6]);
        assert_eq!(&out[6..10], &[-1.0, 0.5, 32767.0 / 32768.0, -0.5]);
        assert!(out[10..].iter().all(|&v| v == 0.0));
        assert_eq!(dec.stats.pcm_blocks, 1);
        out.clear();
        assert_eq!(dec.decode_payload(&silent_frame(0xC4, 0, 2, &[]), &mut out).unwrap(), 576);
    }

    #[test]
    fn pcm_range_must_fit_the_current_granule() {
        let mut dec = SectionDecoder::new(1).unwrap();
        let mut out = Vec::new();
        assert_eq!(dec.decode_payload(&pcm_at_offset(0xCC, 1, &[1, 2], 45), &mut out).unwrap(), 47);
        assert_eq!(&out[45..], &[1.0 / 32768.0, 2.0 / 32768.0]);
        for offset in [46, 576, u32::MAX] {
            let mut dec = SectionDecoder::new(1).unwrap();
            assert!(dec.decode_payload(&pcm_at_offset(0xCC, 1, &[1, 2], offset), &mut Vec::new()).is_err());
        }
    }
}
