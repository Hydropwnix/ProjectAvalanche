use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::http::limits::{Rate, Verdict};

#[derive(Clone, Copy)]
struct Bucket {
    tokens: f64,
    at: Instant,
}

impl Bucket {
    fn refill(&mut self, rate: Rate, now: Instant) {
        let gained = now.saturating_duration_since(self.at).as_secs_f64() * rate.per_second;
        self.tokens = (self.tokens + gained).min(rate.burst);
        self.at = now;
    }
}

pub struct Throttle<K> {
    rate: Rate,
    capacity: usize,
    buckets: Mutex<HashMap<K, Bucket>>,
}

impl<K: Hash + Eq + Copy> Throttle<K> {
    pub fn new(rate: Rate, capacity: usize) -> Self {
        Self { rate, capacity, buckets: Mutex::new(HashMap::new()) }
    }

    fn wait(&self, tokens: f64) -> Duration {
        Duration::from_secs_f64(((1.0 - tokens) / self.rate.per_second).max(0.0))
    }

    pub fn spend(&self, key: K, now: Instant) -> Verdict {
        let mut buckets = self.buckets.lock().unwrap_or_else(PoisonError::into_inner);
        if !buckets.contains_key(&key) && buckets.len() >= self.capacity {
            let rate = self.rate;
            buckets.retain(|_, b| {
                b.refill(rate, now);
                b.tokens < rate.burst
            });
            if buckets.len() >= self.capacity {
                return Verdict::Full;
            }
        }
        let bucket = buckets.entry(key).or_insert(Bucket { tokens: self.rate.burst, at: now });
        bucket.refill(self.rate, now);
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            Verdict::Allowed
        } else {
            Verdict::Limited(self.wait(bucket.tokens))
        }
    }

    pub fn peek(&self, key: K, now: Instant) -> Verdict {
        let mut buckets = self.buckets.lock().unwrap_or_else(PoisonError::into_inner);
        match buckets.get_mut(&key) {
            None => Verdict::Allowed,
            Some(b) => {
                b.refill(self.rate, now);
                if b.tokens >= 1.0 { Verdict::Allowed } else { Verdict::Limited(self.wait(b.tokens)) }
            }
        }
    }

    pub fn tracked(&self) -> usize {
        self.buckets.lock().map_or(0, |b| b.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_lock_a_key_out_until_its_bucket_refills() {
        let t = Throttle::new(Rate { per_second: 1.0 / 60.0, burst: 3.0 }, 2);
        let t0 = Instant::now();
        assert_eq!(t.peek(7u8, t0), Verdict::Allowed);
        for _ in 0..3 {
            assert_eq!(t.spend(7u8, t0), Verdict::Allowed);
        }
        assert!(matches!(t.peek(7u8, t0), Verdict::Limited(d) if d.as_secs() == 60));
        assert!(matches!(t.spend(7u8, t0), Verdict::Limited(_)));
        assert_eq!(t.peek(7u8, t0 + Duration::from_secs(61)), Verdict::Allowed);
        assert_eq!(t.spend(8u8, t0), Verdict::Allowed);
        assert_eq!(t.spend(9u8, t0), Verdict::Full);
        assert_eq!(t.spend(9u8, t0 + Duration::from_secs(600)), Verdict::Allowed);
        assert!(t.tracked() <= 2);
    }
}
