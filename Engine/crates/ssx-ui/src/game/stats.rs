use std::collections::BTreeMap;

use ssx_assets::career::profile::{BadgeText, Profile, Rules};
use ssx_formats::save::chunks::StatRecord;
use ssx_lua::game::objectives::StatValue;
use ssx_physics::stats::{PERSISTENT, Record, StatsStore};

fn word(v: StatValue) -> (u32, u32) {
    match v {
        StatValue::Int(i) => (i, 0),
        StatValue::Float(f) => (f.to_bits(), 1),
    }
}

fn value(word: u32, kind: u32) -> StatValue {
    if kind == 1 { StatValue::Float(f32::from_bits(word)) } else { StatValue::Int(word) }
}

pub fn register(profile: &mut Profile) {
    for (&key, r) in &StatsStore::default().maps[PERSISTENT] {
        let (v, kind) = word(r.value);
        let (initial, _) = word(r.initial);
        profile.stats.records.entry(key).or_insert(StatRecord {
            key,
            value: v,
            key_again: key,
            scope: 2,
            kind,
            initial,
        });
    }
}

pub fn into_session(store: &mut StatsStore, profile: &Profile, names: &BTreeMap<u32, String>) {
    for r in profile.persistent_records() {
        let rec = Record { value: value(r.value, r.kind), initial: value(r.initial, r.kind) };
        store.maps[PERSISTENT].insert(r.key, rec);
        if let Some(n) = names.get(&r.key) {
            store.names.entry(r.key).or_insert_with(|| n.clone());
        }
    }
    for (&k, _) in profile.stats.awards.iter().filter(|(k, _)| profile.stats.awarded(**k)) {
        store.awarded[2].insert(k);
    }
}

fn take_records(profile: &mut Profile, store: &StatsStore, modelled: &BTreeMap<u32, String>) {
    for (&key, r) in &store.maps[PERSISTENT] {
        if modelled.contains_key(&key) {
            continue;
        }
        let (v, kind) = word(r.value);
        let (initial, _) = word(r.initial);
        profile.stats.records.insert(key, StatRecord { key, value: v, key_again: key, scope: 2, kind, initial });
    }
}

pub fn from_session<'a>(
    profile: &mut Profile,
    store: &StatsStore,
    modelled: &BTreeMap<u32, String>,
    rules: &'a Rules,
) -> Vec<&'a BadgeText> {
    take_records(profile, store, modelled);
    take_added_tricks(profile, store);
    let mut new = Vec::new();
    for k in &store.awarded[2] {
        if let Some(b) = rules.badge_texts.get(k)
            && profile.award_badge(*k)
        {
            new.push(b);
        }
    }
    new
}

pub const ADDED_TRICK_FIELDS: [&str; 3] = ["performed", "uber_performed", "super_uber_performed"];

fn take_added_tricks(profile: &mut Profile, store: &StatsStore) {
    for (id, counts) in &store.added_tricks {
        let Ok(id) = id.parse::<ssx_content::ContentId>() else { continue };
        for (field, &n) in ADDED_TRICK_FIELDS.iter().zip(counts).filter(|(_, n)| **n > 0) {
            let had = profile.mod_state.int(&id, field).unwrap_or(0);
            let value = ssx_content::profile_state::StateValue::Int(had.saturating_add(i64::from(n)));
            profile.mod_state.set(&id, field, value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_registration_keeps_a_loaded_profiles_values() {
        let mut p = Profile::new();
        let air = ssx_formats::attrib::fnv1(b"NumAirTime");
        let grind = ssx_formats::attrib::fnv1(b"TrickNumPerformed_Grind_1");
        let rider = ssx_formats::attrib::fnv1(b"TrickSignaturePerformed_Zoe");
        let saved = StatRecord { key: air, value: 12.5f32.to_bits(), key_again: air, scope: 2, kind: 1, initial: 0 };
        p.stats.records.insert(air, saved);
        p.stats
            .records
            .insert(rider, StatRecord { key: rider, value: 3, key_again: rider, scope: 2, kind: 0, initial: 0 });
        register(&mut p);
        assert_eq!(p.stats.records[&air], saved, "the saved value wins");
        assert_eq!(p.stats.records[&rider].value, 3);
        assert_eq!(p.stats.records[&grind].value, 0, "a stat the save did not hold joins at 0");
        let n = p.stats.records.len();
        register(&mut p);
        assert_eq!(p.stats.records.len(), n);
    }

    #[test]
    fn a_ride_stat_and_badge_reach_the_profile_and_come_back() {
        let mut p = Profile::new();
        register(&mut p);
        let n = p.stats.records.len();
        assert!(n > 40, "the registered persistent stats ({n})");
        let mut rules = Rules::default();
        let badge = ssx_formats::attrib::fnv1(b"Badge_1");
        rules.badge_texts.insert(badge, BadgeText { name: "Badge_1".into(), ..Default::default() });
        let mut store = StatsStore::default();
        into_session(&mut store, &p, &BTreeMap::new());
        store.add_int_key(ssx_formats::attrib::fnv1(b"NumWipeouts"), PERSISTENT, 3, false);
        store.award("Badge_1", 2);
        store.award("Undefined", 2);
        let new = from_session(&mut p, &store, &BTreeMap::new(), &rules);
        assert_eq!(new.len(), 1);
        assert_eq!(p.recent_badges, vec![badge]);
        assert_eq!(p.stats.awards.get(&badge), Some(&(1 << 24)));
        assert_eq!(p.stats.records[&ssx_formats::attrib::fnv1(b"NumWipeouts")].value, 3);
        let mut again = StatsStore::default();
        into_session(&mut again, &p, &BTreeMap::new());
        assert!(again.awarded(badge, 2));
        assert!(from_session(&mut p, &again, &BTreeMap::new(), &rules).is_empty());
        assert_eq!(p.stats.records.len(), n);
    }

    #[test]
    fn a_rides_added_tricks_are_added_to_the_profiles_mod_state() {
        let mut p = Profile::new();
        let id: ssx_content::ContentId = "p:trick/air_walk".parse().expect("an id");
        let mut store = StatsStore::default();
        store.added_tricks.insert(id.to_string().into(), [2, 1, 0]);
        take_added_tricks(&mut p, &store);
        take_added_tricks(&mut p, &store);
        let counts: Vec<Option<i64>> = ADDED_TRICK_FIELDS.iter().map(|f| p.mod_state.int(&id, f)).collect();
        assert_eq!(counts, [Some(4), Some(2), None]);
        p.mod_state.set(&id, "performed", ssx_content::profile_state::StateValue::Int(i64::MAX - 1));
        take_added_tricks(&mut p, &store);
        assert_eq!(p.mod_state.int(&id, "performed"), Some(i64::MAX), "saturating");
    }
}
