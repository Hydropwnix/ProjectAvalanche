use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::{Duration, Instant};

use super::{Pull, RATE};

pub static RENDER_NS: AtomicU64 = AtomicU64::new(0);
pub static RENDER_MAX_NS: AtomicU64 = AtomicU64::new(0);
pub static LOCK_NS: AtomicU64 = AtomicU64::new(0);
pub static FRAMES: AtomicU64 = AtomicU64::new(0);
pub static ON_DEMAND: AtomicU64 = AtomicU64::new(0);
pub static SILENCE: AtomicU64 = AtomicU64::new(0);

pub static LEAD: AtomicU64 = AtomicU64::new(0);

pub static QUEUED: AtomicU64 = AtomicU64::new(0);
pub static DEVICE: AtomicU64 = AtomicU64::new(0);

pub static PULLS: AtomicU64 = AtomicU64::new(0);
pub static PULLED: AtomicU64 = AtomicU64::new(0);
pub static SILENT: AtomicU64 = AtomicU64::new(0);
pub static STARTUP: AtomicU64 = AtomicU64::new(0);
pub static LATE: AtomicU64 = AtomicU64::new(0);
pub static WAIT_NS: AtomicU64 = AtomicU64::new(0);
pub static WAIT_MAX_NS: AtomicU64 = AtomicU64::new(0);
pub static RENDERED: AtomicU64 = AtomicU64::new(0);
pub static RENDERED_NS: AtomicU64 = AtomicU64::new(0);
pub static FRAME_MAX_NS: AtomicU64 = AtomicU64::new(0);
pub static UNDERRUNS: AtomicU64 = AtomicU64::new(0);
pub static STARVED: AtomicU64 = AtomicU64::new(0);
pub static ERRORS: AtomicU64 = AtomicU64::new(0);

pub fn render(lock_ns: u64, render_ns: u64, frames: u64) {
    LOCK_NS.fetch_add(lock_ns, Relaxed);
    RENDER_NS.fetch_add(render_ns, Relaxed);
    RENDER_MAX_NS.fetch_max(render_ns, Relaxed);
    FRAMES.fetch_add(frames, Relaxed);
    RENDERED.fetch_add(frames, Relaxed);
    RENDERED_NS.fetch_add(render_ns, Relaxed);
    FRAME_MAX_NS.fetch_max(render_ns, Relaxed);
}

pub fn pull(p: &Pull, frames: usize) {
    PULLS.fetch_add(1, Relaxed);
    PULLED.fetch_add(frames as u64, Relaxed);
    if p.startup {
        STARTUP.fetch_add(frames as u64, Relaxed);
        return;
    }
    SILENCE.fetch_add(p.silence as u64, Relaxed);
    SILENT.fetch_add(p.silence as u64, Relaxed);
    if !p.wait.is_zero() {
        let ns = p.wait.as_nanos() as u64;
        LATE.fetch_add(1, Relaxed);
        WAIT_NS.fetch_add(ns, Relaxed);
        WAIT_MAX_NS.fetch_max(ns, Relaxed);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub pulls: u64,
    pub pulled: u64,
    pub silent: u64,
    pub startup: u64,
    pub late: u64,
    pub wait_ns: u64,
    pub rendered: u64,
    pub rendered_ns: u64,
    pub underruns: u64,
    pub starved: u64,
    pub errors: u64,
    pub lead: u64,
}

pub fn snapshot() -> Snapshot {
    Snapshot {
        pulls: PULLS.load(Relaxed),
        pulled: PULLED.load(Relaxed),
        silent: SILENT.load(Relaxed),
        startup: STARTUP.load(Relaxed),
        late: LATE.load(Relaxed),
        wait_ns: WAIT_NS.load(Relaxed),
        rendered: RENDERED.load(Relaxed),
        rendered_ns: RENDERED_NS.load(Relaxed),
        underruns: UNDERRUNS.load(Relaxed),
        starved: STARVED.load(Relaxed),
        errors: ERRORS.load(Relaxed),
        lead: LEAD.load(Relaxed),
    }
}

pub struct Reporter {
    last: Snapshot,
    at: Instant,
    slow: bool,
}

impl Default for Reporter {
    fn default() -> Self {
        Self { last: snapshot(), at: Instant::now(), slow: false }
    }
}

impl Reporter {
    pub fn poll(&mut self) -> Option<String> {
        let secs = self.at.elapsed().as_secs_f64();
        if secs < 1.0 {
            return None;
        }
        let now = snapshot();
        let d = |a: u64, b: u64| a.saturating_sub(b);
        let (l, wait_max) = (self.last, WAIT_MAX_NS.swap(0, Relaxed));
        let frame_max = FRAME_MAX_NS.swap(0, Relaxed);
        (self.last, self.at) = (now, Instant::now());
        let rendered = d(now.rendered, l.rendered);
        let load = d(now.rendered_ns, l.rendered_ns) as f64 * 1e-9 / (rendered as f64 / f64::from(RATE)).max(1e-9);
        let slow = rendered > 0 && load > 0.9;
        let mut parts = Vec::new();
        let mut count = |n: u64, what: &str| {
            if n > 0 {
                parts.push(format!("{n} {what}"));
            }
        };
        count(d(now.underruns, l.underruns), "underruns reported by the device");
        count(
            d(now.starved, l.starved),
            "probable underruns (a callback later or longer than the device's buffer lasts)",
        );
        count(d(now.silent, l.silent), "frames of silence inserted");
        count(d(now.errors, l.errors), "stream errors");
        let late = d(now.late, l.late);
        if late > 0 {
            parts.push(format!("{late} pulls waited for the mixer (longest {:.1} ms)", wait_max as f64 * 1e-6));
        }
        if now.lead != l.lead {
            parts.push(format!("the lead beyond a pull {} -> {} frames", l.lead, now.lead));
        }
        if slow && (!self.slow || !parts.is_empty()) {
            parts.push(format!("the mixer needs {:.0} % of real time", load * 100.0));
        }
        self.slow = slow;
        if parts.is_empty() {
            return None;
        }
        let frame = Duration::from_nanos(frame_max).as_secs_f64() * 1e3;
        Some(format!("audio: {} in the last {secs:.1} s (longest mixer frame {frame:.1} ms)", parts.join(", ")))
    }
}
