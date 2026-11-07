use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use crate::address::AddressKey;

#[derive(Clone, Copy, Debug, PartialEq)]
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

    fn take(&mut self, rate: Rate) -> Verdict {
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            Verdict::Allowed
        } else {
            Verdict::Limited(Duration::from_secs_f64((1.0 - self.tokens) / rate.per_second))
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Verdict {
    Allowed,
    Limited(Duration),
    Full,
}

pub struct Limiter<K> {
    rate: Rate,
    capacity: usize,
    buckets: Mutex<HashMap<K, Bucket>>,
}

pub type AddressLimiter = Limiter<AddressKey>;

impl<K: Hash + Eq + Copy> Limiter<K> {
    pub fn new(rate: Rate, capacity: usize) -> Self {
        Self { rate, capacity, buckets: Mutex::new(HashMap::new()) }
    }

    pub fn check(&self, key: K, now: Instant) -> Verdict {
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
        bucket.take(self.rate)
    }

    pub fn tracked(&self) -> usize {
        self.buckets.lock().map_or(0, |b| b.len())
    }
}

pub struct GlobalLimiter {
    rate: Rate,
    bucket: Mutex<Bucket>,
}

impl GlobalLimiter {
    pub fn new(rate: Rate) -> Self {
        Self { rate, bucket: Mutex::new(Bucket { tokens: rate.burst, at: Instant::now() }) }
    }

    pub fn check(&self, now: Instant) -> Verdict {
        let Ok(mut b) = self.bucket.lock() else { return Verdict::Full };
        b.refill(self.rate, now);
        b.take(self.rate)
    }
}

pub struct Connections {
    total: AtomicUsize,
    max_total: usize,
    max_per_address: u32,
    per_address: Mutex<HashMap<AddressKey, u32>>,
    proxied: AtomicUsize,
    max_proxied: usize,
}

pub struct ConnectionSlot<'a> {
    owner: &'a Connections,
    key: AddressKey,
}

impl Drop for ConnectionSlot<'_> {
    fn drop(&mut self) {
        self.owner.total.fetch_sub(1, Ordering::AcqRel);
        self.owner.release(self.key);
    }
}

pub struct ProxySlot<'a> {
    owner: &'a Connections,
}

impl Drop for ProxySlot<'_> {
    fn drop(&mut self) {
        self.owner.proxied.fetch_sub(1, Ordering::AcqRel);
        self.owner.total.fetch_sub(1, Ordering::AcqRel);
    }
}

pub struct ClientSlot<'a> {
    owner: &'a Connections,
    key: AddressKey,
}

impl Drop for ClientSlot<'_> {
    fn drop(&mut self) {
        self.owner.release(self.key);
    }
}

impl Connections {
    pub fn new(max_total: usize, max_per_address: u32, max_proxied: usize) -> Self {
        Self {
            total: AtomicUsize::new(0),
            max_total,
            max_per_address,
            per_address: Mutex::new(HashMap::new()),
            proxied: AtomicUsize::new(0),
            max_proxied,
        }
    }

    fn release(&self, key: AddressKey) {
        if let Ok(mut map) = self.per_address.lock()
            && let Some(n) = map.get_mut(&key)
        {
            *n -= 1;
            if *n == 0 {
                map.remove(&key);
            }
        }
    }

    pub fn open_proxied(&self) -> Option<ProxySlot<'_>> {
        let _order = self.per_address.lock().ok()?;
        if self.total.load(Ordering::Acquire) >= self.max_total
            || self.proxied.load(Ordering::Acquire) >= self.max_proxied
        {
            return None;
        }
        self.proxied.fetch_add(1, Ordering::AcqRel);
        self.total.fetch_add(1, Ordering::AcqRel);
        Some(ProxySlot { owner: self })
    }

    pub fn client(&self, key: AddressKey) -> Option<ClientSlot<'_>> {
        let mut map = self.per_address.lock().ok()?;
        let n = map.entry(key).or_insert(0);
        if *n >= self.max_per_address {
            return None;
        }
        *n += 1;
        Some(ClientSlot { owner: self, key })
    }

    pub fn proxied_now(&self) -> usize {
        self.proxied.load(Ordering::Acquire)
    }

    pub fn addresses_now(&self) -> usize {
        self.per_address.lock().map_or(0, |m| m.len())
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
    fn buckets_refill_and_tables_stay_bounded() {
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
        assert_eq!(limiter.check(key(5), t0), Verdict::Full, "four active entries: a fifth is refused");
        assert_eq!(limiter.check(key(5), t0 + Duration::from_secs(10)), Verdict::Allowed, "full buckets make room");
        assert!(limiter.tracked() <= 4);
        let by_key: Limiter<[u8; 32]> = Limiter::new(Rate { per_second: 0.2, burst: 1.0 }, 2);
        assert_eq!(by_key.check([1; 32], t0), Verdict::Allowed);
        assert!(matches!(by_key.check([1; 32], t0), Verdict::Limited(d) if d > Duration::from_secs(4)));
        let global = GlobalLimiter::new(Rate { per_second: 1.0, burst: 2.0 });
        let now = Instant::now();
        assert_eq!((global.check(now), global.check(now)), (Verdict::Allowed, Verdict::Allowed));
        assert!(matches!(global.check(now), Verdict::Limited(_)));
    }

    #[test]
    fn connection_caps() {
        let c = Connections::new(3, 2, 3);
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

    #[test]
    fn a_proxys_pool_and_its_clients() {
        let c = Connections::new(4, 2, 3);
        let p: Vec<_> = (0..3).map(|_| c.open_proxied().expect("in the pool")).collect();
        assert!(c.open_proxied().is_none(), "the proxies' pool is full");
        let direct = c.open(key(9)).expect("the rest stays for direct peers");
        assert!(c.open(key(8)).is_none(), "overall");
        drop(direct);
        let a1 = c.client(key(1)).expect("a");
        let a2 = c.client(key(1)).expect("a again");
        assert!(c.client(key(1)).is_none(), "a holds its two");
        let b1 = c.client(key(2)).expect("b passes while a is limited");
        drop(a1);
        let d = c.open(key(1)).expect("a direct connection of a: one of its two");
        assert!(c.client(key(1)).is_none());
        assert_eq!((c.open_now(), c.proxied_now(), c.addresses_now()), (4, 3, 2));
        drop((a2, b1, d, p));
        assert_eq!((c.open_now(), c.proxied_now(), c.addresses_now()), (0, 0, 0));
        let full = Connections::new(2, 16, 2);
        let _d = (full.open(key(1)), full.open(key(2)));
        assert!(full.open_proxied().is_none(), "the overall cap holds for the proxies too");
    }
}
