use super::*;

pub trait MusicSource: Send {
    fn render(&mut self, out: &mut [f32]);

    fn finished(&self) -> bool {
        false
    }
}

pub struct OutputMix {
    pub music: Option<Box<dyn MusicSource>>,
    pub music_tail: Option<Box<dyn MusicSource>>,
    music_rate: u32,
    resampler: crate::sfx_mixer::StreamResampler,
    tail_resampler: crate::sfx_mixer::StreamResampler,
    pub sfx: Option<SfxEngine>,
    pub music_volume: f32,
    pub sfx_volume: f32,
    pub remix_event_type: i32,
    pub swapped: bool,
    scratch: Vec<f32>,
}

impl OutputMix {
    pub fn new(music: Option<Box<dyn MusicSource>>, music_rate: u32, sfx: Option<SfxEngine>) -> Self {
        Self {
            music,
            music_tail: None,
            music_rate,
            resampler: crate::sfx_mixer::StreamResampler::new(music_rate, crate::sfx_aems::MIXER_RATE),
            tail_resampler: crate::sfx_mixer::StreamResampler::new(music_rate, crate::sfx_aems::MIXER_RATE),
            sfx,
            music_volume: 1.0,
            sfx_volume: 1.0,
            remix_event_type: 1,
            swapped: false,
            scratch: Vec::new(),
        }
    }

    pub fn music_rate(&self) -> u32 {
        self.music_rate
    }

    pub fn render(&mut self, out: &mut [f32]) {
        self.render_channels(out, 2);
    }

    pub fn render_channels(&mut self, out: &mut [f32], channels: usize) {
        out.fill(0.0);
        if self.music_tail.as_ref().is_some_and(|t| t.finished()) {
            self.music_tail = None;
        }
        if let Some(sfx) = self.sfx.as_mut() {
            sfx.set_trims(self.sfx_volume, self.music_volume);
            sfx.mixer.set_music(self.music.is_some() || self.music_tail.is_some());
            sfx.mixer.sources.music = self.music.take();
            sfx.mixer.sources.music_tail = self.music_tail.take();
            sfx.mixer.sources.music_rate = self.music_rate;
            if channels == crate::sfx_bus::DAC_CHANNELS {
                sfx.render_dac(out);
            } else {
                sfx.render(out);
            }
            self.music = sfx.mixer.sources.music.take();
            self.music_tail = sfx.mixer.sources.music_tail.take();
            return;
        }
        if channels != 2 {
            let mut stereo = vec![0.0f32; out.len() / channels * 2];
            self.render_channels(&mut stereo, 2);
            for (o, s) in out.chunks_exact_mut(channels).zip(stereo.as_chunks::<2>().0) {
                (o[0], o[1]) = (s[0], s[1]);
            }
            return;
        }
        for (m, r) in [(&mut self.music, &mut self.resampler), (&mut self.music_tail, &mut self.tail_resampler)] {
            if let Some(m) = m.as_mut() {
                self.scratch.resize(out.len(), 0.0);
                r.render(&mut self.scratch, &mut |b| m.render(b));
                for (o, x) in out.iter_mut().zip(&self.scratch) {
                    *o += x * self.music_volume;
                }
            }
        }
    }
}
