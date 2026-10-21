use super::*;

fn f(s: &StatsStore, n: &str, sc: usize) -> f32 {
    match s.value(n, sc) {
        Some(StatValue::Float(x)) => x,
        v => panic!("{n}: {v:?}"),
    }
}

#[test]
fn the_tracker_keeps_the_longest_streak_and_counts_edges() {
    let mut s = StatsStore::default();
    s.level_start(&[false; 15], Some(6));
    let dt = 0.1;
    let fast = PlayerStats { speed: 30.0, boosting: true, ..Default::default() };
    let slow = PlayerStats { speed: 20.0, ..Default::default() };
    for _ in 0..5 {
        s.update(dt, 0, false, Some(6), &fast);
    }
    s.update(dt, 0, false, Some(6), &slow);
    for _ in 0..3 {
        s.update(dt, 1000, false, Some(6), &fast);
    }
    assert!((f(&s, "TimeExceedingTargetSpeed1", TRACK) - 0.5).abs() < 1e-5);
    assert!((f(&s, "MaxSpeed_Track", TRACK) - 30.0).abs() < 1e-6);
    assert_eq!(s.value("NumTimesUsedBoost_Track", TRACK), Some(StatValue::Int(2)));
    assert_eq!(s.value("NumTimesCompetedRace_Track", TRACK), Some(StatValue::Int(1)));
    assert!((f(&s, "Time_Track", TRACK) - 1.0).abs() < 1e-6);
    s.update(dt, 1100, true, Some(6), &fast);
    s.update(dt, 1200, true, Some(6), &fast);
    assert_eq!(s.value("NumRewindsUsed_Track", TRACK), Some(StatValue::Int(1)));
    assert!((f(&s, "NumTimeBoosting_Track", TRACK) - 0.8).abs() < 1e-5, "paused while rewinding");
}

#[test]
fn a_trick_sets_its_scope_and_the_awards_follow_the_track() {
    let mut s = StatsStore::default();
    let m = BanksFlushed { air_time: 90.0, flip_total: 7.0, spin_total: 6.4, ..Default::default() };
    let r = TrickRecord::default();
    s.banks_flushed(&m, &r);
    s.banks_flushed(&BanksFlushed { air_time: 30.0, ..Default::default() }, &r);
    assert!((f(&s, "NumAirTime_Trick", TRICK) - 0.5).abs() < 1e-6);
    assert!((f(&s, "NumAirTime_Track", TRACK) - 2.0).abs() < 1e-6);
    assert_eq!(s.value("NumFlips_Track", TRACK), Some(StatValue::Int(1)));
    assert!((f(&s, "NumFlipDegrees_Track", TRACK) - 360.0).abs() < 1e-6);
    s.award("TrackBonus_8", 0);
    assert!(s.awarded(fnv1(b"TrackBonus_8"), 0) && s.track_bonuses() == ["TrackBonus_8"]);
    s.writeable = false;
    s.banks_flushed(&m, &r);
    assert!((f(&s, "NumAirTime_Track", TRACK) - 2.0).abs() < 1e-6);
    s.writeable = true;
    s.level_start(&[false; 15], None);
    assert!(s.track_bonuses().is_empty() && f(&s, "NumAirTime_Track", TRACK) == 0.0);
    assert!(f(&s, "NumAirTime", PERSISTENT) > 0.0, "the persistent scope stays");
}

#[test]
fn every_trick_family_is_registered_and_a_grind_counts() {
    use crate::tricks::TrickType;
    use ssx_lua::game::Binder;
    use ssx_lua::game::bindings::CONSTANTS;
    for (i, name) in TRICK_NAMES.iter().enumerate() {
        let v = CONSTANTS.iter().find(|(b, n, _)| *b == Binder::Objectives && n == name).map(|c| c.2);
        assert_eq!(v, Some(i as f64), "{name}");
    }
    assert_eq!((trick_name(TrickType::Grind1), trick_name(TrickType::Grind6SUber)), ("Grind_1", "Grind_6_SUber"));
    assert_eq!(trick_name(TrickType::None), "None");
    let mut s = StatsStore::default();
    for family in TRICK_FAMILIES {
        for trick in TRICK_NAMES {
            let n = format!("{family}{trick}");
            assert_eq!(s.value(&n, PERSISTENT), Some(StatValue::Int(0)), "{n}");
            assert_eq!((s.value(&n, TRACK), s.value(&n, TRICK)), (None, None), "{n}");
        }
    }
    let flush = |s: &mut StatsStore, kind, trick_id, awesome, super_awesome| {
        let r = TrickRecord { kind, trick_id, awesome, super_awesome, ..Default::default() };
        s.banks_flushed(&BanksFlushed::default(), &r);
    };
    flush(&mut s, RecordKind::Grind, TrickType::Grind1, false, false);
    flush(&mut s, RecordKind::Grind, TrickType::Grind2Uber, true, false);
    flush(&mut s, RecordKind::Grind, TrickType::Grind3SUber, false, true);
    flush(&mut s, RecordKind::Air, TrickType::Indy, false, false);
    flush(&mut s, RecordKind::Air, TrickType::Indy, true, false);
    flush(&mut s, RecordKind::Air, TrickType::Mute, false, true);
    flush(&mut s, RecordKind::Interaction, TrickType::None, false, false);
    for (n, v) in [
        ("TrickNumPerformed_Grind_1", 1),
        ("TrickNumPerformed_Grind_2_Uber", 1),
        ("TrickNumPerformed_Grind_3_SUber", 1),
        ("TrickNumUberPerformed_Grind_2_Uber", 0),
        ("TrickNumSuperUberPerformed_Grind_3_SUber", 0),
        ("TrickNumPerformed_Grab_Indy", 1),
        ("TrickNumUberPerformed_Grab_Indy", 1),
        ("TrickNumSuperUberPerformed_Grab_Mute", 1),
        ("TrickNumPerformed_None", 1),
    ] {
        assert_eq!(s.value(n, PERSISTENT), Some(StatValue::Int(v)), "{n}");
    }
    assert_eq!(s.value("NumUberTricksPerformed_Track", TRACK), Some(StatValue::Int(1)));
    assert_eq!(s.value("NumSuperUberTricksPerformed_Track", TRACK), Some(StatValue::Int(1)));
}

#[test]
fn the_message_writers() {
    use ssx_lua::game::objectives::{Name, StatKey};
    let mut s = StatsStore::default();
    s.level_start(&[false; 15], Some(1));
    s.track_ended(&crate::scoring::TrackEnded { tricky: true, uber_tricky: false, wingsuit: true, boosting: false });
    assert_eq!(s.value("CompletedTrackWhileTricky_Track", TRACK), Some(StatValue::Int(1)));
    assert_eq!(s.value("CompletedTrackWhileUberTricky_Track", TRACK), Some(StatValue::Int(0)));
    assert_eq!(s.value("CompletedTrackWhileWingsuitDeployed_Track", TRACK), Some(StatValue::Int(1)));
    s.base_points_awarded(250.0, Some(1));
    s.base_points_awarded(250.0, Some(6));
    assert!((f(&s, "ComboScoreTrickEvent_Track", TRACK) - 250.0).abs() < 1e-6);
    assert!((f(&s, "ComboScoreTrickEvent", PERSISTENT) - 250.0).abs() < 1e-6);
    let racer = |w, a| Some(RacerStats { trigger_wipeout: w, in_air: a });
    let mut p = PlayerStats { avalanche_mass: 60.0, ..Default::default() };
    for (w, a) in [(true, true), (true, false), (true, false), (false, false), (true, false)] {
        p.racers[0] = racer(w, a);
        s.update(0.1, 0, false, Some(1), &p);
        p.avalanche_mass = 10.0;
    }
    assert_eq!(s.value("NumCharactersTakenDown_Track", TRACK), Some(StatValue::Int(2)));
    assert!((f(&s, "MaxAvalancheMass_Track", TRACK) - 60.0).abs() < 1e-6);
    s.rewound_out_of_death();
    assert_eq!(s.value("NumRewindsOutOfDeath", PERSISTENT), Some(StatValue::Int(1)));
    s.rider = Some("Alex".into());
    let sig =
        TrickRecord { kind: RecordKind::Air, trick_id: crate::tricks::TrickType::Signature, ..Default::default() };
    s.banks_flushed(&BanksFlushed::default(), &sig);
    s.banks_flushed(&BanksFlushed::default(), &sig);
    assert_eq!(s.value("NumUniqueSignatureTricksPerformed", PERSISTENT), Some(StatValue::Int(1)));
    assert_eq!(s.value("TrickSignaturePerformed_Alex", PERSISTENT), Some(StatValue::Int(2)));
    assert_eq!(s.value("NumSignatureTricksPerformed_Track", TRACK), Some(StatValue::Int(2)));
    let key = StatKey {
        name: Name::new("MedalBronzeNumEarned_"),
        scope: PERSISTENT as i32,
        location: Some(Name::new("exp_alps_vorassay_drop_3")),
    };
    let h = fnv1(format!("MedalBronzeNumEarned_{}", fnv1(b"exp_alps_vorassay_drop_3")).as_bytes());
    assert_eq!(key.key(), h);
    assert_eq!(s.get(&key), None);
    s.add_int_key(h, PERSISTENT, 1, true);
    assert_eq!(s.get(&key), Some(StatValue::Int(1)));
}
