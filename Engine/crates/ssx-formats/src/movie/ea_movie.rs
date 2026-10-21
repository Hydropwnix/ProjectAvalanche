use crate::ealayer3::Frame;
use crate::ealayer3_decode::{WARM_UP_SAMPLES, frame_granule};
use crate::mp3_layer3::{ChannelState, GRANULE_SAMPLES, decode_granule};
use crate::{Error, Result};

#[derive(Clone, Copy, Debug)]
pub struct Chunk<'a> {
    pub tag: [u8; 4],
    pub offset: usize,
    pub body: &'a [u8],
}

pub fn chunks(data: &[u8]) -> Result<Vec<Chunk<'_>>> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        let head =
            data.get(pos..pos + 8).ok_or_else(|| Error::invalid(format!("chunk header past end at {pos:#x}")))?;
        let size = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
        if size < 8 || pos + size > data.len() {
            return Err(Error::invalid(format!("chunk at {pos:#x}: size {size} out of range")));
        }
        out.push(Chunk { tag: [head[0], head[1], head[2], head[3]], offset: pos, body: &data[pos + 8..pos + size] });
        pos += size;
    }
    Ok(out)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoHeader {
    pub codec: [u8; 4],
    pub width: u16,
    pub height: u16,
    pub frame_count: u32,
    pub unk_0c: u32,
    pub rate: u32,
    pub scale: u32,
}

impl VideoHeader {
    pub fn parse(body: &[u8]) -> Result<Self> {
        if body.len() != 24 {
            return Err(Error::invalid(format!("MVhd body of {} bytes (24 expected)", body.len())));
        }
        let u32le = |o: usize| u32::from_le_bytes([body[o], body[o + 1], body[o + 2], body[o + 3]]);
        let h = Self {
            codec: [body[0], body[1], body[2], body[3]],
            width: u16::from_le_bytes([body[4], body[5]]),
            height: u16::from_le_bytes([body[6], body[7]]),
            frame_count: u32le(8),
            unk_0c: u32le(12),
            rate: u32le(16),
            scale: u32le(20),
        };
        if h.rate == 0 || h.scale == 0 {
            return Err(Error::invalid("MVhd: zero frame rate"));
        }
        Ok(h)
    }

    pub fn fps(&self) -> f64 {
        f64::from(self.rate) / f64::from(self.scale)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioHeader {
    pub unk_gstr: u32,
    pub block_rate: u32,
    pub version: u32,
    pub num_samples: u32,
    pub channels: u32,
    pub codec2: u32,
    pub sample_rate: u32,
    pub patches: Vec<(u8, u32)>,
}

pub const CODEC2_EALAYER3: u32 = 0x17;

impl AudioHeader {
    pub fn parse(body: &[u8]) -> Result<Self> {
        if body.get(..4) != Some(b"GSTR") || body.len() < 8 {
            return Err(Error::invalid("audio header without GSTR"));
        }
        let unk_gstr = u32::from_be_bytes([body[4], body[5], body[6], body[7]]);
        let mut patches = Vec::new();
        let mut p = 8;
        loop {
            let t = *body.get(p).ok_or_else(|| Error::invalid("audio header: no 0xFF end"))?;
            p += 1;
            match t {
                0xFF => break,
                0xFD => patches.push((t, 0)),
                0x00 | 0x80 | 0x82 | 0x84 | 0x85 | 0xA0 => {
                    let n = usize::from(*body.get(p).ok_or_else(|| Error::invalid("audio header: patch past end"))?);
                    let v = body.get(p + 1..p + 1 + n).filter(|_| n <= 4);
                    let v = v.ok_or_else(|| Error::invalid(format!("audio header: patch {t:#04x} of {n} bytes")))?;
                    patches.push((t, v.iter().fold(0u32, |a, &b| a << 8 | u32::from(b))));
                    p += 1 + n;
                }
                _ => return Err(Error::invalid(format!("audio header: unknown patch {t:#04x}"))),
            }
        }
        if body[p..].iter().any(|&b| b != 0) {
            return Err(Error::invalid("audio header: nonzero bytes after 0xFF"));
        }
        let get = |t: u8| patches.iter().find(|p| p.0 == t).map(|p| p.1);
        let need = |t: u8| get(t).ok_or_else(|| Error::invalid(format!("audio header: no patch {t:#04x}")));
        Ok(Self {
            unk_gstr,
            block_rate: need(0x00)?,
            version: need(0x80)?,
            num_samples: need(0x85)?,
            channels: need(0x82)?,
            codec2: need(0xA0)?,
            sample_rate: need(0x84)?,
            patches,
        })
    }
}

#[derive(Clone, Debug)]
pub struct AudioBlock<'a> {
    pub samples: u32,
    pub offsets: Vec<u32>,
    pub data: &'a [u8],
    pub video_frames_before: usize,
}

impl<'a> AudioBlock<'a> {
    fn parse(body: &'a [u8], channels: usize, video_frames_before: usize) -> Result<Self> {
        let words = 1 + channels;
        if body.len() < 4 * words {
            return Err(Error::invalid("audio block shorter than its offset table"));
        }
        let word = |i: usize| u32::from_be_bytes([body[4 * i], body[4 * i + 1], body[4 * i + 2], body[4 * i + 3]]);
        let offsets: Vec<u32> = (1..words).map(word).collect();
        Ok(Self { samples: word(0), offsets, data: &body[4 * words..], video_frames_before })
    }

    pub fn streams(&self) -> Result<Vec<&'a [u8]>> {
        let starts: Vec<usize> = if self.offsets.len() > 2 {
            self.offsets.iter().map(|&o| o as usize).collect()
        } else {
            vec![self.offsets[0] as usize]
        };
        if starts[0] != 0
            || starts.windows(2).any(|w| w[0] > w[1])
            || starts.last().is_some_and(|&s| s > self.data.len())
        {
            return Err(Error::invalid(format!(
                "audio block: channel offsets {:?} in {} bytes",
                self.offsets,
                self.data.len()
            )));
        }
        Ok((0..starts.len())
            .map(|i| &self.data[starts[i]..starts.get(i + 1).copied().unwrap_or(self.data.len())])
            .collect())
    }
}

#[derive(Clone, Debug)]
pub struct AudioStream<'a> {
    pub lang: Option<[u8; 2]>,
    pub header: AudioHeader,
    pub block_count: u32,
    pub blocks: Vec<AudioBlock<'a>>,
}

#[derive(Clone, Copy, Debug)]
pub struct VideoFrame<'a> {
    pub key: bool,
    pub offset: usize,
    pub data: &'a [u8],
}

#[derive(Clone, Debug)]
pub struct Movie<'a> {
    pub video: VideoHeader,
    pub frames: Vec<VideoFrame<'a>>,
    pub audio: Vec<AudioStream<'a>>,
}

fn audio_tag(tag: [u8; 4]) -> Option<(u8, Option<[u8; 2]>)> {
    match &tag {
        b"SCHl" => Some((b'H', None)),
        b"SCCl" => Some((b'C', None)),
        b"SCDl" => Some((b'D', None)),
        [b'S', k @ (b'H' | b'C' | b'D'), a, b] if a.is_ascii_lowercase() && b.is_ascii_lowercase() => {
            Some((*k, Some([*a, *b])))
        }
        _ => None,
    }
}

impl<'a> Movie<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        let chunks = chunks(data)?;
        let first =
            chunks.first().filter(|c| &c.tag == b"MVhd").ok_or_else(|| Error::invalid("movie: no MVhd first"))?;
        let video = VideoHeader::parse(first.body)?;
        let mut frames = Vec::new();
        let mut audio: Vec<AudioStream> = Vec::new();
        for c in &chunks[1..] {
            let tag = String::from_utf8_lossy(&c.tag).into_owned();
            if &c.tag == b"MV0K" || &c.tag == b"MV0F" {
                frames.push(VideoFrame { key: &c.tag == b"MV0K", offset: c.offset, data: c.body });
                continue;
            }
            let (kind, lang) =
                audio_tag(c.tag).ok_or_else(|| Error::invalid(format!("movie: chunk {tag} at {:#x}", c.offset)))?;
            let stream = audio.iter_mut().find(|s| s.lang == lang);
            match (kind, stream) {
                (b'H', None) => audio.push(AudioStream {
                    lang,
                    header: AudioHeader::parse(c.body)?,
                    block_count: 0,
                    blocks: Vec::new(),
                }),
                (b'C', Some(s)) if c.body.len() == 4 && s.block_count == 0 => {
                    s.block_count = u32::from_le_bytes([c.body[0], c.body[1], c.body[2], c.body[3]]);
                }
                (b'D', Some(s)) => {
                    let ch = s.header.channels as usize;
                    s.blocks.push(AudioBlock::parse(c.body, ch, frames.len())?);
                }
                _ => return Err(Error::invalid(format!("movie: {tag} at {:#x} out of order", c.offset))),
            }
        }
        if frames.len() != video.frame_count as usize || !frames.first().is_some_and(|f| f.key) {
            return Err(Error::invalid(format!("movie: {} frames, MVhd says {}", frames.len(), video.frame_count)));
        }
        for s in &audio {
            if s.blocks.len() != s.block_count as usize {
                return Err(Error::invalid(format!(
                    "audio stream: {} blocks, count {}",
                    s.blocks.len(),
                    s.block_count
                )));
            }
        }
        Ok(Self { video, frames, audio })
    }
}

pub const OUTPUT_SCALE: f32 = 1.0 / 32767.0;

pub const LANGUAGE_CODES: [&str; 6] = ["eng", "fr", "ger", "sp", "it", "jpn"];

pub fn language_stream(language: usize) -> Option<[u8; 2]> {
    let code = LANGUAGE_CODES.get(language).filter(|_| (1..=4).contains(&language))?.as_bytes();
    Some([code[0], code[1]])
}

impl<'a> Movie<'a> {
    pub fn audio_stream(&self, lang: Option<[u8; 2]>) -> Option<&AudioStream<'a>> {
        self.audio.iter().find(|s| s.lang == lang).or_else(|| self.audio.iter().find(|s| s.lang.is_none()))
    }
}

#[derive(Clone, Debug, Default)]
pub struct DecodedAudio {
    pub channels: usize,
    pub sample_rate: u32,
    pub samples: Vec<f32>,
    pub pcm_blocks: usize,
    pub pcm_checked: usize,
    pub pcm_rule_best: usize,
}

pub struct StreamDecoder {
    channels: usize,
    states: Vec<ChannelState>,
    raw: Vec<f32>,
    base: u64,
    decoded: u64,
    out: u64,
    pub pcm_blocks: usize,
    pub pcm_checked: usize,
    pub pcm_rule_best: usize,
    pub evidence: bool,
}

impl StreamDecoder {
    pub fn new(channels: usize) -> Result<Self> {
        if !(1..=2).contains(&channels) {
            return Err(Error::invalid(format!("movie audio: {channels}-channel EALayer3 stream")));
        }
        let w = WARM_UP_SAMPLES as u64;
        Ok(Self {
            channels,
            states: vec![ChannelState::default(); channels],
            raw: Vec::new(),
            base: 0,
            decoded: 0,
            out: w,
            pcm_blocks: 0,
            pcm_checked: 0,
            pcm_rule_best: 0,
            evidence: true,
        })
    }

    fn check_alignment(&mut self, pcm: &[f32], at: usize) {
        let energy: f64 = pcm.iter().map(|x| f64::from(*x).powi(2)).sum();
        if (energy / pcm.len() as f64).sqrt() <= 0.005 {
            return;
        }
        let ch = self.channels;
        let err = |p: usize| -> f64 { pcm.iter().zip(&self.raw[p..]).map(|(a, b)| f64::from(a - b).powi(2)).sum() };
        let rule = err(at);
        let best =
            (0..=self.raw.len() - pcm.len()).step_by(ch).filter(|&p| p != at).map(err).fold(f64::INFINITY, f64::min);
        self.pcm_checked += 1;
        self.pcm_rule_best += usize::from(rule < best);
    }

    pub fn decode_block(&mut self, payload: &[u8], samples: usize, out: &mut Vec<f32>) -> Result<()> {
        let ch = self.channels;
        let mut pos = 0;
        let mut pcm = [[0f32; GRANULE_SAMPLES]; 2];
        while pos < payload.len() {
            let rest = &payload[pos..];
            if rest.iter().all(|&b| b == 0) {
                break;
            }
            if rest[0] == 0xEE {
                let head = rest.get(1..5).ok_or_else(|| Error::invalid("movie audio: PCM frame header past end"))?;
                let (offset, count) = (
                    u64::from(u16::from_be_bytes([head[0], head[1]])),
                    usize::from(u16::from_be_bytes([head[2], head[3]])),
                );
                let body =
                    rest.get(5..5 + 2 * count * ch).ok_or_else(|| Error::invalid("movie audio: PCM frame past end"))?;
                let start =
                    self.decoded.checked_sub(offset).filter(|&s| s >= self.out && s + count as u64 <= self.decoded);
                let start = start.ok_or_else(|| {
                    Error::invalid(format!("movie audio: PCM block ({offset}, {count}) after {} samples", self.decoded))
                })?;
                let at = (start - self.base) as usize * ch;
                let pcm: Vec<f32> =
                    body.as_chunks::<2>().0.iter().map(|&v| f32::from(i16::from_be_bytes(v)) / 32768.0).collect();
                if self.evidence {
                    self.check_alignment(&pcm, at);
                }
                self.raw[at..at + pcm.len()].copy_from_slice(&pcm);
                self.pcm_blocks += 1;
                pos += 5 + body.len();
                continue;
            }
            let f =
                Frame::parse(rest).map_err(|e| Error::invalid(format!("movie audio at payload byte {pos}: {e}")))?;
            if f.flag != 0 || f.channels as usize != ch {
                return Err(Error::invalid(format!("movie audio: frame flag {:#x}, {} channels", f.flag, f.channels)));
            }
            let data = &rest[..f.size];
            let (g, bit) = frame_granule(&f, data)?;
            decode_granule(&mut self.states, &g, data, bit, &mut pcm)?;
            for i in 0..GRANULE_SAMPLES {
                for c in &pcm[..ch] {
                    self.raw.push(c[i]);
                }
            }
            self.decoded += GRANULE_SAMPLES as u64;
            pos += f.size;
        }
        if self.out + samples as u64 > self.decoded {
            return Err(Error::invalid(format!(
                "movie audio: block wants {samples} samples, {} decoded",
                self.decoded.saturating_sub(self.out)
            )));
        }
        let from = (self.out - self.base) as usize * ch;
        out.extend(self.raw[from..from + samples * ch].iter().map(|x| x * 32768.0 * OUTPUT_SCALE));
        self.out += samples as u64;
        self.raw.drain(..from + samples * ch);
        self.base = self.out;
        Ok(())
    }
}

pub struct AudioDecoder {
    pub channels: usize,
    pub sample_rate: u32,
    per_stream: usize,
    decoders: Vec<StreamDecoder>,
    block_pcm: Vec<Vec<f32>>,
}

impl AudioDecoder {
    pub fn new(h: &AudioHeader, evidence: bool) -> Result<Self> {
        if h.codec2 != CODEC2_EALAYER3 {
            return Err(Error::invalid(format!("movie audio: codec2 {:#x} (only EALayer3 on the disc)", h.codec2)));
        }
        let ch = h.channels as usize;
        let per_stream = if ch > 2 { 1 } else { ch };
        let mut decoders = (0..ch / per_stream).map(|_| StreamDecoder::new(per_stream)).collect::<Result<Vec<_>>>()?;
        for d in &mut decoders {
            d.evidence = evidence;
        }
        let block_pcm = vec![Vec::new(); decoders.len()];
        Ok(Self { channels: ch, sample_rate: h.sample_rate, per_stream, decoders, block_pcm })
    }

    pub fn decode_block(&mut self, b: &AudioBlock, out: &mut Vec<f32>) -> Result<()> {
        let want = b.samples as usize;
        let per = self.per_stream;
        for ((dec, payload), pcm) in self.decoders.iter_mut().zip(b.streams()?).zip(&mut self.block_pcm) {
            pcm.clear();
            dec.decode_block(payload, want, pcm)?;
        }
        out.reserve(want * self.channels);
        for t in 0..want {
            for pcm in &self.block_pcm {
                out.extend_from_slice(&pcm[t * per..(t + 1) * per]);
            }
        }
        Ok(())
    }
}

pub fn decode_audio(s: &AudioStream) -> Result<DecodedAudio> {
    let h = &s.header;
    let mut dec = AudioDecoder::new(h, true)?;
    let ch = dec.channels;
    let mut samples = Vec::with_capacity(h.num_samples as usize * ch);
    for (i, b) in s.blocks.iter().enumerate() {
        dec.decode_block(b, &mut samples).map_err(|e| Error::invalid(format!("block {i}: {e}")))?;
    }
    if samples.len() != h.num_samples as usize * ch {
        return Err(Error::invalid(format!(
            "movie audio: {} samples, header says {}",
            samples.len() / ch,
            h.num_samples
        )));
    }
    let mut d = DecodedAudio { channels: ch, sample_rate: h.sample_rate, samples, ..Default::default() };
    for dec in &dec.decoders {
        d.pcm_blocks += dec.pcm_blocks;
        d.pcm_checked += dec.pcm_checked;
        d.pcm_rule_best += dec.pcm_rule_best;
    }
    Ok(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(tag: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut c = tag.to_vec();
        c.extend((8 + body.len() as u32).to_le_bytes());
        c.extend(body);
        c
    }

    fn header(channels: u8, samples: u32) -> Vec<u8> {
        let mut h = b"GSTR\x01\x00\x00\x00".to_vec();
        h.extend([0x00, 4, 0x1E, 0, 0, 0, 0xFD, 0x80, 4, 0, 0, 0, 3, 0x85, 4]);
        h.extend(samples.to_be_bytes());
        h.extend([0x82, 4, 0, 0, 0, channels, 0xA0, 4, 0, 0, 0, 0x17, 0x84, 4, 0, 0, 0xBB, 0x80, 0xFF, 0, 0]);
        h
    }

    fn silent_granule(granule: u8) -> Vec<u8> {
        let mut f = vec![0x00, 0xDC, granule << 7];
        f.resize(10, 0);
        f
    }

    #[test]
    fn audio_header_patches() {
        let h = AudioHeader::parse(&header(6, 401_600)).unwrap();
        assert_eq!((h.channels, h.sample_rate, h.num_samples, h.codec2, h.version), (6, 48_000, 401_600, 0x17, 3));
        assert_eq!(h.block_rate, 30 << 24);
        assert!(AudioHeader::parse(b"GSTR\x01\x00\x00\x00\x55\x01\x00\xFF").is_err());
    }

    #[test]
    fn chunks_must_tile() {
        let mut d = chunk(b"MVhd", &[0; 24]);
        assert_eq!(chunks(&d).unwrap().len(), 1);
        d.push(0);
        assert!(chunks(&d).is_err());
    }

    #[test]
    fn movie_structure_and_language_streams() {
        let mut mvhd = b"vp60".to_vec();
        mvhd.extend(1280u16.to_le_bytes());
        mvhd.extend(720u16.to_le_bytes());
        mvhd.extend(2u32.to_le_bytes());
        mvhd.extend(0u32.to_le_bytes());
        mvhd.extend(300_000u32.to_le_bytes());
        mvhd.extend(10_000u32.to_le_bytes());
        let mut block = 7u32.to_be_bytes().to_vec();
        block.extend([0u8; 8]);
        block.extend([1, 2, 3]);
        let mut d = chunk(b"MVhd", &mvhd);
        d.extend(chunk(b"SCHl", &header(2, 7)));
        d.extend(chunk(b"SHfr", &header(2, 7)));
        d.extend(chunk(b"MV0K", &[0]));
        d.extend(chunk(b"SCCl", &1u32.to_le_bytes()));
        d.extend(chunk(b"SCfr", &1u32.to_le_bytes()));
        d.extend(chunk(b"MV0F", &[0x80]));
        d.extend(chunk(b"SCDl", &block));
        d.extend(chunk(b"SDfr", &block));
        let m = Movie::parse(&d).unwrap();
        assert_eq!((m.video.width, m.video.frame_count, m.video.fps()), (1280, 2, 30.0));
        assert_eq!(m.frames.iter().map(|f| f.key).collect::<Vec<_>>(), [true, false]);
        assert_eq!(m.audio.len(), 2);
        assert_eq!(m.audio[1].lang, Some(*b"fr"));
        let b = &m.audio[0].blocks[0];
        assert_eq!((b.samples, b.video_frames_before, b.streams().unwrap()), (7, 2, vec![&[1u8, 2, 3][..]]));
        let mut bad = chunk(b"MVhd", &mvhd);
        bad.extend(chunk(b"SDge", &block));
        assert!(Movie::parse(&bad).is_err());
    }

    #[test]
    fn pcm_frame_overwrites_decoded_samples_before_the_granule_count() {
        let mut p = silent_granule(0);
        p.extend(silent_granule(1));
        p.extend([0xEE, 0, 47, 0, 47]);
        for i in 0..47i16 {
            p.extend((i * 100).to_be_bytes());
        }
        p.extend(silent_granule(0));
        p.extend(silent_granule(1));
        let mut dec = StreamDecoder::new(1).unwrap();
        let mut out = Vec::new();
        dec.decode_block(&p, 1199, &mut out).unwrap();
        assert_eq!(out.len(), 1199);
        assert_eq!(out[1], 100.0 * OUTPUT_SCALE);
        assert_eq!(out[46], 4600.0 * OUTPUT_SCALE);
        assert!(out[47..].iter().all(|&v| v == 0.0));
        let mut late = silent_granule(0);
        late.extend([0xEE, 0x04, 0x00, 0, 1, 0, 0]);
        assert!(dec.decode_block(&late, 0, &mut out).is_err());
    }
}
