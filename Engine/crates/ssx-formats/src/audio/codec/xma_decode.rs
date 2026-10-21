use crate::eaac::{Codec, SoundBlocks, SoundHeader};
use crate::wmapro::{FRAME_SAMPLES, FrameInfo, FrameStats, StreamDecoder, rate_of_index};
use crate::xma::rate_index_for;
use crate::xma::{FRAME_LENGTH_BITS, NO_MORE_FRAMES, PACKET_SIZE, PacketHeader, split_block};
use crate::{Error, Result};

const PAYLOAD_BYTES: usize = PACKET_SIZE - 4;
const PAYLOAD_BITS: usize = PAYLOAD_BYTES * 8;

#[derive(Clone, Debug, Default)]
pub struct SubstreamFrames {
    pub bits: Vec<u8>,
    pub frames: Vec<usize>,
}

impl SubstreamFrames {
    pub fn parse(packets: &[u8]) -> Result<Self> {
        let count = packets.len().div_ceil(PACKET_SIZE);
        let mut bits = Vec::with_capacity(count * PAYLOAD_BYTES + 8);
        let mut first = Vec::with_capacity(count);
        for p in packets.chunks(PACKET_SIZE) {
            if p.len() < 4 {
                return Err(Error::invalid("XMA packet shorter than its header"));
            }
            first.push(usize::from(PacketHeader::parse(p).first_frame_bits));
            bits.extend_from_slice(&p[4..]);
        }
        let total = bits.len() * 8;
        bits.extend_from_slice(&[0; 8]);
        let read15 = |pos: usize| {
            let b = pos / 8;
            let w = u32::from_be_bytes([bits[b], bits[b + 1], bits[b + 2], 0]);
            (w << (pos % 8)) >> (32 - FRAME_LENGTH_BITS)
        };
        let packet_start = |mut k: usize| {
            while k < count {
                if first[k] < PAYLOAD_BITS {
                    return Some(k * PAYLOAD_BITS + first[k]);
                }
                k += 1;
            }
            None
        };
        let mut frames = Vec::new();
        let mut pos = packet_start(0);
        while let Some(p) = pos {
            if p + FRAME_LENGTH_BITS > total {
                break;
            }
            let len = read15(p) as usize;
            if len == NO_MORE_FRAMES as usize || len == 0 {
                pos = packet_start(p / PAYLOAD_BITS + 1);
                continue;
            }
            if p + len > total {
                return Err(Error::invalid(format!("XMA frame at bit {p} (length {len}) runs past the packets")));
            }
            frames.push(p);
            pos = Some(p + len);
        }
        Ok(Self { bits, frames })
    }
}

#[derive(Clone, Debug, Default)]
pub struct RawBlock {
    pub frames: usize,
    pub infos: Vec<Vec<FrameInfo>>,
}

#[derive(Clone)]
pub struct FrameDecoder {
    channels: usize,
    streams: Vec<StreamDecoder>,
    rate_index: u8,
    pub stats: FrameStats,
}

impl FrameDecoder {
    pub fn new(channels: u8, rate_index: u8) -> Result<Self> {
        Self::with_band_rate(channels, rate_index, rate_of_index(rate_index))
    }

    pub fn with_band_rate(channels: u8, rate_index: u8, band_rate: u32) -> Result<Self> {
        let n = usize::from(channels);
        if n == 0 || n > 8 {
            return Err(Error::invalid(format!("EA-XMA: {n} channels")));
        }
        let streams = (0..n.div_ceil(2))
            .map(|s| StreamDecoder::with_band_rate(if 2 * s + 1 == n { 1 } else { 2 }, band_rate))
            .collect::<Result<_>>()?;
        Ok(Self { channels: n, streams, rate_index, stats: FrameStats::default() })
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn decode_block_raw(&mut self, payload: &[u8], out: &mut Vec<f32>) -> Result<RawBlock> {
        let (subs, _extra) = split_block(payload, self.streams.len())?;
        let mut raw = RawBlock::default();
        let mut pcm: Vec<Vec<[[f32; FRAME_SAMPLES]; 2]>> = Vec::with_capacity(subs.len());
        for (s, (sub, dec)) in subs.iter().zip(&mut self.streams).enumerate() {
            if sub.rate_index != self.rate_index {
                return Err(Error::invalid(format!("EA-XMA substream {s}: rate index {}", sub.rate_index)));
            }
            let f = SubstreamFrames::parse(sub.packets)?;
            if s > 0 && f.frames.len() != raw.frames {
                return Err(Error::invalid(format!(
                    "EA-XMA substream {s}: {} frames, not {}",
                    f.frames.len(),
                    raw.frames
                )));
            }
            raw.frames = f.frames.len();
            let mut frames = vec![[[0f32; FRAME_SAMPLES]; 2]; f.frames.len()];
            let mut infos = Vec::with_capacity(f.frames.len());
            for (&start, o) in f.frames.iter().zip(&mut frames) {
                infos.push(dec.decode_frame(&f.bits, start, o, &mut self.stats)?);
            }
            raw.infos.push(infos);
            pcm.push(frames);
        }
        out.reserve(raw.frames * FRAME_SAMPLES * self.channels);
        interleave(
            &self.streams,
            raw.frames * FRAME_SAMPLES,
            |s, c, i| pcm[s][i / FRAME_SAMPLES][c][i % FRAME_SAMPLES],
            out,
        );
        Ok(raw)
    }

    pub fn tail(&self, out: &mut Vec<f32>) {
        let t: Vec<_> = self
            .streams
            .iter()
            .map(|dec| {
                let mut b = [[0f32; FRAME_SAMPLES / 2]; 2];
                dec.tail(&mut b);
                b
            })
            .collect();
        interleave(&self.streams, FRAME_SAMPLES / 2, |s, c, i| t[s][c][i], out);
    }
}

fn interleave(streams: &[StreamDecoder], n: usize, at: impl Fn(usize, usize, usize) -> f32, out: &mut Vec<f32>) {
    for i in 0..n {
        for (s, dec) in streams.iter().enumerate() {
            for c in 0..dec.channels() {
                out.push(at(s, c, i));
            }
        }
    }
}

pub const OUTPUT_OFFSET: usize = 192;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BlockReport {
    pub frames: usize,
    pub leftover: usize,
    pub trim_start: Option<u16>,
    pub trim_end: Option<u16>,
}

#[derive(Clone)]
pub struct SectionDecoder {
    dec: FrameDecoder,
    raw: Vec<f32>,
    tail: Vec<f32>,
    frames: usize,
    pos: usize,
    trim_start: Option<u16>,
}

impl SectionDecoder {
    pub fn new(channels: u8, rate_index: u8) -> Result<Self> {
        Ok(Self {
            dec: FrameDecoder::new(channels, rate_index)?,
            raw: Vec::new(),
            tail: Vec::new(),
            frames: 0,
            pos: 0,
            trim_start: None,
        })
    }

    pub fn stats(&self) -> &FrameStats {
        &self.dec.stats
    }

    pub fn decode_block(&mut self, payload: &[u8], samples: u32, out: &mut Vec<f32>) -> Result<BlockReport> {
        let ch = self.dec.channels();
        self.raw.clear();
        let raw = self.dec.decode_block_raw(payload, &mut self.raw)?;
        let first = raw.infos.first().and_then(|f| f.first()).copied().unwrap_or_default();
        let last = raw.infos.first().and_then(|f| f.last()).copied().unwrap_or_default();
        if self.frames == 0 {
            let skip =
                first.trim_start.ok_or_else(|| Error::invalid("EA-XMA section: first frame has no start trim"))?;
            if skip != 512 {
                return Err(Error::invalid(format!("EA-XMA section: start trim {skip} (only 512 on the disc)")));
            }
            self.trim_start = Some(skip);
            self.pos = OUTPUT_OFFSET + 384;
        }
        let base = self.frames * FRAME_SAMPLES;
        self.frames += raw.frames;
        let avail = self.frames * FRAME_SAMPLES + OUTPUT_OFFSET;
        let want = samples as usize;
        if self.pos + want > avail {
            return Err(Error::invalid(format!(
                "EA-XMA block: {} samples decodable, the block word says {want}",
                avail.saturating_sub(self.pos)
            )));
        }
        self.tail.clear();
        self.dec.tail(&mut self.tail);
        let frames_end = self.frames * FRAME_SAMPLES;
        let (from, to) = (self.pos, self.pos + want);
        if from < base {
            return Err(Error::invalid("EA-XMA block: output position before the block's frames"));
        }
        let r_end = to.min(frames_end);
        if from < r_end {
            out.extend_from_slice(&self.raw[(from - base) * ch..(r_end - base) * ch]);
        }
        let t_from = from.max(frames_end);
        if t_from < to {
            out.extend_from_slice(&self.tail[(t_from - frames_end) * ch..(to - frames_end) * ch]);
        }
        self.pos = to;
        Ok(BlockReport {
            frames: raw.frames,
            leftover: avail - to,
            trim_start: self.trim_start,
            trim_end: last.trim_end,
        })
    }
}

#[derive(Clone, Debug)]
pub struct DecodedSound {
    pub channels: u8,
    pub sample_rate: u32,
    pub samples: Vec<f32>,
    pub stats: FrameStats,
    pub sections: Vec<SectionInfo>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SectionInfo {
    pub frames: usize,
    pub trim_start: Option<u16>,
    pub trim_end: Option<u16>,
}

pub fn decode_sound(h: &SoundHeader, blocks: &SoundBlocks) -> Result<DecodedSound> {
    if h.codec != Codec::EaXma {
        return Err(Error::invalid(format!("decode EA-XMA: codec {:?}", h.codec)));
    }
    let rate_index = rate_index_for(h.sample_rate);
    let mut samples = Vec::with_capacity(h.num_samples as usize * usize::from(h.channels));
    let mut stats = FrameStats::default();
    let mut sections = Vec::with_capacity(blocks.sections.len());
    for section in &blocks.sections {
        let mut dec = SectionDecoder::new(h.channels, rate_index)?;
        let mut info = SectionInfo::default();
        for (i, b) in section.iter().enumerate() {
            let rep = dec.decode_block(b.payload, b.samples, &mut samples)?;
            info = SectionInfo { frames: info.frames + rep.frames, trim_start: rep.trim_start, trim_end: rep.trim_end };
            let want = if i + 1 == section.len() { rep.trim_end.map_or(usize::MAX, usize::from) } else { 0 };
            if rep.leftover != want {
                return Err(Error::invalid(format!(
                    "EA-XMA block {i}: {} samples left over (end trim {:?})",
                    rep.leftover, rep.trim_end
                )));
            }
        }
        stats.add(dec.stats());
        sections.push(info);
    }
    if samples.len() != h.num_samples as usize * usize::from(h.channels) {
        return Err(Error::invalid(format!(
            "EA-XMA: {} samples decoded, header says {}",
            samples.len() / usize::from(h.channels),
            h.num_samples
        )));
    }
    Ok(DecodedSound { channels: h.channels, sample_rate: h.sample_rate, samples, stats, sections })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eaac::Block;
    use crate::wmapro::test_util::{BitWriter, silent_frame};

    fn packets(frames: &[Option<BitWriter>]) -> Vec<u8> {
        let mut payload = BitWriter::default();
        let mut starts = Vec::new();
        for f in frames {
            match f {
                Some(f) => {
                    starts.push(payload.len);
                    for i in 0..f.len {
                        payload.put(u32::from(f.bytes[i / 8] >> (7 - i % 8) & 1), 1);
                    }
                }
                None => {
                    let rest = PAYLOAD_BITS - payload.len % PAYLOAD_BITS;
                    for _ in 0..rest {
                        payload.put(1, 1);
                    }
                }
            }
        }
        let count = payload.bytes.len().div_ceil(PAYLOAD_BYTES);
        let mut out = Vec::new();
        for k in 0..count {
            let inside: Vec<usize> =
                starts.iter().filter(|&&s| s / PAYLOAD_BITS == k).map(|&s| s % PAYLOAD_BITS).collect();
            let first = inside.first().copied().unwrap_or(0x7FFF) as u32;
            let word = (inside.len() as u32) << 26 | first << 11 | 1 << 8;
            out.extend(word.to_be_bytes());
            out.extend(&payload.bytes[k * PAYLOAD_BYTES..((k + 1) * PAYLOAD_BYTES).min(payload.bytes.len())]);
        }
        out
    }

    fn block_payload(frames: &[Option<BitWriter>], rate_index: u8) -> Vec<u8> {
        let p = packets(frames);
        let mut b = ((((4 + p.len()) as u32) << 2) | u32::from(rate_index)).to_be_bytes().to_vec();
        b.extend(p);
        b
    }

    #[test]
    fn frames_across_packets_and_filler() {
        let mut frames: Vec<_> = (0..700).map(|_| Some(silent_frame(1, None, None, true))).collect();
        frames.push(None);
        frames.extend((0..3).map(|_| Some(silent_frame(1, None, None, true))));
        let p = packets(&frames);
        let f = SubstreamFrames::parse(&p).unwrap();
        assert_eq!(f.frames.len(), 703);
        assert_eq!(f.frames[699], 699 * 45);
        assert_eq!(f.frames[700], 2 * PAYLOAD_BITS);
        assert_eq!(crate::xma::walk_frames(&p).unwrap().frames, 703);
    }

    #[test]
    fn section_trimming() {
        let fr = |s, e| Some(silent_frame(2, s, e, true));
        let b1 = block_payload(&[fr(Some(512), None), fr(None, None), fr(None, None)], 1);
        let b2 = block_payload(&[fr(None, None), fr(None, Some(100))], 1);
        let mut dec = SectionDecoder::new(2, 1).unwrap();
        let mut out = Vec::new();
        let r1 = dec.decode_block(&b1, 3 * 512 - 384, &mut out).unwrap();
        assert_eq!((r1.frames, r1.leftover, r1.trim_start, r1.trim_end), (3, 0, Some(512), None));
        let r2 = dec.decode_block(&b2, 2 * 512 - 100, &mut out).unwrap();
        assert_eq!((r2.frames, r2.leftover, r2.trim_end), (2, 100, Some(100)));
        assert_eq!(out.len(), 2 * (5 * 512 + 128 - 512 - 100));
        assert!(out.iter().all(|&v| v == 0.0));
        let mut dec = SectionDecoder::new(2, 1).unwrap();
        assert!(dec.decode_block(&b1, 3 * 512 - 383, &mut Vec::new()).is_err());
        assert!(SectionDecoder::new(2, 2).unwrap().decode_block(&b1, 1, &mut Vec::new()).is_err());
    }

    #[test]
    fn whole_sound() {
        let words = [0x0300_7D00u32, 512 + 128 - 512 - 20];
        let h = SoundHeader::parse(&words.iter().flat_map(|w| w.to_be_bytes()).collect::<Vec<_>>()).unwrap();
        let payload = block_payload(&[Some(silent_frame(1, Some(512), Some(20), false))], 1);
        let block = Block { offset: 0, flag: 0, samples: words[1], payload: &payload };
        let d = decode_sound(&h, &SoundBlocks { sections: vec![vec![block]] }).unwrap();
        assert_eq!((d.samples.len(), d.sections[0].frames, d.sections[0].trim_end), (108, 1, Some(20)));
        let short = Block { samples: words[1] - 1, ..block };
        assert!(decode_sound(&h, &SoundBlocks { sections: vec![vec![short]] }).is_err());
    }
}
