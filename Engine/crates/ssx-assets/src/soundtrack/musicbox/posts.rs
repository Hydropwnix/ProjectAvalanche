use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::{RaceEvent, Session};

pub const RACE_EVENT_PERIOD_MS: u64 = 500;

pub(super) fn ms_frames(ms: u64) -> u64 {
    ms * u64::from(crate::pathfinder::SAMPLE_RATE) / 1000
}

fn frames_duration(frames: u64) -> Duration {
    Duration::from_secs_f64(frames as f64 / f64::from(crate::pathfinder::SAMPLE_RATE))
}

fn duration_frames(d: Duration) -> u64 {
    (d.as_secs_f64() * f64::from(crate::pathfinder::SAMPLE_RATE)).round() as u64
}

#[derive(Debug, Default)]
pub struct RemixClock {
    armed: bool,
    next: Option<Instant>,
}

impl RemixClock {
    pub fn armed(&self) -> bool {
        self.armed
    }

    pub fn arm(&mut self, now: Instant) {
        self.armed = true;
        let first = crate::sfx_game::tuning::MUSIC_RACE_EVENT_DELAY;
        self.next = Some(now + Duration::from_secs_f64(f64::from(first)));
    }

    pub fn hand_over(&mut self, now: Instant, frames: u64) {
        if self.armed {
            self.next = Some(now + frames_duration(frames));
        }
    }

    pub fn take_over(&self, now: Instant) -> Option<u64> {
        if !self.armed {
            return None;
        }
        let next = self.next.unwrap_or(now);
        if next >= now {
            return Some(duration_frames(next - now));
        }
        let period = ms_frames(RACE_EVENT_PERIOD_MS);
        let late = duration_frames(now - next) % period;
        Some((period - late) % period)
    }
}

pub(super) fn first_menu_check(next: Option<u64>) -> Option<u64> {
    let update = (f64::from(crate::sfx::UPDATE_PERIOD) * f64::from(crate::pathfinder::SAMPLE_RATE)).round() as u64;
    next.map(|n| if n < update { n + ms_frames(RACE_EVENT_PERIOD_MS) } else { n })
}

static REMIX: OnceLock<Mutex<RemixClock>> = OnceLock::new();

pub fn remix() -> &'static Mutex<RemixClock> {
    REMIX.get_or_init(Default::default)
}

pub(super) struct RacePosts {
    pub event: RaceEvent,
    pub session: &'static Mutex<Session>,
    pub remix: &'static Mutex<RemixClock>,
    pub clock: u64,
    pub next: Option<u64>,
    pub held: bool,
}

impl RacePosts {
    pub fn new(event: RaceEvent, session: &'static Mutex<Session>, remix: &'static Mutex<RemixClock>) -> Self {
        Self { event, session, remix, clock: 0, next: None, held: false }
    }

    pub fn hand_over(&self) {
        if let (Some(next), Ok(mut r)) = (self.next, self.remix.lock()) {
            r.hand_over(Instant::now(), next.saturating_sub(self.clock));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_checks_start_with_a_ride_and_go_on_every_500_ms() {
        let t0 = Instant::now();
        let mut r = RemixClock::default();
        assert_eq!(r.take_over(t0), None, "the constructor's +0x20 = 0: no check before an event set-up");
        r.arm(t0);
        assert_eq!(r.take_over(t0), Some(44_100), "the first check `audio_tuning` +0x4C = 1 s after the set-up");
        let period = ms_frames(RACE_EVENT_PERIOD_MS);
        r.hand_over(t0, 1000);
        assert_eq!(r.take_over(t0), Some(1000));
        let later = t0 + frames_duration(1000 + 2 * period + 300);
        assert_eq!(r.take_over(later), Some(period - 300));
        let on_grid = t0 + frames_duration(1000 + 3 * period);
        assert_eq!(r.take_over(on_grid), Some(0));
    }

    #[test]
    fn the_leaves_update_holds_its_check() {
        assert_eq!(first_menu_check(None), None);
        assert_eq!(first_menu_check(Some(0)), Some(22_050));
        assert_eq!(first_menu_check(Some(734)), Some(734 + 22_050));
        assert_eq!(first_menu_check(Some(735)), Some(735));
    }

    #[test]
    fn a_hand_over_does_not_arm() {
        let mut r = RemixClock::default();
        r.hand_over(Instant::now(), 10);
        assert!(!r.armed() && r.take_over(Instant::now()).is_none());
    }
}
