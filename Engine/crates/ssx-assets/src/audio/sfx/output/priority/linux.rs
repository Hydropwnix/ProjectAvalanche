use std::sync::Arc;
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering::Relaxed};
use std::time::{Duration, Instant};

use super::dbus::{Arg, Bus};
use crate::sfx::output::stats;

const RTKIT: &str = "org.freedesktop.RealtimeKit1";
const RTKIT_PATH: &str = "/org/freedesktop/RealtimeKit1";

// PORT(choice): SCHED_RR priority 10 — real-time priorities only order real-time threads among themselves (each runs
// before every normal thread); on the development machine the sound server's data loops run at FIFO 83 / 88 (above),
// the compositor's threads at RR 1 (below), and rtkit grants up to 20. The console runs its Dac thread at EA::Thread
// priority 15 (THREAD_PRIORITY_TIME_CRITICAL, 0x82A49A58) on a hardware thread of its own; Linux has neither.
const PRIORITY: u32 = 10;
// PORT(choice): nice −11, the level the desktop's sound servers ask rtkit for their own threads beside their real-time
// loops (`/usr/share/pipewire/pipewire.conf` and `pipewire-pulse.conf`, module-rt `nice.level = -11`); rtkit grants down
// to its MinNiceLevel (−15 on the development machine). The console runs "rwfilesys" at EA::Thread priority 2
// (THREAD_PRIORITY_HIGHEST: ahead of every thread of a lower priority, behind the Dac's 15), and Linux's normal
// scheduler has no strict priorities, only weights by nice level; the mixer thread's SCHED_RR stays above it.
const FILE_THREAD_NICE: i32 = -11;
const TIMEOUT: Duration = Duration::from_secs(5);
const REST: Duration = Duration::from_micros(100);

static PROMOTED: AtomicI32 = AtomicI32::new(0);

pub(super) struct Grant {
    tid: i32,
    slice_ns: Arc<AtomicU64>,
    awake: Instant,
}

impl Grant {
    pub(super) fn blocked(&mut self) {
        self.awake = Instant::now();
    }

    pub(super) fn check(&mut self) {
        let slice = self.slice_ns.load(Relaxed);
        if slice > 0 && self.awake.elapsed() >= Duration::from_nanos(slice) {
            std::thread::sleep(REST);
            self.awake = Instant::now();
        }
    }
}

impl Drop for Grant {
    fn drop(&mut self) {
        let _ = PROMOTED.compare_exchange(self.tid, 0, Relaxed, Relaxed);
    }
}

pub(super) fn promote() -> Option<Grant> {
    let tid = unsafe { libc::gettid() };
    if PROMOTED.compare_exchange(0, tid, Relaxed, Relaxed).is_err() {
        ssx_core::eprintln!(
            "audio: the mixer thread keeps its normal priority (another mixer thread of this process asked)"
        );
        return None;
    }
    let slice_ns = Arc::new(AtomicU64::new(0));
    let theirs = slice_ns.clone();
    let spawned = std::thread::Builder::new().name("ssx_audio_rt".into()).spawn(move || match request(tid, &theirs) {
        Ok(granted) => ssx_core::eprintln!("audio: the mixer thread runs real-time ({granted})"),
        Err(why) => ssx_core::eprintln!("audio: the mixer thread keeps its normal priority ({why})"),
    });
    if let Err(e) = spawned {
        PROMOTED.store(0, Relaxed);
        ssx_core::eprintln!("audio: the mixer thread keeps its normal priority (no thread for the request: {e})");
        return None;
    }
    Some(Grant { tid, slice_ns, awake: Instant::now() })
}

pub(super) fn raise_file_thread() {
    let tid = unsafe { libc::gettid() };
    let normal = libc::sched_param { sched_priority: 0 };
    if unsafe { libc::sched_setscheduler(0, libc::SCHED_OTHER | libc::SCHED_RESET_ON_FORK, &normal) } != 0 {
        let e = std::io::Error::last_os_error();
        ssx_core::eprintln!("audio: the stream file thread keeps its normal priority (SCHED_RESET_ON_FORK: {e})");
        return;
    }
    let spawned = std::thread::Builder::new().name("ssx_stream_io_nice".into()).spawn(move || match nice(tid) {
        Ok(level) => ssx_core::eprintln!(
            "audio: the stream file thread runs at nice {level} (rtkit; the console's EA::Thread priority 2)"
        ),
        Err(why) => ssx_core::eprintln!("audio: the stream file thread keeps its normal priority ({why})"),
    });
    if let Err(e) = spawned {
        ssx_core::eprintln!("audio: the stream file thread keeps its normal priority (no thread for the request: {e})");
    }
}

fn nice(tid: i32) -> Result<i32, String> {
    let mut bus = Bus::system(TIMEOUT).map_err(|e| format!("the system bus: {e}"))?;
    let level = nice_level(property(&mut bus, "MinNiceLevel")?)?;
    bus.call(RTKIT, RTKIT_PATH, RTKIT, "MakeThreadHighPriority", &[Arg::U64(tid as u64), Arg::I32(level)])
        .map_err(|e| format!("rtkit refused: {e}"))?;
    let got = unsafe {
        *libc::__errno_location() = 0;
        let got = libc::getpriority(libc::PRIO_PROCESS, tid as libc::id_t);
        (*libc::__errno_location() == 0).then_some(got)
    };
    match got {
        Some(got) if got == level => Ok(level),
        Some(got) => Err(format!("rtkit answered, but the thread's nice level is {got}")),
        None => Err(format!("rtkit answered; the thread's nice level: {}", std::io::Error::last_os_error())),
    }
}

fn nice_level(min: i64) -> Result<i32, String> {
    let level = i64::from(FILE_THREAD_NICE).max(min);
    if level >= 0 {
        return Err(format!("rtkit grants no nice level below 0 (MinNiceLevel {min})"));
    }
    i32::try_from(level).map_err(|_| format!("rtkit's MinNiceLevel {min}"))
}

fn property(bus: &mut Bus, name: &str) -> Result<i64, String> {
    bus.call(RTKIT, RTKIT_PATH, "org.freedesktop.DBus.Properties", "Get", &[Arg::Str(RTKIT), Arg::Str(name)])
        .and_then(|r| r.variant_int())
        .map_err(|e| format!("rtkit's {name}: {e}"))
}

struct Granted {
    priority: i32,
    max: i64,
    soft_us: u64,
    hard_us: u64,
    handler: bool,
}

impl std::fmt::Display for Granted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "rtkit: SCHED_RR priority {} of rtkit's {}; RLIMIT_RTTIME {} / {} ms{}",
            self.priority,
            self.max,
            self.soft_us / 1000,
            self.hard_us / 1000,
            if self.handler { "" } else { "; SIGXCPU has another handler" }
        )
    }
}

fn request(tid: i32, slice_ns: &AtomicU64) -> Result<Granted, String> {
    let mut bus = Bus::system(TIMEOUT).map_err(|e| format!("the system bus: {e}"))?;
    let max = property(&mut bus, "MaxRealtimePriority")?;
    let rttime = property(&mut bus, "RTTimeUSecMax")?;
    if max < 1 {
        return Err(format!("rtkit grants no real-time priority (MaxRealtimePriority {max})"));
    }
    if rttime < 50_000 {
        return Err(format!("rtkit's RTTimeUSecMax {rttime} us is too short for the mixer"));
    }
    let priority = PRIORITY.min(u32::try_from(max).unwrap_or(u32::MAX));
    let old = rttime_limit().map_err(|e| format!("RLIMIT_RTTIME: {e}"))?;
    let hard = old.rlim_max.min(rttime as u64);
    let soft = old.rlim_cur.min(hard / 2);
    set_rttime_limit(soft, hard).map_err(|e| format!("RLIMIT_RTTIME {soft} / {hard} us: {e}"))?;
    let handler = install_handler();
    slice_ns.store(soft.saturating_mul(1000) / 4, Relaxed);
    let made = bus.call(RTKIT, RTKIT_PATH, RTKIT, "MakeThreadRealtime", &[Arg::U64(tid as u64), Arg::U32(priority)]);
    if let Err(e) = made {
        slice_ns.store(0, Relaxed);
        if handler {
            remove_handler();
        }
        let kept = match set_rttime_limit(old.rlim_cur, old.rlim_max) {
            Ok(()) => String::new(),
            Err(_) => format!("; RLIMIT_RTTIME stays {} / {} ms", soft / 1000, hard / 1000),
        };
        return Err(format!("rtkit refused: {e}{kept}"));
    }
    let (policy, param) = unsafe {
        let mut param = libc::sched_param { sched_priority: 0 };
        (libc::sched_getscheduler(tid), (libc::sched_getparam(tid, &mut param) == 0).then_some(param.sched_priority))
    };
    if policy & !libc::SCHED_RESET_ON_FORK != libc::SCHED_RR {
        slice_ns.store(0, Relaxed);
        return Err(format!("rtkit answered, but the thread's policy is {policy}"));
    }
    Ok(Granted { priority: param.unwrap_or(-1), max, soft_us: soft, hard_us: hard, handler })
}

fn rttime_limit() -> std::io::Result<libc::rlimit> {
    let mut limit = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
    if unsafe { libc::getrlimit(libc::RLIMIT_RTTIME, &mut limit) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(limit)
}

fn set_rttime_limit(soft: u64, hard: u64) -> std::io::Result<()> {
    let limit = libc::rlimit { rlim_cur: soft, rlim_max: hard };
    if unsafe { libc::setrlimit(libc::RLIMIT_RTTIME, &limit) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

extern "C" fn on_sigxcpu(_: libc::c_int) {
    let errno = unsafe { *libc::__errno_location() };
    let tid = PROMOTED.load(Relaxed);
    if tid > 0 {
        let normal = libc::sched_param { sched_priority: 0 };
        if unsafe { libc::sched_setscheduler(tid, libc::SCHED_OTHER | libc::SCHED_RESET_ON_FORK, &normal) } == 0 {
            stats::DEMOTED.store(true, Relaxed);
        }
    }
    unsafe { *libc::__errno_location() = errno };
}

fn install_handler() -> bool {
    unsafe {
        let mut current: libc::sigaction = std::mem::zeroed();
        if libc::sigaction(libc::SIGXCPU, std::ptr::null(), &mut current) != 0 || current.sa_sigaction != libc::SIG_DFL
        {
            return false;
        }
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = on_sigxcpu as extern "C" fn(libc::c_int) as libc::sighandler_t;
        action.sa_flags = libc::SA_RESTART;
        libc::sigemptyset(&mut action.sa_mask);
        libc::sigaction(libc::SIGXCPU, &action, std::ptr::null_mut()) == 0
    }
}

fn remove_handler() {
    let failed = unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = libc::SIG_DFL;
        libc::sigemptyset(&mut action.sa_mask);
        libc::sigaction(libc::SIGXCPU, &action, std::ptr::null_mut()) != 0
    };
    if failed {
        ssx_core::eprintln!("audio: SIGXCPU's handler stays: {}", std::io::Error::last_os_error());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_threads_level_stays_within_rtkits() {
        assert_eq!(nice_level(-15), Ok(FILE_THREAD_NICE));
        assert_eq!(nice_level(-20), Ok(FILE_THREAD_NICE));
        assert_eq!(nice_level(-5), Ok(-5));
        assert!(nice_level(0).is_err());
        assert!(nice_level(i64::MAX).is_err());
    }
}
