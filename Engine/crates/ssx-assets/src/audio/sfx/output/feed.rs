use std::path::Path;
use std::sync::atomic::Ordering::Relaxed;
use std::time::{Duration, Instant};

use super::capture::{Capture, Record};
use super::resample::Resampler;
use super::{Output, RATE, stats};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceFormat {
    pub rate: u32,
    pub channels: usize,
    pub buffer: usize,
    pub period: usize,
    pub surround: Option<[usize; super::DAC_CHANNELS]>,
}

pub const IN_ORDER: [usize; super::DAC_CHANNELS] = [0, 1, 2, 3, 4, 5];
pub const ALSA_ORDER: [usize; super::DAC_CHANNELS] = [0, 1, 4, 5, 2, 3];

const WAIT_UNKNOWN: Duration = Duration::from_millis(40);

pub struct DeviceFeed {
    out: Output,
    format: DeviceFormat,
    resampler: Option<Resampler>,
    lead: usize,
    mix: Vec<f32>,
    stereo: Vec<f32>,
    capture: Option<Capture>,
    started: Option<Instant>,
    last_end: Option<Instant>,
    last_render: (u64, u64),
    profile: Option<Vec<usize>>,
    infer: bool,
}

impl DeviceFeed {
    pub fn new(out: &Output, mut format: DeviceFormat, capture: Option<&Path>) -> Self {
        if format.channels < super::DAC_CHANNELS {
            format.surround = None;
        }
        let lead = if format.surround.is_some() { super::DAC_CHANNELS } else { 2 };
        out.set_channels(lead);
        let resampler = (format.rate != RATE).then(|| Resampler::with_channels(RATE, format.rate, lead));
        if format.period > 0 {
            out.expect_pull((format.period as u64 * u64::from(RATE)).div_ceil(u64::from(format.rate.max(1))) as usize);
        }
        let capture = capture.and_then(|p| {
            Capture::create(p, format.rate, format.channels)
                .inspect_err(|e| ssx_core::eprintln!("warning: audio capture {}: {e}", p.display()))
                .ok()
        });
        let profile = std::env::var("SSX_SFX_PROFILE").is_ok_and(|v| v == "1").then(Vec::new);
        let feed = Self {
            out: out.clone(),
            format,
            resampler,
            lead,
            mix: Vec::new(),
            stereo: Vec::new(),
            capture,
            started: None,
            last_end: None,
            last_render: (stats::RENDERED.load(Relaxed), stats::RENDERED_NS.load(Relaxed)),
            profile,
            infer: false,
        };
        feed.publish_buffer();
        feed
    }

    pub fn format(&self) -> DeviceFormat {
        self.format
    }

    pub fn set_buffer(&mut self, frames: usize, infer: bool) {
        self.format.buffer = frames;
        self.infer = infer;
        self.publish_buffer();
    }

    fn publish_buffer(&self) {
        let frames = self.format.buffer as u64 * u64::from(RATE) / u64::from(self.format.rate.max(1));
        stats::DEVICE.store(frames, Relaxed);
    }

    pub fn fill(&mut self, data: &mut [f32]) {
        let begin = Instant::now();
        let started = *self.started.get_or_insert(begin);
        let ch = self.format.channels.max(1);
        let frames = data.len() / ch;
        let rate = f64::from(self.format.rate.max(1));
        let gap = self.last_end.map_or(Duration::ZERO, |e| begin.duration_since(e));
        let known = self.format.buffer > 0;
        if known && self.infer && gap > Duration::from_secs_f64(1.25 * self.format.buffer as f64 / rate) {
            stats::STARVED.fetch_add(1, Relaxed);
        }
        let held = Duration::from_secs_f64(self.format.buffer.saturating_sub(frames) as f64 / rate);
        let wait = if known { held.clamp(Duration::from_millis(10), Duration::from_millis(100)) } else { WAIT_UNKNOWN };
        let need = self.resampler.as_ref().map_or(frames, |r| r.need(frames));
        let lead = self.lead;
        self.mix.resize(lead * need, 0.0);
        let pull = self.out.pull(&mut self.mix, wait);
        if known && self.infer && pull.wait > held {
            stats::STARVED.fetch_add(1, Relaxed);
        }
        let stereo: &[f32] = match self.resampler.as_mut() {
            Some(r) => {
                self.stereo.resize(lead * frames, 0.0);
                r.process(&self.mix, &mut self.stereo);
                &self.stereo
            }
            None => &self.mix,
        };
        match (ch, self.format.surround) {
            (_, Some(map)) => data.chunks_exact_mut(ch).zip(stereo.chunks_exact(lead)).for_each(|(o, s)| {
                o.fill(0.0);
                for (&to, x) in map.iter().zip(s) {
                    o[to] = *x;
                }
            }),
            (1, None) => data.iter_mut().zip(stereo.as_chunks::<2>().0).for_each(|(o, s)| *o = 0.5 * (s[0] + s[1])),
            (2, None) => data[..2 * frames].copy_from_slice(stereo),
            (_, None) => data.chunks_exact_mut(ch).zip(stereo.as_chunks::<2>().0).for_each(|(o, s)| {
                o.fill(0.0);
                (o[0], o[1]) = (s[0], s[1]);
            }),
        }
        data[frames * ch..].fill(0.0);
        let end = Instant::now();
        if let Some(c) = &self.capture {
            let now = (stats::RENDERED.load(Relaxed), stats::RENDERED_NS.load(Relaxed));
            let rec = Record {
                at: begin.duration_since(started),
                frames,
                pulled: need,
                pull,
                took: end.duration_since(begin),
                gap,
                rendered: now.0 - self.last_render.0,
                render: Duration::from_nanos(now.1 - self.last_render.1),
                underruns: stats::UNDERRUNS.load(Relaxed),
                starved: stats::STARVED.load(Relaxed),
            };
            self.last_render = now;
            c.push(data, rec);
        }
        if let Some(v) = self.profile.as_mut() {
            v.push(frames);
            if v.len() == 400 {
                v.sort_unstable();
                let q = |f: f32| v[((v.len() - 1) as f32 * f) as usize];
                ssx_core::eprintln!(
                    "audio pulls (frames): min {} p10 {} p50 {} p90 {} max {}",
                    v[0],
                    q(0.1),
                    q(0.5),
                    q(0.9),
                    q(1.0)
                );
                v.clear();
            }
        }
        self.last_end = Some(end);
    }
}
