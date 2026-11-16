use std::collections::VecDeque;

use ssx_assets::career::random::GameRandom;

pub mod kind {
    pub const FIRST_TRACKS: u32 = 0;
    pub const RIVAL: u32 = 2;
    pub const FAVORITE: u32 = 11;
    pub const GEAR: u32 = 12;
}

pub const BUCKETS: usize = 13;
pub const BUCKET_LEN: usize = 10;
pub const CAPS: [usize; BUCKETS] = [3, 1, 10, 2, 2, 2, 5, 5, 3, 2, 1, 2, 1];
pub const GEAR_TYPES: usize = 7;
pub const LIKED: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candidate {
    pub kind: u32,
    pub score: f32,
    pub drop: u32,
    pub mode: i64,
    pub gear: usize,
}

pub fn weight(kind: u32) -> f64 {
    match kind {
        1 | 3 | 4 | 9 | 11 | 12 => 1.0,
        2 | 7 => 2.0,
        6 | 8 => 3.0,
        10 => 4.0,
        _ => 0.0,
    }
}

pub fn priority(c: &Candidate) -> f32 {
    (weight(c.kind) + f64::from(c.score)) as f32
}

fn sort(v: &mut [Candidate]) {
    for i in 1..v.len() {
        let x = v[i];
        let mut j = i;
        while j > 0 && priority(&v[j - 1]) < priority(&x) {
            v[j] = v[j - 1];
            j -= 1;
        }
        v[j] = x;
    }
}

#[derive(Clone, Debug, Default)]
pub struct Collector {
    buckets: [Vec<Candidate>; BUCKETS],
}

impl Collector {
    pub fn add(&mut self, c: Candidate) {
        let Some(bucket) = usize::try_from(c.kind).ok().and_then(|k| self.buckets.get_mut(k)) else { return };
        if bucket.len() >= BUCKET_LEN {
            match bucket.last() {
                Some(last) if priority(last) < priority(&c) => {
                    bucket.pop();
                }
                _ => return,
            }
        }
        bucket.push(c);
        sort(bucket);
    }

    pub fn bucket(&self, kind: u32) -> &[Candidate] {
        usize::try_from(kind).ok().and_then(|k| self.buckets.get(k)).map_or(&[], Vec::as_slice)
    }

    pub fn draw(&self, n: usize, rng: &mut impl GameRandom) -> Vec<Candidate> {
        let mut pool: Vec<Candidate> =
            self.buckets.iter().zip(CAPS).flat_map(|(b, cap)| b.iter().take(cap).copied()).collect();
        let mut out = Vec::with_capacity(n.min(pool.len()));
        for _ in 0..n.min(pool.len()) {
            let i = rng.below(pool.len() as u32) as usize;
            if i >= pool.len() {
                break;
            }
            out.push(pool.remove(i));
        }
        sort(&mut out);
        out
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Likes {
    pub drops: VecDeque<u32>,
    pub gear: [u32; GEAR_TYPES],
}

impl Likes {
    pub fn like(&mut self, drop: u32, required: &[i64]) {
        if !self.drops.contains(&drop) {
            if self.drops.len() >= LIKED {
                self.drops.pop_front();
            }
            self.drops.push_back(drop);
        }
        for &g in required {
            // PORT(choice): a negative type is skipped — only a mod's record has one (the disc's 154 Explore drops
            // hold 0…7); the console's compare is signed and it read its seven-name table 0x837E90C8 before its start.
            if let Some(n) = usize::try_from(g).ok().and_then(|g| self.gear.get_mut(g)) {
                *n = n.saturating_add(1);
            }
        }
    }

    pub fn pick_gear(&self, rng: &mut impl GameRandom) -> Option<usize> {
        let total = self.gear.iter().fold(0u32, |a, &n| a.saturating_add(n));
        if total == 0 {
            return None;
        }
        let r = rng.below(total);
        let mut sum = 0u32;
        for (i, &n) in self.gear.iter().enumerate() {
            sum = sum.saturating_add(n);
            if n > 0 && r <= sum {
                return Some(i);
            }
        }
        None
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PeakDrop {
    pub key: u32,
    pub cost: u32,
    pub event_type: i64,
    pub required: Vec<i64>,
    pub open: bool,
    pub own_best: bool,
}

pub fn first_tracks_score(max: u32, cost: u32) -> f32 {
    let n = max.wrapping_sub(cost).wrapping_add(1);
    let r = f64::from((f64::from(n) / f64::from(max)) as f32);
    let s = if -r < 0.0 { r } else { 0.0 };
    let s = if ((1.0 - s) as f32) < 0.0 { 1.0 } else { s };
    s as f32
}

pub fn peak(drops: &[PeakDrop], likes: &Likes, collector: &mut Collector, rng: &mut impl GameRandom) {
    let max = drops.iter().fold(0u32, |m, d| m.max(d.cost));
    let open: Vec<&PeakDrop> = drops.iter().filter(|d| d.open).collect();
    let gear = likes.pick_gear(rng);
    for d in &open {
        if likes.drops.contains(&d.key) {
            collector.add(Candidate { kind: kind::FAVORITE, score: 1.0, drop: d.key, mode: 0, gear: 0 });
            break;
        }
        if let Some(g) = gear
            && d.required.iter().any(|&r| usize::try_from(r).ok() == Some(g))
        {
            collector.add(Candidate { kind: kind::GEAR, score: 0.5, drop: d.key, mode: 0, gear: g });
        }
    }
    for d in open.iter().filter(|d| !d.own_best) {
        let score = first_tracks_score(max, d.cost);
        collector.add(Candidate { kind: kind::FIRST_TRACKS, score, drop: d.key, mode: 0, gear: 0 });
    }
}

pub fn suggestions<'a>(
    peaks: impl IntoIterator<Item = &'a [PeakDrop]>,
    likes: &Likes,
    current: u32,
    count: usize,
    rng: &mut impl GameRandom,
) -> Vec<Candidate> {
    let mut collector = Collector::default();
    for p in peaks {
        peak(p, likes, &mut collector, rng);
    }
    collector.draw(count, rng).into_iter().filter(|c| c.drop != current).collect()
}
