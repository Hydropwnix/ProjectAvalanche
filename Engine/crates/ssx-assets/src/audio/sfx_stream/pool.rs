use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::Result;

use super::{Ring, Slot};

pub const UNSTEALABLE: f32 = 100.0;

pub struct Pool {
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    ring: Ring,
    slots: Vec<Entry>,
    clock: u64,
}

struct Entry {
    slot: Option<Slot>,
    owner: u64,
    priority: f32,
    since: u64,
    holds: u16,
    generation: u64,
}

pub struct Lease {
    inner: Arc<Mutex<Inner>>,
    index: usize,
    generation: u64,
}

fn lock(inner: &Mutex<Inner>) -> MutexGuard<'_, Inner> {
    inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Pool {
    pub fn new(ring: Ring, slots: usize) -> Self {
        let slots = (0..slots)
            .map(|_| Entry { slot: None, owner: 0, priority: 0.0, since: 0, holds: 0, generation: 0 })
            .collect();
        Self { inner: Arc::new(Mutex::new(Inner { ring, slots, clock: 0 })) }
    }

    pub fn acquire(&self, owner: u64, priority: f32) -> Result<Option<Lease>> {
        let mut p = lock(&self.inner);
        let p = &mut *p;
        p.clock += 1;
        let index = if let Some(i) = p.slots.iter().position(|e| e.holds > 0 && e.owner == owner) {
            p.slots[i].holds += 1;
            i
        } else {
            let taken = match p.slots.iter().position(|e| e.holds == 0) {
                Some(i) => i,
                None => {
                    let mut low: Option<usize> = None;
                    for (i, e) in p.slots.iter().enumerate() {
                        if low.is_none_or(|l| {
                            let l = &p.slots[l];
                            e.priority < l.priority || (e.priority == l.priority && e.since < l.since)
                        }) {
                            low = Some(i);
                        }
                    }
                    let Some(i) = low.filter(|&i| p.slots[i].priority < priority && p.slots[i].priority < UNSTEALABLE)
                    else {
                        return Ok(None);
                    };
                    if let Some(s) = p.slots[i].slot.as_mut() {
                        s.flush();
                    }
                    i
                }
            };
            let ring = p.ring;
            let e = &mut p.slots[taken];
            if e.slot.is_none() {
                e.slot = Some(Slot::new(ring)?);
            }
            e.generation += 1;
            (e.owner, e.priority, e.holds) = (owner, priority, 1);
            // TODO(guess): a slot's age is the order of acquisitions; the game stores the pool's clock (a double
            // behind pool +0, 0x82A2DAF8 `*slot = *(double *)*pool`) and breaks a tie of the lowest priority by the
            // smallest — the same order unless two slots were taken at one clock value; no SSX stream is ever taken
            // over (its voices have priority 100), so the tie is not reached; settle: the clock pool +0 points at
            // (the pool's set-up 0x82A49408).
            e.since = p.clock;
            taken
        };
        let generation = p.slots[index].generation;
        Ok(Some(Lease { inner: self.inner.clone(), index, generation }))
    }

    pub fn held(&self) -> usize {
        lock(&self.inner).slots.iter().filter(|e| e.holds > 0).count()
    }
}

impl Lease {
    pub fn with<R>(&self, f: impl FnOnce(&mut Slot) -> R) -> Option<R> {
        let mut p = lock(&self.inner);
        let e = &mut p.slots[self.index];
        if e.generation != self.generation {
            return None;
        }
        e.slot.as_mut().map(f)
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let mut p = lock(&self.inner);
        let e = &mut p.slots[self.index];
        if e.generation != self.generation || e.holds == 0 {
            return;
        }
        e.holds -= 1;
        if e.holds == 0 {
            if let Some(s) = e.slot.as_mut() {
                s.flush();
            }
            e.generation += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sfx_pool_never_takes_a_slot_over() {
        let pool = Pool::new(Ring::SPEECH, 5);
        let leases: Vec<_> = (1..=5).map(|o| pool.acquire(o, UNSTEALABLE).unwrap().unwrap()).collect();
        assert!(pool.acquire(6, UNSTEALABLE).unwrap().is_none());
        let again = pool.acquire(3, UNSTEALABLE).unwrap().unwrap();
        assert_eq!(again.index, leases[2].index);
        assert_eq!(pool.held(), 5);
        let mut leases = leases;
        drop(leases.remove(2));
        assert_eq!(pool.held(), 5, "owner 3 still holds it once");
        drop(again);
        assert_eq!(pool.held(), 4);
        let six = pool.acquire(6, UNSTEALABLE).unwrap().unwrap();
        assert!(six.with(|s| s.queued()).is_some());
        assert_eq!(pool.held(), 5);
    }

    #[test]
    fn a_lower_priority_loses_its_slot() {
        let pool = Pool::new(Ring::SPEECH, 2);
        let a = pool.acquire(1, 10.0).unwrap().unwrap();
        let b = pool.acquire(2, 10.0).unwrap().unwrap();
        assert!(pool.acquire(3, 10.0).unwrap().is_none(), "not above the lowest");
        let c = pool.acquire(3, 50.0).unwrap().unwrap();
        assert_eq!(c.index, a.index, "the oldest of the lowest");
        assert!(a.with(|_| ()).is_none());
        assert!(b.with(|_| ()).is_some());
        drop(a);
        assert_eq!(pool.held(), 2, "a lost lease gives nothing back");
        let d = pool.acquire(4, 99.0).unwrap().unwrap();
        assert!(b.with(|_| ()).is_none());
        let e = pool.acquire(5, 1000.0).unwrap().unwrap();
        assert_eq!(e.index, c.index, "the lowest, 50, is below 100");
        let f = pool.acquire(6, UNSTEALABLE).unwrap().unwrap();
        assert_eq!(f.index, d.index, "99 is below 100");
        assert!(pool.acquire(7, 2000.0).unwrap().is_none(), "the lowest is 100: never taken over");
    }
}
