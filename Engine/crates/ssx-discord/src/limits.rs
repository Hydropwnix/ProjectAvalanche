use std::collections::VecDeque;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub(crate) struct RateWindow {
    count: usize,
    window: Duration,
    sent: VecDeque<Instant>,
}

impl RateWindow {
    pub(crate) fn new(count: usize, window: Duration) -> Self {
        RateWindow { count: count.max(1), window, sent: VecDeque::with_capacity(count.max(1)) }
    }

    pub(crate) fn next_allowed(&mut self, now: Instant) -> Instant {
        while self.sent.front().is_some_and(|&t| now.saturating_duration_since(t) >= self.window) {
            self.sent.pop_front();
        }
        match self.sent.front() {
            Some(&oldest) if self.sent.len() >= self.count => oldest + self.window,
            _ => now,
        }
    }

    pub(crate) fn record(&mut self, now: Instant) {
        if self.sent.len() >= self.count {
            self.sent.pop_front();
        }
        self.sent.push_back(now);
    }
}

#[derive(Debug)]
pub(crate) struct Retry {
    first: Duration,
    max: Duration,
    delay: Duration,
    at: Instant,
}

impl Retry {
    pub(crate) fn new(first: Duration, max: Duration, now: Instant) -> Self {
        Retry { first, max: max.max(first), delay: first, at: now }
    }

    pub(crate) fn at(&self) -> Instant {
        self.at
    }

    pub(crate) fn failed(&mut self, now: Instant) -> Duration {
        let delay = self.delay;
        self.at = now + delay;
        self.delay = (self.delay * 2).min(self.max);
        delay
    }

    pub(crate) fn reset(&mut self) {
        self.delay = self.first;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn five_in_any_twenty_seconds() {
        let t0 = Instant::now();
        let s = Duration::from_secs;
        let mut r = RateWindow::new(5, s(20));
        for i in 0..5 {
            assert_eq!(r.next_allowed(t0 + s(i)), t0 + s(i));
            r.record(t0 + s(i));
        }
        assert_eq!(r.next_allowed(t0 + s(5)), t0 + s(20));
        assert_eq!(r.next_allowed(t0 + s(20)), t0 + s(20));
        r.record(t0 + s(20));
        assert_eq!(r.next_allowed(t0 + s(20)), t0 + s(21));
        let later = t0 + s(100);
        for i in 0..5 {
            assert_eq!(r.next_allowed(later), later, "{i}");
            r.record(later);
        }
        assert_eq!(r.next_allowed(later), later + s(20));
    }

    #[test]
    fn retries_double_up_to_the_cap_and_start_over_after_ready() {
        let t0 = Instant::now();
        let s = Duration::from_secs;
        let mut r = Retry::new(s(1), s(30), t0);
        assert_eq!(r.at(), t0);
        let delays: Vec<u64> = (0..8).map(|_| r.failed(t0).as_secs()).collect();
        assert_eq!(delays, [1, 2, 4, 8, 16, 30, 30, 30]);
        assert_eq!(r.at(), t0 + s(30));
        r.reset();
        assert_eq!(r.failed(t0), s(1));
    }
}
