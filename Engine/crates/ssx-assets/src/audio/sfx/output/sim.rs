use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::feed::{DeviceFeed, DeviceFormat};
use super::stats;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Alsa,
    Wasapi,
    Null,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SimSpec {
    pub kind: Kind,
    pub rate: u32,
    pub channels: usize,
    pub period: usize,
    pub buffer: usize,
    pub jitter: Duration,
}

impl SimSpec {
    pub fn parse(s: &str) -> Option<Self> {
        let mut it = s.split(':');
        let kind = match it.next()? {
            "null" => return Some(Self::null()),
            "alsa" => Kind::Alsa,
            "wasapi" => Kind::Wasapi,
            _ => return None,
        };
        let rate: u32 = it.next()?.parse().ok().filter(|r| *r >= 8000)?;
        let period: usize = it.next()?.parse().ok().filter(|p| *p > 0)?;
        let default = if kind == Kind::Alsa { 2 * period } else { period.max(2048) };
        let buffer = it.next().map_or(Some(default), |b| b.parse().ok())?.max(period);
        let jitter = it.next().map_or(Some(1.0), |j| j.parse::<f64>().ok())?.clamp(0.0, 1000.0);
        let channels = it.next().map_or(Some(2), |c| c.parse::<usize>().ok())?.clamp(1, 8);
        Some(Self { kind, rate, channels, period, buffer, jitter: Duration::from_secs_f64(jitter * 1e-3) })
    }

    pub fn null() -> Self {
        Self { kind: Kind::Null, rate: super::RATE, channels: 2, period: 480, buffer: 0, jitter: Duration::ZERO }
    }

    pub fn format(&self) -> DeviceFormat {
        let surround = match self.kind {
            Kind::Alsa => super::feed::ALSA_ORDER,
            _ => super::feed::IN_ORDER,
        };
        let surround = (self.channels >= super::DAC_CHANNELS).then_some(surround);
        DeviceFormat { rate: self.rate, channels: self.channels, buffer: self.buffer, period: self.period, surround }
    }

    pub fn label(&self) -> String {
        let kind = match self.kind {
            Kind::Alsa => "alsa",
            Kind::Wasapi => "wasapi",
            Kind::Null => return "null".into(),
        };
        let channels = if self.channels == 2 { String::new() } else { format!(":{}", self.channels) };
        format!("{kind}:{}:{}:{}:{}{channels}", self.rate, self.period, self.buffer, self.jitter.as_secs_f64() * 1e3)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SimReport {
    pub callbacks: u64,
    pub frames: u64,
    pub underruns: u64,
    pub longest_callback: Duration,
    pub smallest_pull: usize,
    pub largest_pull: usize,
}

pub struct SimDevice {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<SimReport>>,
}

impl SimDevice {
    pub fn start(spec: SimSpec, feed: DeviceFeed) -> std::io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = std::thread::Builder::new().name("ssx_audio_sim".into()).spawn(move || run(spec, feed, &flag))?;
        Ok(Self { stop, thread: Some(thread) })
    }

    pub fn stop(mut self) -> SimReport {
        self.stop.store(true, Relaxed);
        self.thread.take().and_then(|t| t.join().ok()).unwrap_or_default()
    }
}

impl Drop for SimDevice {
    fn drop(&mut self) {
        self.stop.store(true, Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct Model {
    spec: SimSpec,
    feed: DeviceFeed,
    data: Vec<f32>,
    report: SimReport,
    rng: u64,
    queued: f64,
    playing: bool,
}

impl Model {
    fn callback(&mut self, frames: usize) {
        self.data.resize(frames * self.spec.channels, 0.0);
        let t = Instant::now();
        self.feed.fill(&mut self.data);
        let r = &mut self.report;
        r.longest_callback = r.longest_callback.max(t.elapsed());
        r.smallest_pull = if r.callbacks == 0 { frames } else { r.smallest_pull.min(frames) };
        r.largest_pull = r.largest_pull.max(frames);
        r.callbacks += 1;
        r.frames += frames as u64;
    }

    fn underrun(&mut self) {
        self.report.underruns += 1;
        stats::UNDERRUNS.fetch_add(1, Relaxed);
    }

    fn late(&mut self) -> Duration {
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        let x = self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D);
        let unit = (x >> 11) as f64 / (1u64 << 53) as f64;
        self.spec.jitter.mul_f64(unit * if x & 0x7F == 0 { 4.0 } else { 1.0 })
    }

    fn alsa(&mut self, stop: &AtomicBool) {
        let rate = f64::from(self.spec.rate);
        let (period, buffer) = (self.spec.period as f64, self.spec.buffer as f64);
        let threshold = (2.0 * period).min(buffer);
        let mut at = Instant::now();
        let play = |m: &mut Self, at: &mut Instant| {
            let now = Instant::now();
            if m.playing {
                m.queued -= rate * now.duration_since(*at).as_secs_f64();
                if m.queued < 0.0 {
                    m.underrun();
                    (m.playing, m.queued) = (false, 0.0);
                }
            }
            *at = now;
        };
        while !stop.load(Relaxed) {
            play(self, &mut at);
            if buffer - self.queued >= period {
                self.callback(self.spec.period);
                play(self, &mut at);
                self.queued += period;
                self.playing |= self.queued >= threshold;
            } else {
                let free_in = Duration::from_secs_f64((self.queued - (buffer - period)) / rate);
                std::thread::sleep(free_in + self.late());
            }
        }
    }

    fn wasapi(&mut self, stop: &AtomicBool) {
        let (tick, buffer) = (self.spec.period as f64, self.spec.buffer as f64);
        let dt = Duration::from_secs_f64(tick / f64::from(self.spec.rate));
        let mut next = Instant::now();
        let mut fed = false;
        let ticks = |m: &mut Self, next: &mut Instant, fed: bool| {
            let now = Instant::now();
            while *next <= now {
                if fed && m.queued >= tick {
                    m.queued -= tick;
                } else if fed {
                    m.underrun();
                    m.queued = 0.0;
                }
                *next += dt;
            }
        };
        while !stop.load(Relaxed) {
            let wake = next + self.late();
            std::thread::sleep(wake.saturating_duration_since(Instant::now()));
            ticks(self, &mut next, fed);
            let free = (buffer - self.queued).floor() as usize;
            if free > 0 {
                self.callback(free);
                ticks(self, &mut next, fed);
                self.queued += free as f64;
                fed = true;
            }
        }
    }

    fn null(&mut self, stop: &AtomicBool) {
        let frames = (self.spec.rate / 100) as usize;
        let mut next = Instant::now();
        while !stop.load(Relaxed) {
            self.callback(frames);
            next += Duration::from_millis(10);
            std::thread::sleep(next.saturating_duration_since(Instant::now()));
        }
    }
}

fn run(spec: SimSpec, feed: DeviceFeed, stop: &AtomicBool) -> SimReport {
    let rng = 0x9E37_79B9_7F4A_7C15 ^ (spec.period as u64) << 20 ^ u64::from(spec.rate);
    let mut m = Model { spec, feed, data: Vec::new(), report: SimReport::default(), rng, queued: 0.0, playing: false };
    match spec.kind {
        Kind::Alsa => m.alsa(stop),
        Kind::Wasapi => m.wasapi(stop),
        Kind::Null => m.null(stop),
    }
    m.report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specs_parse() {
        let s = SimSpec::parse("alsa:44100:2048").unwrap();
        assert_eq!((s.kind, s.rate, s.period, s.buffer), (Kind::Alsa, 44_100, 2048, 4096));
        let s = SimSpec::parse("wasapi:48000:480:1056:2.5").unwrap();
        assert_eq!((s.kind, s.period, s.buffer, s.jitter), (Kind::Wasapi, 480, 1056, Duration::from_micros(2500)));
        assert_eq!(SimSpec::parse("wasapi:48000:480").unwrap().buffer, 2048);
        assert_eq!(SimSpec::parse("null").unwrap().kind, Kind::Null);
        assert!(SimSpec::parse("alsa:48000").is_none() && SimSpec::parse("pulse:48000:480").is_none());
        assert_eq!(SimSpec::parse(&s.label()), Some(s));
    }
}
