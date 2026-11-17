use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Mutex, TryLockError};
use std::time::{Duration, Instant};

use super::{Pull, RATE};

pub static RENDER_NS: AtomicU64 = AtomicU64::new(0);
pub static RENDER_MAX_NS: AtomicU64 = AtomicU64::new(0);
pub static DRAIN_NS: AtomicU64 = AtomicU64::new(0);
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
pub static POSTED: AtomicU64 = AtomicU64::new(0);
pub static APPLIED: AtomicU64 = AtomicU64::new(0);
pub static UNDERRUNS: AtomicU64 = AtomicU64::new(0);
pub static STARVED: AtomicU64 = AtomicU64::new(0);
pub static ERRORS: AtomicU64 = AtomicU64::new(0);
pub static DEMOTED: AtomicBool = AtomicBool::new(false);

pub fn render(render_ns: u64, frames: u64) {
    RENDER_NS.fetch_add(render_ns, Relaxed);
    RENDER_MAX_NS.fetch_max(render_ns, Relaxed);
    FRAMES.fetch_add(frames, Relaxed);
    RENDERED.fetch_add(frames, Relaxed);
    RENDERED_NS.fetch_add(render_ns, Relaxed);
    FRAME_MAX_NS.fetch_max(render_ns, Relaxed);
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ThreadUsage {
    pub user_us: u64,
    pub system_us: u64,
    pub voluntary: u64,
    pub involuntary: u64,
    pub major_faults: u64,
}

impl ThreadUsage {
    #[cfg(target_os = "linux")]
    pub fn now() -> Option<Self> {
        let usage = unsafe {
            let mut usage: libc::rusage = std::mem::zeroed();
            (libc::getrusage(libc::RUSAGE_THREAD, &mut usage) == 0).then_some(usage)
        }?;
        let us = |t: libc::timeval| {
            u64::try_from(t.tv_sec).unwrap_or(0).saturating_mul(1_000_000) + u64::try_from(t.tv_usec).unwrap_or(0)
        };
        let n = |x: libc::c_long| u64::try_from(x).unwrap_or(0);
        Some(Self {
            user_us: us(usage.ru_utime),
            system_us: us(usage.ru_stime),
            voluntary: n(usage.ru_nvcsw),
            involuntary: n(usage.ru_nivcsw),
            major_faults: n(usage.ru_majflt),
        })
    }

    #[cfg(not(target_os = "linux"))]
    pub fn now() -> Option<Self> {
        None
    }

    pub fn since(self, before: Self) -> Self {
        Self {
            user_us: self.user_us.saturating_sub(before.user_us),
            system_us: self.system_us.saturating_sub(before.system_us),
            voluntary: self.voluntary.saturating_sub(before.voluntary),
            involuntary: self.involuntary.saturating_sub(before.involuntary),
            major_faults: self.major_faults.saturating_sub(before.major_faults),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrainRecord {
    pub ns: u64,
    pub commands: u64,
    pub kind: &'static str,
    pub kind_ns: u64,
    pub thread: Option<ThreadUsage>,
}

impl std::fmt::Display for DrainRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ms = |ns: u64| ns as f64 * 1e-6;
        let mut kind = self.kind;
        while let Some(outer) = kind.strip_suffix("::{{closure}}") {
            kind = outer;
        }
        write!(f, "{:.1} ms, {} commands, the longest {kind} {:.1} ms", ms(self.ns), self.commands, ms(self.kind_ns))?;
        if let Some(t) = self.thread {
            write!(
                f,
                "; the thread on a CPU {:.1} ms (user {:.1}, system {:.1}), {} blocked / {} preempted, {} major faults",
                (t.user_us + t.system_us) as f64 * 1e-3,
                t.user_us as f64 * 1e-3,
                t.system_us as f64 * 1e-3,
                t.voluntary,
                t.involuntary,
                t.major_faults
            )?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrainReader {
    Report = 0,
    Perf = 1,
}

static LONGEST_DRAIN: Mutex<[Option<DrainRecord>; 2]> = Mutex::new([None, None]);

pub fn take_longest_drain(reader: DrainReader) -> Option<DrainRecord> {
    let mut slots = LONGEST_DRAIN.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    slots[reader as usize].take()
}

#[derive(Debug, Default)]
pub struct DrainLog {
    staged: Option<DrainRecord>,
}

impl DrainLog {
    pub fn record(&mut self, r: DrainRecord) {
        APPLIED.fetch_add(r.commands, Relaxed);
        DRAIN_NS.fetch_add(r.ns, Relaxed);
        if self.staged.is_none_or(|s| r.ns > s.ns) {
            self.staged = Some(r);
        }
        self.publish();
    }

    pub fn publish(&mut self) {
        let Some(r) = self.staged else { return };
        let mut slots = match LONGEST_DRAIN.try_lock() {
            Ok(slots) => slots,
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(TryLockError::WouldBlock) => return,
        };
        for slot in slots.iter_mut() {
            if slot.is_none_or(|s| r.ns > s.ns) {
                *slot = Some(r);
            }
        }
        self.staged = None;
    }
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
    pub stream_first: u64,
    pub stream_dry: u64,
    pub waited_first: u64,
    pub waited_dry: u64,
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
        stream_first: crate::sfx_stream::LATE_FIRST.load(Relaxed),
        stream_dry: crate::sfx_stream::LATE_DRY.load(Relaxed),
        waited_first: crate::sfx_stream::WAITED_FIRST.load(Relaxed),
        waited_dry: crate::sfx_stream::WAITED_DRY.load(Relaxed),
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
        let (frame_max, drain) = (FRAME_MAX_NS.swap(0, Relaxed), take_longest_drain(DrainReader::Report));
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
        let ms = |frames: u64| frames as f64 * 1000.0 / f64::from(RATE);
        let (dry, first) = (d(now.stream_dry, l.stream_dry), d(now.stream_first, l.stream_first));
        if dry > 0 {
            parts.push(format!(
                "{dry} times a running stream ran dry (its next block not read yet; {:.1} ms of its sound late)",
                ms(d(now.waited_dry, l.waited_dry))
            ));
        }
        if first > 0 {
            parts.push(format!(
                "{first} times a new stream waited for its first block ({:.1} ms late in all)",
                ms(d(now.waited_first, l.waited_first))
            ));
        }
        if let Some(w) = crate::sfx_stream::take_longest().filter(|w| w.frames > 0 || dry > 0 || first > 0) {
            parts.push(format!("the longest wait {w}"));
        }
        let late = d(now.late, l.late);
        if late > 0 {
            parts.push(format!("{late} pulls waited for the mixer (longest {:.1} ms)", wait_max as f64 * 1e-6));
        }
        if DEMOTED.swap(false, Relaxed) {
            parts.push(
                "the mixer thread ran past its real-time limit (RLIMIT_RTTIME) without blocking and runs at normal \
                 priority from here"
                    .to_owned(),
            );
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
        let drain = drain.map_or_else(|| "none".to_owned(), |d| d.to_string());
        let waiting = POSTED.load(Relaxed).saturating_sub(APPLIED.load(Relaxed));
        Some(format!(
            "audio: {} in the last {secs:.1} s (longest mixer frame {frame:.1} ms; longest drain of the game's \
             commands: {drain}; {waiting} commands queued)",
            parts.join(", ")
        ))
    }
}
