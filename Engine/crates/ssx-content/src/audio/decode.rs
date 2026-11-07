use std::io::Cursor;
use std::sync::Arc;

use symphonia::core::codecs::CodecParameters;
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::formats::{FormatOptions, FormatReader, TrackType};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::default::codecs::{FlacDecoder, PcmDecoder, VorbisDecoder};
use symphonia::default::formats::{FlacReader, OggReader, WavReader};

use super::{AudioError, AudioInfo, Format, Limits, MAX_ABS_SAMPLE, Pcm, probe};

const MAX_PACKET_FRAMES: usize = 65_536;

pub struct Decoder {
    name: String,
    info: AudioInfo,
    bytes: Arc<[u8]>,
    reader: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track: u32,
    decoded: u64,
    finished: bool,
    packet: Vec<f32>,
}

impl Decoder {
    pub fn open(name: &str, bytes: Arc<[u8]>, limits: &Limits) -> Result<Self, AudioError> {
        let info = probe(name, &bytes, limits)?;
        let Built { reader, decoder, track } = build(name, &info, bytes.clone())?;
        Ok(Self {
            name: name.to_string(),
            info,
            bytes,
            reader,
            decoder,
            track,
            decoded: 0,
            finished: false,
            packet: Vec::new(),
        })
    }

    pub fn info(&self) -> &AudioInfo {
        &self.info
    }

    pub fn position(&self) -> u64 {
        self.decoded
    }

    fn fail(&self, message: impl Into<String>) -> AudioError {
        AudioError::new(&self.name, message)
    }

    pub fn next(&mut self, out: &mut Vec<f32>) -> Result<bool, AudioError> {
        if self.finished {
            return Ok(false);
        }
        loop {
            let packet = match self.reader.next_packet() {
                Ok(Some(p)) => p,
                Ok(None) => return self.end(),
                Err(e) => {
                    self.finished = true;
                    return Err(self.fail(format!("damaged at frame {}: {e}", self.decoded)));
                }
            };
            if packet.track_id != self.track {
                continue;
            }
            let at = self.decoded;
            let buf = match self.decoder.decode(&packet) {
                Ok(b) => b,
                Err(e) => {
                    self.finished = true;
                    return Err(AudioError::new(&self.name, format!("damaged at frame {at}: {e}")));
                }
            };
            let (frames, channels, rate) = (buf.frames(), buf.spec().channels().count(), buf.spec().rate());
            if frames == 0 {
                continue;
            }
            let want = usize::from(self.info.channels);
            let problem = if channels != want || rate != self.info.rate {
                Some(format!(
                    "a packet of {channels} channels at {rate} Hz in a stream of {want} at {}",
                    self.info.rate
                ))
            } else if frames > MAX_PACKET_FRAMES {
                Some(format!("a packet of {frames} frames (at most {MAX_PACKET_FRAMES})"))
            } else if at + frames as u64 > self.info.frames {
                Some(format!("decodes past its stated length of {} frames", self.info.frames))
            } else {
                None
            };
            if let Some(p) = problem {
                self.finished = true;
                return Err(AudioError::new(&self.name, format!("damaged at frame {at}: {p}")));
            }
            buf.copy_to_vec_interleaved(&mut self.packet);
            if self.packet.len() != frames * want {
                self.finished = true;
                return Err(self.fail(format!("damaged at frame {at}: a packet's samples do not fill its frames")));
            }
            if let Some(k) = self.packet.iter().position(|x| !x.is_finite() || x.abs() > MAX_ABS_SAMPLE) {
                let v = self.packet[k];
                self.finished = true;
                let frame = at + (k / want) as u64;
                return Err(self.fail(format!(
                    "damaged at frame {frame}: it decodes to {v} (audio stays within ±{MAX_ABS_SAMPLE})"
                )));
            }
            out.extend_from_slice(&self.packet);
            self.decoded += frames as u64;
            return Ok(true);
        }
    }

    fn end(&mut self) -> Result<bool, AudioError> {
        self.finished = true;
        if self.decoded != self.info.frames {
            return Err(self.fail(format!(
                "ends after {} frames, its header states {} (cut short or damaged)",
                self.decoded, self.info.frames
            )));
        }
        if self.info.format == Format::Flac && self.decoder.finalize().verify_ok != Some(true) {
            return Err(self.fail("the decoded audio does not match the MD5 its header states (damaged)"));
        }
        Ok(false)
    }

    pub fn rewind(&mut self) -> Result<(), AudioError> {
        let Built { reader, decoder, track } = build(&self.name, &self.info, self.bytes.clone())?;
        (self.reader, self.decoder, self.track) = (reader, decoder, track);
        (self.decoded, self.finished) = (0, false);
        Ok(())
    }
}

struct Built {
    reader: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track: u32,
}

fn build(name: &str, info: &AudioInfo, bytes: Arc<[u8]>) -> Result<Built, AudioError> {
    let fail = |m: String| AudioError::new(name, m);
    let mss = MediaSourceStream::new(Box::new(Cursor::new(bytes)), MediaSourceStreamOptions::default());
    let opts = FormatOptions::default();
    let reader: Box<dyn FormatReader> = match info.format {
        Format::OggVorbis => Box::new(OggReader::try_new(mss, opts).map_err(|e| fail(format!("OGG: {e}")))?),
        Format::Flac => Box::new(FlacReader::try_new(mss, opts).map_err(|e| fail(format!("FLAC: {e}")))?),
        Format::Wav => Box::new(WavReader::try_new(mss, opts).map_err(|e| fail(format!("WAV: {e}")))?),
    };
    let audio_tracks = reader.tracks().iter().filter(|t| matches!(t.codec_params, Some(CodecParameters::Audio(_))));
    if audio_tracks.count() != 1 {
        return Err(fail("not exactly one audio stream".into()));
    }
    let track = reader.default_track(TrackType::Audio).ok_or_else(|| fail("no audio stream".into()))?;
    let Some(CodecParameters::Audio(params)) = &track.codec_params else {
        return Err(fail("no audio stream".into()));
    };
    let id = track.id;
    let dopts = AudioDecoderOptions::default().gapless(true).verify(info.format == Format::Flac);
    let decoder: Box<dyn AudioDecoder> = match info.format {
        Format::OggVorbis => {
            Box::new(VorbisDecoder::try_new(params, &dopts).map_err(|e| fail(format!("Vorbis: {e}")))?)
        }
        Format::Flac => Box::new(FlacDecoder::try_new(params, &dopts).map_err(|e| fail(format!("FLAC: {e}")))?),
        Format::Wav => Box::new(PcmDecoder::try_new(params, &dopts).map_err(|e| fail(format!("WAV: {e}")))?),
    };
    Ok(Built { reader, decoder, track: id })
}

pub fn decode_file(name: &str, bytes: Arc<[u8]>, limits: &Limits) -> Result<Pcm, AudioError> {
    let mut d = Decoder::open(name, bytes, limits)?;
    let total = usize::try_from(d.info.frames.saturating_mul(u64::from(d.info.channels)))
        .map_err(|_| AudioError::new(name, "too long for this machine"))?;
    let mut samples = Vec::with_capacity(total);
    while d.next(&mut samples)? {}
    Ok(Pcm { channels: d.info.channels, rate: d.info.rate, samples })
}
