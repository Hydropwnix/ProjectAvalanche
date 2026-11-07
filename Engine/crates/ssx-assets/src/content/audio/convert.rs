use std::sync::Arc;

use ssx_content::audio::Pcm;

use crate::sfx::output::resample::Resampler;
use crate::sfx_override::Samples48;

const CHUNK: usize = 4096;

pub fn to_rate(pcm: Pcm, to: u32) -> Vec<f32> {
    let ch = usize::from(pcm.channels.max(1));
    if pcm.rate == to {
        return pcm.samples;
    }
    let frames_in = pcm.samples.len() / ch;
    let frames_out = (frames_in as u64 * u64::from(to)).div_ceil(u64::from(pcm.rate.max(1))) as usize;
    let mut r = Resampler::with_channels(pcm.rate, to, ch);
    let mut out = vec![0.0f32; frames_out * ch];
    let (mut fed, mut done) = (0usize, 0usize);
    while done < frames_out {
        let n = CHUNK.min(frames_out - done);
        let need = r.need(n);
        let mut input = vec![0.0f32; need * ch];
        let take = need.min(frames_in.saturating_sub(fed));
        input[..take * ch].copy_from_slice(&pcm.samples[fed * ch..(fed + take) * ch]);
        fed += take;
        r.process(&input, &mut out[done * ch..(done + n) * ch]);
        done += n;
    }
    out
}

pub fn samples48(file: &str, pcm: Pcm) -> Arc<Samples48> {
    let channels = usize::from(pcm.channels.max(1));
    Arc::new(Samples48 { file: file.to_string(), channels, samples: to_rate(pcm, crate::sfx_aems::MIXER_RATE) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tone_keeps_its_length_and_level_across_rates() {
        let rate = 22_050u32;
        let frames = rate as usize;
        let samples: Vec<f32> =
            (0..frames).map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / rate as f32).sin() * 0.5).collect();
        let out = to_rate(Pcm { channels: 1, rate, samples }, 48_000);
        assert_eq!(out.len(), 48_000);
        let rms = (out[2000..46_000].iter().map(|x| x * x).sum::<f32>() / 44_000.0).sqrt();
        assert!((rms - 0.5 / std::f32::consts::SQRT_2).abs() < 0.005, "rms {rms}");
    }
}
