use symphonia::core::codecs::audio::well_known::CODEC_ID_VORBIS;
use symphonia::core::codecs::audio::{AudioCodecParameters, AudioDecoder, AudioDecoderOptions};
use symphonia::core::packet::Packet;
use symphonia::core::units::{Duration, Timestamp};
use symphonia::default::codecs::VorbisDecoder;

use ssx_content::movie::{AudioTrack, Packet as MoviePacket};

pub const MAX_ABS_SAMPLE: f32 = 4.0;
const MAX_PACKET_FRAMES: usize = 4096;

pub struct VorbisStream {
    decoder: Box<dyn AudioDecoder>,
    pub channels: usize,
    pub rate: u32,
    pub decoded: u64,
    max_frames: u64,
    packet: Vec<f32>,
}

impl VorbisStream {
    pub fn new(track: &AudioTrack, duration_ns: u64, overrun_ns: u64) -> Result<Self, String> {
        let channels = symphonia::core::audio::Channels::Discrete(u16::from(track.channels));
        let mut params = AudioCodecParameters::new();
        params
            .for_codec(CODEC_ID_VORBIS)
            .with_sample_rate(track.rate)
            .with_channels(channels)
            .with_extra_data(track.codec_private.clone().into_boxed_slice());
        let decoder = VorbisDecoder::try_new(&params, &AudioDecoderOptions::default())
            .map_err(|e| format!("Vorbis: the decoder refuses the headers: {e}"))?;
        let rate = u64::from(track.rate);
        let max_frames = (u128::from(duration_ns + overrun_ns) * u128::from(rate) / 1_000_000_000) as u64;
        Ok(Self {
            decoder: Box::new(decoder),
            channels: usize::from(track.channels),
            rate: track.rate,
            decoded: 0,
            max_frames,
            packet: Vec::new(),
        })
    }

    pub fn decode(&mut self, p: &MoviePacket, out: &mut Vec<f32>) -> Result<(), String> {
        let at = self.decoded;
        let packet = Packet::new(0, Timestamp::new(0), Duration::new(0), p.data.as_slice());
        let buf = self.decoder.decode(&packet).map_err(|e| format!("Vorbis: damaged at frame {at}: {e}"))?;
        let (frames, channels, rate) = (buf.frames(), buf.spec().channels().count(), buf.spec().rate());
        if frames == 0 {
            return Ok(());
        }
        if channels != self.channels || rate != self.rate || frames > MAX_PACKET_FRAMES {
            return Err(format!(
                "Vorbis: damaged at frame {at}: a packet of {frames} frames, {channels} channels at {rate} Hz"
            ));
        }
        buf.copy_to_vec_interleaved(&mut self.packet);
        if self.packet.len() != frames * self.channels {
            return Err(format!("Vorbis: damaged at frame {at}: a packet's samples do not fill its frames"));
        }
        if let Some(v) = self.packet.iter().find(|x| !x.is_finite() || x.abs() > MAX_ABS_SAMPLE) {
            return Err(format!(
                "Vorbis: damaged at frame {at}: it decodes to {v} (audio stays within ±{MAX_ABS_SAMPLE})"
            ));
        }
        let from = 0usize;
        let mut to = frames;
        if p.discard_padding_ns > 0 {
            let ns = u128::try_from(p.discard_padding_ns).unwrap_or(0);
            let cut = (ns * u128::from(self.rate)).div_ceil(1_000_000_000);
            to = to.saturating_sub(usize::try_from(cut).unwrap_or(usize::MAX)).max(from);
        }
        let n = (to - from) as u64;
        if self.decoded + n > self.max_frames {
            return Err(format!(
                "Vorbis: the audio runs past the movie's duration ({} frames at most)",
                self.max_frames
            ));
        }
        out.extend_from_slice(&self.packet[from * self.channels..to * self.channels]);
        self.decoded += n;
        Ok(())
    }
}
