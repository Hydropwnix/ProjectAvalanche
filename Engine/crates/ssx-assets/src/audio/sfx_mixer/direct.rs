use super::*;

impl Mixer {
    pub fn start_pcm(
        &mut self,
        patch: &str,
        voice: &str,
        pcm: Vec<f32>,
        channels: usize,
        rate: u32,
        label: String,
    ) -> Option<u32> {
        let frames = pcm.len() / channels.max(1);
        let sound = Arc::new(RamSound { channels, looped: false, sections: vec![pcm] });
        let src = Source {
            frames: Frames::Ram(sound),
            channels,
            rate,
            num_samples: frames as u32,
            looped: false,
            resampler: Resampler::default(),
            paused: false,
            finished: false,
            label: label.clone(),
            fade: None,
        };
        self.start_source(patch, voice, src, label)
    }

    pub fn start_stream(&mut self, patch: &str, voice: &str, sound: StreamedSound, label: String) -> Option<u32> {
        let header = &sound.cursor.header;
        let channels = usize::from(header.channels);
        if channels > 8 {
            return None;
        }
        let src = Source {
            channels,
            rate: header.sample_rate,
            num_samples: header.num_samples,
            looped: false,
            frames: Frames::Stream { cursor: Box::new(sound.cursor), buf: Vec::new(), base: 0, ended: false },
            resampler: Resampler::default(),
            paused: false,
            finished: false,
            label: label.clone(),
            fade: None,
        };
        self.start_source(patch, voice, src, label)
    }

    fn start_source(&mut self, patch: &str, voice: &str, src: Source, label: String) -> Option<u32> {
        let channels = src.channels;
        let sid = self.sources.voices.iter().position(Option::is_none).unwrap_or(self.sources.voices.len());
        if sid == self.sources.voices.len() {
            self.sources.voices.push(None);
        }
        self.sources.voices[sid] = Some(src);
        let Some(slot) = self.graph.add_aems_voice(patch, voice, channels, sid as u32) else {
            self.sources.voices[sid] = None;
            eprintln!("sfx: speech patch {patch} / {voice} not found");
            return None;
        };
        self.graph.set_attr(slot, b"Rsp0", 0, 1.0);
        let h = self.pcm.iter().position(Option::is_none).unwrap_or(self.pcm.len());
        if h == self.pcm.len() {
            self.pcm.push(None);
        }
        self.pcm[h] = Some((slot, sid as u32));
        self.events.push(format!("speech voice {} start {label} → {patch}/{voice}", h + 1));
        Some(h as u32 + 1)
    }

    pub fn pcm_remaining(&mut self, handle: u32) -> Option<f32> {
        let (slot, sid) = (*self.pcm.get(handle as usize - 1)?)?;
        let left = self.sources.voices.get(sid as usize).and_then(Option::as_ref).and_then(|s| {
            (!s.finished).then(|| (f64::from(s.num_samples) - s.resampler.index as f64).max(0.0) as f32 / s.rate as f32)
        });
        if left.is_none() {
            self.graph.remove_voice(slot);
            self.sources.voices[sid as usize] = None;
            self.pcm[handle as usize - 1] = None;
        }
        left
    }

    pub fn pause_pcm(&mut self, handle: u32, paused: bool) {
        if let Some(Some((slot, _))) = self.pcm.get(handle as usize - 1) {
            self.graph.set_attr(*slot, b"Pau0", 0, if paused { 1.0 } else { 0.0 });
        }
    }

    pub fn pcm_left(&self, handle: u32) -> Option<f32> {
        let (_, sid) = (*self.pcm.get(handle as usize - 1)?)?;
        let s = self.sources.voices.get(sid as usize)?.as_ref()?;
        (!s.finished).then(|| (f64::from(s.num_samples) - s.resampler.index as f64).max(0.0) as f32 / s.rate as f32)
    }

    pub fn pcm_paused(&self, handle: u32) -> bool {
        matches!(self.pcm.get(handle as usize - 1), Some(Some((slot, _))) if self.graph.voice_paused(*slot))
    }

    pub fn fade_pcm(&mut self, handle: u32, secs: f32) {
        if let Some(Some((_, sid))) = self.pcm.get(handle as usize - 1)
            && let Some(s) = self.sources.voices.get_mut(*sid as usize).and_then(Option::as_mut)
        {
            let n = (secs * MIXER_RATE as f32).max(1.0);
            s.fade = Some((s.fade.map_or(1.0, |f| f.0), 1.0 / n));
        }
    }
}

pub struct StreamResampler {
    from: u32,
    to: u32,
    buf: Vec<f32>,
    pos: Resampler,
}

impl StreamResampler {
    pub fn new(from: u32, to: u32) -> Self {
        Self { from, to, buf: Vec::new(), pos: Resampler::default() }
    }

    pub fn render(&mut self, out: &mut [f32], source: &mut dyn FnMut(&mut [f32])) {
        let step = Resampler::step(self.from as f32 / self.to as f32);
        for f in 0..out.len() / 2 {
            let i = self.pos.index as usize;
            while self.buf.len() / 2 < i + 2 {
                let n = self.buf.len();
                self.buf.resize(n + 2048, 0.0);
                source(&mut self.buf[n..]);
            }
            for c in 0..2 {
                out[2 * f + c] = self.pos.sample(self.buf[2 * i + c], self.buf[2 * i + 2 + c]);
            }
            self.pos.advance(step);
        }
        let used = (self.pos.index as usize).min(self.buf.len() / 2);
        self.buf.drain(..used * 2);
        self.pos.index -= used as u64;
    }
}
