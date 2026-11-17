use std::collections::VecDeque;

use ssx_assets::career::random::{GameRandom, Mt19937};

use super::engine::{self, BUCKET_LEN, CAPS, Candidate, Collector, Likes, PeakDrop, kind};
use crate::game::fe::menus::postrace::next_index;

#[derive(Default)]
struct Script {
    answers: VecDeque<u32>,
    asked: Vec<u32>,
}

impl Script {
    fn new(answers: &[u32]) -> Self {
        Self { answers: answers.iter().copied().collect(), asked: Vec::new() }
    }
}

impl GameRandom for Script {
    fn next_u32(&mut self) -> u32 {
        0
    }
    fn below(&mut self, n: u32) -> u32 {
        self.asked.push(n);
        self.answers.pop_front().unwrap_or(0).min(n.saturating_sub(1))
    }
}

fn first(drop: u32, score: f32) -> Candidate {
    Candidate { kind: kind::FIRST_TRACKS, score, drop, mode: 0, gear: 0 }
}

fn open(key: u32, cost: u32, required: &[i64]) -> PeakDrop {
    PeakDrop { key, cost, event_type: 0, required: required.to_vec(), open: true, own_best: false }
}

#[test]
fn weights_and_priorities_are_the_tables() {
    let w: Vec<f64> = (0..14).map(engine::weight).collect();
    assert_eq!(w, [0.0, 1.0, 2.0, 1.0, 1.0, 0.0, 3.0, 2.0, 3.0, 1.0, 4.0, 1.0, 1.0, 0.0]);
    assert_eq!(engine::priority(&first(1, 0.25)), 0.25);
    let fav = Candidate { kind: kind::FAVORITE, score: 1.0, drop: 1, mode: 0, gear: 0 };
    assert_eq!(engine::priority(&fav), 2.0);
    assert_eq!(CAPS.iter().sum::<usize>(), 39);
}

#[test]
fn a_bucket_keeps_its_best_ten_in_order_ties_first_come() {
    let mut c = Collector::default();
    for d in 0..12u32 {
        c.add(first(d, if d % 2 == 0 { 0.5 } else { 0.25 }));
    }
    let b = c.bucket(kind::FIRST_TRACKS);
    assert_eq!(b.len(), BUCKET_LEN);
    let drops: Vec<u32> = b.iter().map(|x| x.drop).collect();
    assert_eq!(drops, [0, 2, 4, 6, 8, 10, 1, 3, 5, 7]);
    c.add(first(99, 0.25));
    assert!(!c.bucket(kind::FIRST_TRACKS).iter().any(|x| x.drop == 99));
    c.add(first(98, 0.75));
    let b = c.bucket(kind::FIRST_TRACKS);
    assert_eq!((b[0].drop, b.len(), b[9].drop), (98, BUCKET_LEN, 5));
}

#[test]
fn a_kind_out_of_the_table_is_not_collected() {
    let mut c = Collector::default();
    c.add(Candidate { kind: 13, ..first(1, 1.0) });
    assert!(c.draw(10, &mut Script::default()).is_empty());
}

#[test]
fn the_draw_pools_the_capped_heads_and_sorts_by_priority() {
    let mut c = Collector::default();
    for d in 0..5u32 {
        c.add(first(d, 1.0 - d as f32 * 0.1));
    }
    c.add(Candidate { kind: kind::FAVORITE, score: 1.0, drop: 50, mode: 0, gear: 0 });
    c.add(Candidate { kind: kind::FAVORITE, score: 1.0, drop: 51, mode: 0, gear: 0 });
    c.add(Candidate { kind: kind::GEAR, score: 0.5, drop: 60, mode: 0, gear: 2 });
    c.add(Candidate { kind: kind::GEAR, score: 0.5, drop: 61, mode: 0, gear: 2 });
    let mut rng = Script::new(&[5, 0, 3, 0, 0, 0]);
    let out = c.draw(10, &mut rng);
    assert_eq!(rng.asked, [6, 5, 4, 3, 2, 1]);
    let drops: Vec<u32> = out.iter().map(|x| x.drop).collect();
    assert_eq!(drops, [51, 50, 60, 0, 1, 2]);
    let mut rng = Script::new(&[5, 0]);
    let drops: Vec<u32> = c.draw(2, &mut rng).iter().map(|x| x.drop).collect();
    assert_eq!((rng.asked.as_slice(), drops.as_slice()), ([6, 5].as_slice(), [60, 0].as_slice()));
}

#[test]
fn first_tracks_scores_by_price_against_the_peaks_highest() {
    assert_eq!(engine::first_tracks_score(10_000, 10_000), (1.0f64 / 10_000.0) as f32);
    assert_eq!(engine::first_tracks_score(10_000, 5_000), (5_001.0f64 / 10_000.0) as f32);
    assert_eq!(engine::first_tracks_score(10_000, 0), 1.0);
    assert_eq!(engine::first_tracks_score(0, 0), 1.0);
}

#[test]
fn a_like_rings_five_drops_and_counts_the_gear_below_seven() {
    let mut l = Likes::default();
    for d in 1..=6u32 {
        l.like(d, &[0, 7]);
    }
    l.like(4, &[3, -1]);
    assert_eq!(l.drops, [2, 3, 4, 5, 6]);
    assert_eq!(l.gear, [6, 0, 0, 1, 0, 0, 0]);
}

#[test]
fn the_liked_gear_is_drawn_by_its_running_sum() {
    let l = Likes { drops: VecDeque::new(), gear: [0, 2, 0, 3, 0, 0, 1] };
    let pick = |r: u32| l.pick_gear(&mut Script::new(&[r]));
    assert_eq!([pick(0), pick(2), pick(3), pick(5)], [Some(1), Some(1), Some(3), Some(3)]);
    let mut none = Script::default();
    assert_eq!(Likes::default().pick_gear(&mut none), None);
    assert!(none.asked.is_empty());
}

#[test]
fn a_peak_gives_its_favourite_or_gear_drops_then_its_unplayed_ones() {
    let mut drops = vec![open(1, 0, &[0]), open(2, 10_000, &[1]), open(3, 20_000, &[1, 7]), open(4, 5_000, &[1])];
    drops[0].own_best = true;
    drops.push(PeakDrop { open: false, ..open(5, 40_000, &[1]) });
    let likes = Likes { drops: VecDeque::new(), gear: [0, 1, 0, 0, 0, 0, 0] };
    let mut c = Collector::default();
    engine::peak(&drops, &likes, &mut c, &mut Script::default());
    let gear: Vec<u32> = c.bucket(kind::GEAR).iter().map(|x| x.drop).collect();
    assert_eq!(gear, [2, 3, 4]);
    let ft: Vec<(u32, f32)> = c.bucket(kind::FIRST_TRACKS).iter().map(|x| (x.drop, x.score)).collect();
    let s = |cost: u32| engine::first_tracks_score(40_000, cost);
    assert_eq!(ft, [(4, s(5_000)), (2, s(10_000)), (3, s(20_000))]);
    let likes = Likes { drops: VecDeque::from([3]), gear: [0, 1, 0, 0, 0, 0, 0] };
    let mut c = Collector::default();
    engine::peak(&drops, &likes, &mut c, &mut Script::default());
    let gear: Vec<u32> = c.bucket(kind::GEAR).iter().map(|x| x.drop).collect();
    let fav: Vec<u32> = c.bucket(kind::FAVORITE).iter().map(|x| x.drop).collect();
    assert_eq!((gear.as_slice(), fav.as_slice()), ([2].as_slice(), [3].as_slice()));
}

#[test]
fn the_ridden_drop_takes_a_draw_but_is_left_out() {
    let peaks = [vec![open(1, 0, &[]), open(2, 0, &[]), open(3, 0, &[])]];
    let mut rng = Script::default();
    let out = engine::suggestions(peaks.iter().map(Vec::as_slice), &Likes::default(), 1, 10, &mut rng);
    assert_eq!(rng.asked, [3, 2, 1]);
    assert_eq!(out.iter().map(|x| x.drop).collect::<Vec<_>>(), [2, 3]);
    let peaks: Vec<Vec<PeakDrop>> =
        (0..4u32).map(|p| (0..5).map(|d| open(p * 10 + d, d * 5_000, &[])).collect()).collect();
    let run = |seed| {
        let mut rng = Mt19937::seeded(seed);
        engine::suggestions(peaks.iter().map(Vec::as_slice), &Likes::default(), 999, 10, &mut rng)
    };
    let (a, b) = (run(7), run(7));
    assert_eq!(a, b);
    assert_eq!(a.len(), 3);
}

#[test]
fn x_steps_through_the_list_and_wraps() {
    assert_eq!([next_index(0, 3), next_index(1, 3), next_index(2, 3)], [1, 2, 0]);
    assert_eq!([next_index(0, 1), next_index(0, 0)], [0, 0]);
    assert_eq!([next_index(8, 12), next_index(9, 12)], [9, 0]);
}
