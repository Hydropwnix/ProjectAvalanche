use std::cell::Cell;
use std::time::Instant;

thread_local! {
    static WAITED: Cell<u64> = const { Cell::new(0) };
    static LAST: Cell<Option<(u64, Instant)>> = const { Cell::new(None) };
}

pub fn charged() -> u64 {
    let own = match os::thread_ns() {
        Some(t) => {
            LAST.with(|l| l.set(Some((t, Instant::now()))));
            t
        }
        None => LAST.with(|l| match l.get() {
            Some((t, at)) => t.saturating_add(u64::try_from(at.elapsed().as_nanos()).unwrap_or(u64::MAX)),
            None => 0,
        }),
    };
    own.saturating_add(WAITED.with(Cell::get))
}

pub fn add_waited(ns: u64) {
    WAITED.with(|w| w.set(w.get().saturating_add(ns)));
}

pub fn thread_ns() -> Option<u64> {
    os::thread_ns()
}

pub fn check() -> Result<(), String> {
    os::thread_ns().map(|_| ()).ok_or_else(|| format!("this thread's CPU clock cannot be read ({})", os::NAME))
}

pub fn describe() -> String {
    os::describe()
}

#[cfg(unix)]
mod os {
    pub const NAME: &str = "clock_gettime(CLOCK_THREAD_CPUTIME_ID)";

    pub fn thread_ns() -> Option<u64> {
        let mut t = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        let r = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) };
        if r != 0 {
            return None;
        }
        let secs = u64::try_from(t.tv_sec).ok()?;
        let nanos = u64::try_from(t.tv_nsec).ok()?;
        secs.checked_mul(1_000_000_000)?.checked_add(nanos)
    }

    pub fn describe() -> String {
        let mut r = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        let ok = unsafe { libc::clock_getres(libc::CLOCK_THREAD_CPUTIME_ID, &mut r) } == 0;
        if ok { format!("{NAME}, resolution {} ns", r.tv_nsec) } else { NAME.to_string() }
    }
}

#[cfg(all(windows, any(target_arch = "x86", target_arch = "x86_64")))]
mod os {
    use std::ffi::c_void;
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};

    pub const NAME: &str = "QueryThreadCycleTime";

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentThread() -> *mut c_void;
        fn QueryThreadCycleTime(thread: *mut c_void, cycles: *mut u64) -> i32;
    }

    #[cfg(target_arch = "x86_64")]
    fn tsc() -> u64 {
        unsafe { core::arch::x86_64::_rdtsc() }
    }

    #[cfg(target_arch = "x86")]
    fn tsc() -> u64 {
        unsafe { core::arch::x86::_rdtsc() }
    }

    fn cycles() -> Option<u64> {
        let mut c = 0u64;
        let ok = unsafe { QueryThreadCycleTime(GetCurrentThread(), &mut c) };
        (ok != 0).then_some(c)
    }

    fn rate() -> Option<u64> {
        static RATE: OnceLock<Option<u64>> = OnceLock::new();
        *RATE.get_or_init(|| {
            let (t0, c0) = (Instant::now(), tsc());
            std::thread::sleep(Duration::from_millis(10));
            let (c1, t1) = (tsc(), t0.elapsed());
            let ns = t1.as_nanos().max(1);
            let per_sec = u128::from(c1.checked_sub(c0)?) * 1_000_000_000 / ns;
            u64::try_from(per_sec).ok().filter(|r| *r > 0)
        })
    }

    pub fn thread_ns() -> Option<u64> {
        let c = cycles()?;
        let r = rate()?;
        u64::try_from(u128::from(c) * 1_000_000_000 / u128::from(r)).ok()
    }

    pub fn describe() -> String {
        match rate() {
            Some(r) => format!("{NAME}, time-stamp counter at {:.3} MHz (measured)", r as f64 / 1e6),
            None => format!("{NAME}, the time-stamp counter's rate could not be measured"),
        }
    }
}

#[cfg(all(windows, not(any(target_arch = "x86", target_arch = "x86_64"))))]
compile_error!(
    "the script host's CPU clock is implemented for x86 and x86-64 Windows (QueryThreadCycleTime counts time-stamp \
     counter cycles there; on other architectures it counts core cycles, which have no fixed rate)"
);

#[cfg(not(any(unix, windows)))]
compile_error!("the script host's CPU clock needs a per-thread CPU clock (Unix or Windows)");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_reads_advances_with_work_and_not_with_sleep() {
        check().unwrap();
        let a = charged();
        let t = Instant::now();
        let mut x = 0u64;
        while charged() - a < 20_000_000 && t.elapsed().as_secs() < 60 {
            for _ in 0..10_000 {
                x = std::hint::black_box(x.wrapping_add(1));
            }
        }
        let b = charged();
        assert!(b - a >= 20_000_000, "work charged {} ns in {:?}", b - a, t.elapsed());
        std::thread::sleep(std::time::Duration::from_millis(30));
        let c = charged();
        assert!(c - b < 5_000_000, "30 ms of sleep charged {} ns", c - b);
        add_waited(7);
        assert!(charged() >= c + 7);
    }
}
