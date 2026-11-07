use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use ssx_net::address::AddressKey;

#[derive(Clone, Copy, Debug)]
pub struct Rate {
    pub per_second: f64,
    pub burst: f64,
}

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

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Verdict {
    Allowed,
    Limited(Duration),
    Full,
}

pub struct AddressLimiter {
    rate: Rate,
    capacity: usize,
    buckets: Mutex<HashMap<AddressKey, Bucket>>,
}

impl AddressLimiter {
    pub fn new(rate: Rate, capacity: usize) -> Self {
        Self { rate, capacity, buckets: Mutex::new(HashMap::new()) }
    }

    pub fn check(&self, key: AddressKey, now: Instant) -> Verdict {
        let Ok(mut buckets) = self.buckets.lock() else { return Verdict::Full };
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
            Verdict::Limited(Duration::from_secs_f64((1.0 - bucket.tokens) / self.rate.per_second))
        }
    }

    pub fn tracked(&self) -> usize {
        self.buckets.lock().map_or(0, |b| b.len())
    }
}

pub struct Connections {
    total: AtomicUsize,
    max_total: usize,
    max_per_address: u32,
    per_address: Mutex<HashMap<AddressKey, u32>>,
}

pub struct ConnectionSlot<'a> {
    owner: &'a Connections,
    key: AddressKey,
}

impl Drop for ConnectionSlot<'_> {
    fn drop(&mut self) {
        self.owner.total.fetch_sub(1, Ordering::AcqRel);
        if let Ok(mut map) = self.owner.per_address.lock()
            && let Some(n) = map.get_mut(&self.key)
        {
            *n -= 1;
            if *n == 0 {
                map.remove(&self.key);
            }
        }
    }
}

impl Connections {
    pub fn new(max_total: usize, max_per_address: u32) -> Self {
        Self { total: AtomicUsize::new(0), max_total, max_per_address, per_address: Mutex::new(HashMap::new()) }
    }

    pub fn open(&self, key: AddressKey) -> Option<ConnectionSlot<'_>> {
        let mut map = self.per_address.lock().ok()?;
        if self.total.load(Ordering::Acquire) >= self.max_total {
            return None;
        }
        let n = map.entry(key).or_insert(0);
        if *n >= self.max_per_address {
            return None;
        }
        *n += 1;
        self.total.fetch_add(1, Ordering::AcqRel);
        Some(ConnectionSlot { owner: self, key })
    }

    pub fn open_now(&self) -> usize {
        self.total.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(i: u32) -> AddressKey {
        AddressKey::V4(i.to_be_bytes())
    }

    #[test]
    fn buckets_refill_and_the_table_stays_bounded() {
        let limiter = AddressLimiter::new(Rate { per_second: 2.0, burst: 3.0 }, 4);
        let t0 = Instant::now();
        for _ in 0..3 {
            assert_eq!(limiter.check(key(1), t0), Verdict::Allowed);
        }
        assert!(matches!(limiter.check(key(1), t0), Verdict::Limited(d) if d <= Duration::from_millis(500)));
        assert_eq!(limiter.check(key(1), t0 + Duration::from_millis(500)), Verdict::Allowed);
        for i in 2..=4 {
            assert_eq!(limiter.check(key(i), t0), Verdict::Allowed);
        }
        assert_eq!(limiter.check(key(5), t0), Verdict::Full);
        assert_eq!(limiter.check(key(5), t0 + Duration::from_secs(10)), Verdict::Allowed);
        assert!(limiter.tracked() <= 4);
    }

    #[test]
    fn connection_caps() {
        let c = Connections::new(3, 2);
        let a = c.open(key(1)).expect("first");
        let b = c.open(key(1)).expect("second");
        assert!(c.open(key(1)).is_none(), "per address");
        let d = c.open(key(2)).expect("other address");
        assert!(c.open(key(3)).is_none(), "overall");
        drop(a);
        assert!(c.open(key(3)).is_some());
        drop((b, d));
        assert_eq!(c.open_now(), 0);
    }
}
