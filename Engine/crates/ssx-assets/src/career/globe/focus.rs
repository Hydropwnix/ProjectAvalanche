use std::collections::{BTreeMap, BTreeSet};

use ssx_formats::attrib::fnv1;

use super::super::profile::Rules;
use super::super::world_tour;
use crate::career::data::DropType;

pub fn class_order(keys: &[u32], capacity: u32) -> Vec<u32> {
    let mut t = CollectionTable::default();
    if capacity != 0 {
        t.rebuild(capacity as usize);
    }
    for &k in keys {
        t.add(k);
    }
    t.slots.iter().flatten().copied().collect()
}

#[derive(Default)]
struct CollectionTable {
    slots: Vec<Option<u32>>,
    count: usize,
    max_probe: usize,
}

impl CollectionTable {
    fn rebuild(&mut self, mut capacity: usize) {
        loop {
            let old = std::mem::replace(&mut self.slots, vec![None; capacity]);
            self.count = 0;
            self.max_probe = 0;
            for k in old.into_iter().flatten() {
                self.insert(k);
            }
            if self.max_probe <= 0x60 {
                return;
            }
            capacity += 1;
        }
    }

    fn grown(capacity: usize) -> usize {
        if capacity == 0 { 1 } else { (((capacity * 5) >> 2) + 3) & !3 }
    }

    fn insert(&mut self, key: u32) -> bool {
        if self.count == self.slots.len() {
            self.rebuild(Self::grown(self.slots.len()));
        }
        let cap = self.slots.len();
        let mut slot = key as usize % cap;
        let mut probe = 0;
        while let Some(k) = self.slots[slot] {
            if k == key {
                return false;
            }
            probe += 1;
            slot = (slot + 1) % cap;
        }
        self.slots[slot] = Some(key);
        self.max_probe = self.max_probe.max(probe);
        self.count += 1;
        true
    }

    fn add(&mut self, key: u32) {
        self.insert(key);
        if self.max_probe > 0x60 {
            self.rebuild(Self::grown(self.slots.len()));
        }
    }
}

pub const RANGE_FIRST_PEAKS: [(&str, &str); 9] = [
    ("africa", "africa_peak_1"),
    ("alaska", "alaska_peak_1"),
    ("alps", "alps_peak_1"),
    ("antarctica ", "antarctica_peak_1"),
    ("himalayas", "himalayas_peak_1"),
    ("japan", "japan_peak_1"),
    ("new_zealand", "new_zealand_peak_1"),
    ("patagonia", "patagonia_peak_1"),
    ("rockies", "rockies_peak_1"),
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CampaignGlobe {
    pub range: u32,
    pub range_peak: BTreeMap<u32, u32>,
    pub peak_drop: BTreeMap<u32, u32>,
    pub range_selections: BTreeMap<u32, u32>,
}

impl CampaignGlobe {
    pub fn start(drops: &[u32], peaks: &BTreeSet<u32>, rules: &Rules) -> Self {
        let mut g = CampaignGlobe { range: fnv1(world_tour::START_RANGE.as_bytes()), ..Default::default() };
        for (range, peak) in RANGE_FIRST_PEAKS {
            g.range_peak.insert(fnv1(range.as_bytes()), fnv1(peak.as_bytes()));
        }
        for &d in drops {
            let Some(e) = rules.event(d).filter(|e| e.drop_type == DropType::Campaign) else { continue };
            if peaks.contains(&e.peak) {
                g.peak_drop.entry(e.peak).or_insert(d);
            }
        }
        g
    }

    pub fn merge_start(&mut self, start: &CampaignGlobe) {
        if self.range == 0 {
            self.range = start.range;
        }
        for (map, from) in [(&mut self.range_peak, &start.range_peak), (&mut self.peak_drop, &start.peak_drop)] {
            for (&k, &v) in from {
                map.entry(k).or_insert(v);
            }
        }
        for (&k, &v) in &start.range_selections {
            self.range_selections.entry(k).or_insert(v);
        }
    }

    pub fn progress(&mut self, rules: &Rules, drops: &[u32]) {
        for &d in drops {
            let (peak, range) = rules.event(d).map_or((0, 0), |e| (e.peak, e.range));
            self.peak_drop.insert(peak, d);
            self.range_peak.insert(range, peak);
            self.range = range;
        }
    }

    pub fn child(&mut self, key: u32, is_peak: bool, is_range: bool) -> u32 {
        if key == 0 {
            self.range
        } else if is_peak {
            *self.peak_drop.entry(key).or_insert(0)
        } else if is_range {
            *self.range_peak.entry(key).or_insert(0)
        } else {
            0
        }
    }

    pub fn has_peak(&self, peak: u32) -> bool {
        self.peak_drop.contains_key(&peak)
    }

    pub fn select_range(&mut self, range: u32) {
        *self.range_selections.entry(range).or_insert(0) += 1;
    }
}

pub fn return_node(from: u32) -> u32 {
    let first = fnv1(world_tour::FIRST_DROP.as_bytes());
    let second = fnv1(world_tour::SECOND_DROP.as_bytes());
    if from == first {
        second
    } else if from == second {
        fnv1(b"rockies_peak_1")
    } else {
        from
    }
}

pub fn node_screen(is_drop: bool, is_peak: bool, is_range: bool) -> (i32, u32) {
    if is_drop {
        (2, 0x44)
    } else if is_peak {
        (1, 0x43)
    } else if is_range {
        (0, 0x42)
    } else {
        (-1, 0x57)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Step {
    pub kind: u32,
    pub node: u32,
    pub seconds: f32,
    pub anim: i32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    pub steps: Vec<Step>,
    pub held: u32,
}

pub const FINAL_DROP: u32 = 0x28F6_34F8;

pub fn plan(from: u32, to: &[u32], node: impl Fn(u32) -> (u32, u32), from_dd: bool) -> Option<Plan> {
    let (from_peak, from_range) = node(from);
    let (kind, list) = if to.is_empty() {
        if !from_dd {
            return None;
        }
        (5, vec![from_range])
    } else if to[0] == FINAL_DROP {
        let (peak, range) = node(to[0]);
        (3, vec![from_range, range, peak, to[0]])
    } else {
        let (mut kind, mut list) = (7, Vec::new());
        for (i, &d) in to.iter().enumerate() {
            let (peak, range) = node(d);
            if range == from_range {
                if peak == from_peak {
                    list.push(d);
                    kind = 0;
                } else {
                    list.extend([peak, d]);
                    kind = 4;
                }
            } else {
                if i == 0 {
                    list.push(from_range);
                }
                list.push(range);
                kind = 2;
            }
        }
        (kind, list)
    };
    Some(queue(kind, from, &list))
}

pub fn queue(kind: u32, from: u32, list: &[u32]) -> Plan {
    let at = |i: usize| list.get(i).copied().unwrap_or(0);
    let s = |kind: u32, node: u32, seconds: f32, anim: i32| Step { kind, node, seconds, anim };
    let mut p = Plan::default();
    if kind > 5 {
        return p;
    }
    let mut steps = vec![s(0, from, 5.0, 1), s(1, from, 5.0, 1)];
    match kind {
        1 => steps.extend([s(3, at(0), 5.0, 1), s(9, at(0), 5.0, 1), s(10, at(0), 5.0, 1)]),
        3 => steps.extend([
            s(4, at(0), 5.0, 1),
            s(9, at(0), 5.0, 1),
            s(0, at(0), 5.0, 1),
            s(1, at(0), 5.0, 1),
            s(5, at(1), 5.0, 1),
            s(8, at(1), 2.0, 1),
            s(6, at(2), 2.0, 1),
            s(8, at(2), 2.5, 1),
            s(7, at(3), 2.5, 1),
            s(8, at(3), 2.5, 1),
            s(10, at(3), 2.5, 1),
        ]),
        4 => {
            steps.extend([
                s(3, at(0), 5.0, 1),
                s(9, at(0), 5.0, 1),
                s(8, at(0), 1.5, 1),
                s(7, at(1), 1.5, 1),
                s(0, at(1), 1.5, 2),
                s(1, at(1), 1.5, 2),
                s(10, at(1), 1.5, 2),
            ]);
            p.held = at(1);
        }
        0 => {
            steps.extend([s(2, at(0), 5.0, 1), s(0, at(0), 5.0, 2), s(1, at(0), 5.0, 2), s(10, at(0), 5.0, 2)]);
            p.held = at(0);
        }
        5 => steps.extend([
            s(4, at(0), 5.0, 1),
            s(9, at(0), 5.0, 1),
            s(0, at(0), 5.0, 1),
            s(1, at(0), 5.0, 1),
            s(10, at(0), 5.0, 1),
        ]),
        _ => {
            steps.extend([s(4, at(0), 5.0, 1), s(9, at(0), 5.0, 1), s(0, at(0), 5.0, 1), s(1, at(0), 5.0, 1)]);
            let (mut last, mut seconds) = (at(0), 5.0);
            for (i, &r) in list.iter().enumerate().skip(1) {
                steps.extend([s(5, r, if i == 1 { 5.0 } else { 1.5 }, 1), s(8, r, 1.5, 1)]);
                (last, seconds) = (r, 1.5);
            }
            steps.push(s(10, last, seconds, 1));
        }
    }
    p.steps = steps;
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_class_table_iterates_by_slot() {
        assert_eq!(class_order(&[10, 3, 17, 1], 7), vec![1, 10, 3, 17]);
        assert_eq!(class_order(&[1, 2, 3, 4, 5], 4), vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn the_return_node_skips_the_tutorials() {
        let k = |s: &str| fnv1(s.as_bytes());
        assert_eq!(return_node(k("chapter_2_drop_h2h")), k("chapter_2_drop_gen_1"));
        assert_eq!(return_node(k("chapter_2_drop_gen_1")), k("rockies_peak_1"));
        assert_eq!(return_node(k("chapter_3_drop_dd")), k("chapter_3_drop_dd"));
    }

    #[test]
    fn a_drop_on_the_same_peak_queues_kind_0() {
        let node = |d: u32| match d {
            1 | 2 => (10, 100),
            3 => (11, 100),
            _ => (12, 101),
        };
        let p = plan(1, &[2], node, false).unwrap();
        assert_eq!(p.steps.iter().map(|s| s.kind).collect::<Vec<_>>(), [0, 1, 2, 0, 1, 10]);
        assert_eq!(p.held, 2);
        let p = plan(1, &[3], node, false).unwrap();
        assert_eq!(p.steps.iter().map(|s| s.kind).collect::<Vec<_>>(), [0, 1, 3, 9, 8, 7, 0, 1, 10]);
        let p = plan(1, &[4], node, false).unwrap();
        assert_eq!(
            p.steps.iter().map(|s| (s.kind, s.node)).collect::<Vec<_>>()[2..],
            [(4, 100), (9, 100), (0, 100), (1, 100), (5, 101), (8, 101), (10, 101)]
        );
        assert!(plan(1, &[], node, false).is_none());
        assert_eq!(plan(1, &[], node, true).unwrap().steps[2], Step { kind: 4, node: 100, seconds: 5.0, anim: 1 });
    }
}
