use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::sfx_aems::MIXER_RATE;
use crate::sfx_bus::CorePlugin;
use crate::sfx_dsp::{self, Resampler};

pub const BUS: &str = "Cinematic_BUS";
pub const VOICE: &str = "RWMovie Voice";
pub const PLUGINS: [&[u8; 4]; 5] = [b"PPl0", b"Rch0", b"Rsp0", b"Pau0", b"Sen0"];
pub const MOVIE_SOURCE: u32 = u32::MAX - 1;

pub struct MovieAudio {
    pub channels: usize,
    pub rate: u32,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    queue: VecDeque<f32>,
    finished: bool,
    stopped: bool,
    played: u64,
    attached: bool,
    started: bool,
    pos: Resampler,
}

impl MovieAudio {
    pub fn new(channels: usize, rate: u32) -> Arc<Self> {
        Arc::new(Self { channels: channels.max(1), rate: rate.max(1), state: Mutex::new(State::default()) })
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn push(&self, frames: &[f32]) {
        self.state().queue.extend(frames);
    }

    pub fn finish(&self) {
        self.state().finished = true;
    }

    pub fn stop(&self) {
        self.state().stopped = true;
    }

    pub fn attach(&self) {
        self.state().attached = true;
    }

    pub fn attached(&self) -> bool {
        self.state().attached
    }

    pub fn stopped(&self) -> bool {
        self.state().stopped
    }

    pub fn queued_frames(&self) -> usize {
        self.state().queue.len() / self.channels
    }

    pub fn played_seconds(&self) -> Option<f64> {
        let s = self.state();
        s.started.then(|| s.played as f64 / f64::from(self.rate))
    }

    pub fn is_over(&self) -> bool {
        let s = self.state();
        s.stopped || (s.finished && s.queue.len() < self.channels)
    }

    pub fn fill(&self, pitch: f32, out: &mut [Vec<f32>]) -> bool {
        let ch = self.channels;
        let mut s = self.state();
        s.started = true;
        if s.stopped {
            return false;
        }
        let n = out.first().map_or(0, Vec::len);
        let step = Resampler::step(pitch * self.rate as f32 / MIXER_RATE as f32);
        let mut probe = s.pos.clone();
        for _ in 0..n {
            probe.advance(step);
        }
        let need = probe.index as usize + 1;
        let have = s.queue.len() / ch;
        if have < need && !s.finished {
            out.iter_mut().for_each(|c| c.fill(0.0));
            return true;
        }
        let mut native = vec![vec![0.0f32; n]; ch];
        let frame = |q: &VecDeque<f32>, i: usize, c: usize| q.get(i * ch + c).copied().unwrap_or(0.0);
        for k in 0..n {
            let i = s.pos.index as usize;
            for (c, dst) in native.iter_mut().enumerate() {
                dst[k] = s.pos.sample(frame(&s.queue, i, c), frame(&s.queue, i + 1, c));
            }
            s.pos.advance(step);
        }
        let used = (s.pos.index as usize).min(have);
        s.queue.drain(..used * ch);
        s.pos.index -= used as u64;
        s.played += used as u64;
        let conv = if out.len() == ch { native } else { sfx_dsp::rechannel(&native, out.len()) };
        for (o, c) in out.iter_mut().zip(conv) {
            o.copy_from_slice(&c);
        }
        !(s.finished && s.queue.len() < ch)
    }

    pub fn plugins(&self) -> Vec<CorePlugin> {
        PLUGINS.iter().map(|f| CorePlugin { fourcc: **f, channels: self.channels, ctor: Vec::new() }).collect()
    }
}

#[cfg(feature = "movie-decode")]
impl ssx_movie::play::AudioSink for MovieAudio {
    fn push(&self, frames: &[f32]) {
        MovieAudio::push(self, frames);
    }

    fn finish(&self) {
        MovieAudio::finish(self);
    }

    fn queued_frames(&self) -> usize {
        MovieAudio::queued_frames(self)
    }

    fn stopped(&self) -> bool {
        MovieAudio::stopped(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_waits_for_mixer_and_holds_during_packet_underrun() {
        let audio = MovieAudio::new(2, 48_000);
        assert_eq!(audio.played_seconds(), None);
        audio.push(&vec![0.25; 128 * 2]);
        let mut out = vec![vec![1.0; 256]; 2];
        assert!(audio.fill(1.0, &mut out));
        assert_eq!(audio.played_seconds(), Some(0.0));
        assert_eq!(audio.queued_frames(), 128);
        assert!(out.iter().flatten().all(|&x| x == 0.0));
        audio.push(&vec![0.25; 129 * 2]);
        assert!(audio.fill(1.0, &mut out));
        assert_eq!(audio.played_seconds(), Some(256.0 / 48_000.0));
        assert_eq!(audio.queued_frames(), 1);
        assert!(audio.fill(1.0, &mut out));
        assert_eq!(audio.played_seconds(), Some(256.0 / 48_000.0));
    }

    #[test]
    fn clock_counts_source_frames_not_pitched_output_frames() {
        let audio = MovieAudio::new(1, 48_000);
        audio.push(&vec![0.25; 513]);
        let mut out = vec![vec![0.0; 256]];
        assert!(audio.fill(2.0, &mut out));
        assert_eq!(audio.played_seconds(), Some(512.0 / 48_000.0));
        assert_eq!(audio.queued_frames(), 1);
    }

    #[test]
    fn final_packet_does_not_count_padding_and_stop_holds_clock() {
        let audio = MovieAudio::new(1, 48_000);
        audio.push(&[0.25; 20]);
        audio.finish();
        let mut out = vec![vec![0.0; 256]];
        assert!(!audio.fill(1.0, &mut out));
        assert_eq!(audio.played_seconds(), Some(20.0 / 48_000.0));
        assert!(audio.is_over());
        audio.stop();
        assert!(!audio.fill(1.0, &mut out));
        assert_eq!(audio.played_seconds(), Some(20.0 / 48_000.0));
    }
}
