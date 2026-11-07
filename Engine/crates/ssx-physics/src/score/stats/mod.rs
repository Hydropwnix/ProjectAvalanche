use std::collections::{BTreeMap, BTreeSet};

use ssx_lua::game::objectives::{Cmp, StatKey, StatValue, fnv1};

use crate::scoring::{BanksFlushed, ComboEnded, RecordKind, TrickRecord};

mod registry;
mod writers;

pub use registry::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Record {
    pub value: StatValue,
    pub initial: StatValue,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Tracker {
    pub tricky: bool,
    pub uber: bool,
    pub wingsuit: bool,
    pub boosting: bool,
    pub tricky_time: f32,
    pub uber_time: f32,
    pub speed: f32,
    pub above: [f32; 4],
    pub rewinding: bool,
    pub taken_down: [bool; crate::event::MAX_RACERS],
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerStats {
    pub awesome: bool,
    pub super_awesome: bool,
    pub wingsuit: bool,
    pub boosting: bool,
    pub speed: f32,
    pub avalanche_mass: f32,
    pub racers: [Option<RacerStats>; crate::event::MAX_RACERS],
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RacerStats {
    pub trigger_wipeout: bool,
    pub in_air: bool,
}

#[derive(Clone, Debug)]
pub struct StatsStore {
    pub writeable: bool,
    pub maps: [BTreeMap<u32, Record>; 3],
    pub names: BTreeMap<u32, String>,
    ids: Vec<u32>,
    pub awarded: [BTreeSet<u32>; 3],
    pub badge_progress: BTreeMap<u32, f32>,
    pub tracker: Tracker,
    pub rider: Option<String>,
    pub added_tricks: BTreeMap<std::sync::Arc<str>, [u32; 3]>,
}

impl Default for StatsStore {
    fn default() -> Self {
        let mut s = StatsStore {
            writeable: true,
            maps: Default::default(),
            names: BTreeMap::new(),
            ids: Vec::new(),
            awarded: Default::default(),
            badge_progress: BTreeMap::new(),
            tracker: Tracker::default(),
            rider: None,
            added_tricks: BTreeMap::new(),
        };
        for (name, kind, scopes) in STATS {
            let h = s.register(name, kind, scopes);
            s.ids.push(h);
        }
        for item in GEAR_ITEMS {
            s.register(&format!("GearEquipped_Track_{item}"), Kind::Int, &[TRACK]);
        }
        for family in TRICK_FAMILIES {
            for trick in TRICK_NAMES {
                s.register(&format!("{family}{trick}"), Kind::Int, &[PERSISTENT]);
            }
        }
        s
    }
}

fn zero(kind: Kind) -> StatValue {
    match kind {
        Kind::Int => StatValue::Int(0),
        Kind::Float => StatValue::Float(0.0),
    }
}

impl StatsStore {
    fn register(&mut self, name: &str, kind: Kind, scopes: &[usize]) -> u32 {
        let h = fnv1(name.as_bytes());
        self.names.insert(h, name.to_string());
        for &sc in scopes {
            self.maps[sc].insert(h, Record { value: zero(kind), initial: zero(kind) });
        }
        h
    }

    pub fn get(&self, key: &StatKey) -> Option<StatValue> {
        let sc = usize::try_from(key.scope).ok().filter(|&s| s < 3)?;
        self.maps[sc].get(&key.key()).map(|r| r.value)
    }

    pub fn value(&self, name: &str, scope: usize) -> Option<StatValue> {
        self.maps.get(scope)?.get(&fnv1(name.as_bytes())).map(|r| r.value)
    }

    fn float(&self, i: usize, sc: usize) -> f32 {
        match self.maps[sc].get(&self.ids[i]).map(|r| r.value) {
            Some(StatValue::Float(f)) => f,
            _ => 0.0,
        }
    }

    pub fn add_int_key(&mut self, h: u32, sc: usize, amount: u32, create: bool) {
        if !self.writeable {
            return;
        }
        if create && !self.maps[sc].contains_key(&h) {
            self.maps[sc].insert(h, Record { value: StatValue::Int(0), initial: StatValue::Int(0) });
        }
        if let Some(Record { value: StatValue::Int(v), .. }) = self.maps[sc].get_mut(&h) {
            *v = v.saturating_add(amount);
        }
    }

    fn add_int(&mut self, i: usize, sc: usize, amount: u32) {
        self.add_int_key(self.ids[i], sc, amount, false);
    }

    fn set_int(&mut self, i: usize, sc: usize, v: u32) {
        if let (true, Some(Record { value: StatValue::Int(x), .. })) =
            (self.writeable, self.maps[sc].get_mut(&self.ids[i]))
        {
            *x = v;
        }
    }

    fn add_float(&mut self, i: usize, sc: usize, amount: f32) {
        if let (true, Some(Record { value: StatValue::Float(x), .. })) =
            (self.writeable, self.maps[sc].get_mut(&self.ids[i]))
        {
            *x += amount;
        }
    }

    fn set_float(&mut self, i: usize, sc: usize, v: f32) {
        if let (true, Some(Record { value: StatValue::Float(x), .. })) =
            (self.writeable, self.maps[sc].get_mut(&self.ids[i]))
        {
            *x = v;
        }
    }

    pub fn reset(&mut self, sc: usize) {
        if sc != TRICK {
            self.awarded[if sc == TRACK { 0 } else { 2 }].clear();
        }
        for r in self.maps[sc].values_mut() {
            r.value = r.initial;
        }
    }

    pub fn award(&mut self, name: &str, kind: usize) {
        if self.writeable && kind < 3 {
            self.awarded[kind].insert(fnv1(name.as_bytes()));
            self.names.entry(fnv1(name.as_bytes())).or_insert_with(|| name.to_string());
        }
    }

    pub fn awarded(&self, hash: u32, kind: usize) -> bool {
        self.awarded.get(kind).is_some_and(|m| m.contains(&hash))
    }

    pub fn track_bonuses(&self) -> Vec<String> {
        self.awarded[0].iter().filter_map(|h| self.names.get(h).cloned()).collect()
    }
}

fn trick_event(t: Option<i32>) -> bool {
    matches!(t, Some(0..=2))
}
fn survive_event(t: Option<i32>) -> bool {
    matches!(t, Some(3 | 4))
}
fn race_event(t: Option<i32>) -> bool {
    matches!(t, Some(5..=7))
}

impl StatsStore {
    // TODO(guess): `EquippedSpeedBoard_Track` (`FUN_8248CC00(inventory, 1)`, the board's dominant attribute 0x82524800) is not written (the record's +8 is not traced, no script reads it); settle: the record's layout (`FUN_8239F478`) and the board's item.
    pub fn level_start(&mut self, equipped: &[bool; 15], event_type: Option<i32>) {
        crate::guess_hit!();
        self.reset(TRACK);
        self.reset(TRICK);
        for (i, item) in GEAR_ITEMS.iter().enumerate() {
            if equipped[i] && self.writeable {
                let h = fnv1(format!("GearEquipped_Track_{item}").as_bytes());
                if let Some(r) = self.maps[TRACK].get_mut(&h) {
                    r.value = StatValue::Int(1);
                }
            }
        }
        if trick_event(event_type) {
            self.add_int(id::NUM_TIMES_COMPETED_TRICK_TRACK, TRACK, 1);
        } else if survive_event(event_type) {
            self.add_int(id::NUM_TIMES_COMPETED_SURVIVE_TRACK, TRACK, 1);
        } else if race_event(event_type) {
            self.add_int(id::NUM_TIMES_COMPETED_RACE_TRACK, TRACK, 1);
        }
        self.tracker = Tracker::default();
    }

    pub fn update(&mut self, dt: f32, clock_ms: u32, backwards: bool, event_type: Option<i32>, p: &PlayerStats) {
        self.set_float(id::TIME_TRACK, TRACK, clock_ms as f32 * 0.001);
        self.rewind(backwards, event_type);
        if self.tracker.rewinding {
            return;
        }
        let t = &mut self.tracker;
        let (tricky_up, uber_up) = (!t.tricky && p.awesome, !t.uber && p.super_awesome);
        if t.tricky != p.awesome {
            t.tricky_time = 0.0;
        }
        if t.uber != p.super_awesome {
            t.uber_time = 0.0;
        }
        let (wingsuit_up, boost_up) = (!t.wingsuit && p.wingsuit, !t.boosting && p.boosting);
        (t.tricky, t.uber, t.wingsuit, t.boosting, t.speed) =
            (p.awesome, p.super_awesome, p.wingsuit, p.boosting, p.speed);
        if tricky_up {
            self.add_int(id::NUM_TIMES_ENGAGED_TRICKY, PERSISTENT, 1);
        }
        if uber_up {
            self.add_int(id::NUM_TIMES_ENGAGED_UBER_TRICKY, PERSISTENT, 1);
        }
        if wingsuit_up {
            self.add_int(id::NUM_TIMES_ENGAGED_WINGSUIT, PERSISTENT, 1);
        }
        if boost_up {
            self.add_int(id::NUM_TIMES_USED_BOOST_TRACK, TRACK, 1);
            self.add_int(id::NUM_TIMES_USED_BOOST, PERSISTENT, 1);
        }
        let speed = self.tracker.speed;
        if self.float(id::MAX_SPEED_TRACK, TRACK) < speed {
            self.set_float(id::MAX_SPEED_TRACK, TRACK, speed);
        }
        if self.float(id::MAX_SPEED, PERSISTENT) < speed {
            self.set_float(id::MAX_SPEED, PERSISTENT, speed);
        }
        if self.float(id::MAX_AVALANCHE_MASS_TRACK, TRACK) < p.avalanche_mass {
            self.set_float(id::MAX_AVALANCHE_MASS_TRACK, TRACK, p.avalanche_mass);
        }
        if self.tracker.uber || self.tracker.tricky {
            self.add_float(id::NUM_TIME_IN_TRICKY, PERSISTENT, dt);
            self.add_float(id::NUM_TIME_IN_TRICKY_TRACK, TRACK, dt);
            self.tracker.tricky_time += dt;
            let streak = self.tracker.tricky_time;
            if self.float(id::MAX_TIME_TRICKY_ENGAGED, PERSISTENT) < streak {
                self.add_float(id::MAX_TIME_TRICKY_ENGAGED, PERSISTENT, dt);
            }
            if self.float(id::MAX_TIME_TRICKY_ENGAGED_TRACK, TRACK) < streak {
                self.add_float(id::MAX_TIME_TRICKY_ENGAGED_TRACK, TRACK, dt);
            }
        }
        if self.tracker.wingsuit {
            self.add_float(id::NUM_TIME_USING_WINGSUIT, PERSISTENT, dt);
            self.add_float(id::NUM_TIME_USING_WINGSUIT_TRACK, TRACK, dt);
        }
        if self.tracker.boosting {
            self.add_float(id::NUM_TIME_BOOSTING_TRACK, TRACK, dt);
            self.add_float(id::NUM_TIME_BOOSTING, PERSISTENT, dt);
        }
        let kmh = self.tracker.speed * 3.6;
        for (k, target) in TARGET_SPEEDS_KMH.iter().enumerate() {
            if kmh <= *target {
                self.tracker.above[k] = 0.0;
                continue;
            }
            self.tracker.above[k] += dt;
            let i = id::TIME_EXCEEDING_TARGET_SPEED1 + k;
            for sc in [PERSISTENT, TRACK] {
                if self.float(i, sc) < self.tracker.above[k] {
                    self.add_float(i, sc, dt);
                }
            }
        }
        for (k, r) in p.racers.iter().enumerate() {
            let Some(r) = r else { continue };
            let down = r.trigger_wipeout && !r.in_air;
            if down != self.tracker.taken_down[k] {
                self.tracker.taken_down[k] = down;
                if down {
                    self.add_int(id::NUM_CHARACTERS_TAKEN_DOWN, PERSISTENT, 1);
                    self.add_int(id::NUM_CHARACTERS_TAKEN_DOWN_TRACK, TRACK, 1);
                }
            }
        }
    }

    pub fn track_ended(&mut self, m: &crate::scoring::TrackEnded) {
        for (k, set) in [m.tricky, m.uber_tricky, m.wingsuit, m.boosting].into_iter().enumerate() {
            if set {
                self.add_int(id::COMPLETED_TRACK_WHILE_TRICKY_TRACK + k, TRACK, 1);
            }
        }
    }

    pub fn base_points_awarded(&mut self, points: f32, event_type: Option<i32>) {
        if trick_event(event_type) {
            self.add_float(id::COMBO_SCORE_TRICK_EVENT_TRACK, TRACK, points);
            self.add_float(id::COMBO_SCORE_TRICK_EVENT, PERSISTENT, points);
        }
    }

    pub fn rewound_out_of_death(&mut self) {
        self.add_int(id::NUM_REWINDS_OUT_OF_DEATH, PERSISTENT, 1);
    }

    pub fn rewind(&mut self, backwards: bool, event_type: Option<i32>) {
        if backwards && !self.tracker.rewinding {
            self.add_int(id::NUM_REWINDS_USED, PERSISTENT, 1);
            self.add_int(id::NUM_REWINDS_USED_TRACK, TRACK, 1);
            if survive_event(event_type) {
                self.add_int(id::NUM_SURVIVE_REWINDS_USED, PERSISTENT, 1);
            }
        }
        self.tracker.rewinding = backwards;
    }
}

#[cfg(test)]
mod tests;
