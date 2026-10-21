use glam::Vec3;

use super::*;
use crate::testutil::{plane, world};
use crate::triggers::{IG_DEATH, TriggerScope, TriggerVolume, VolumeShape};

#[test]
fn a_death_volume_kills_and_the_timer_ends_the_event() {
    let mut w = world(plane(400.0, 0.3));
    w.triggers.volumes.push(TriggerVolume {
        name: "death".into(),
        type_text: "death".into(),
        groups: IG_DEATH,
        interacts: crate::triggers::interaction_mask(IG_DEATH),
        shape: VolumeShape::Box { half_extents: Vec3::new(50.0, 40.0, 5.0) },
        centre: Vec3::new(0.0, 0.0, 30.0),
        rotation: glam::Quat::IDENTITY,
        object: 0,
        instance: 0,
        placement: 0,
        scope: TriggerScope::Track,
    });
    let src = br#"BeginObjective("PlayerDeathVolumeTest")
            WAIT_TriggerVolumeByGroup(IG_DEATH)
            DO_SetGameStateValue("Player1", BLN_TriggerDeathByDeathPlane, true)
            DO_RestartObjective()
        EndObjective()"#;
    let mut d = crate::objectives::DropScripts { list: vec!["t.lua".into()], ..Default::default() };
    d.sources.insert("t.lua".into(), std::sync::Arc::from(&src[..]));
    w.survival.objectives = Some(std::sync::Arc::new(d));
    let mut s = RiderState::spawn(&w, Vec3::new(0.0, 50.0, 0.0), Vec3::Z).expect("ground");
    s.velocity = Vec3::Z * 0.4;
    let mut x = Session::default();
    x.event.on_trigger_race_start(&mut x.timer);
    let input = Input { tuck: 1.0, ..Input::default() };
    let (mut died, mut ended, mut latched) = (None, None, None);
    for i in 0..400 {
        let e = x.execute(&mut s, &w, &input, &TrickInput::default(), 0.0, &RewindPad::default(), None);
        if e.rider_dead.is_some() && died.is_none() {
            assert!(e.was_alive && !s.death.is_alive() && x.timer.hint && x.timer.remaining <= 4.0);
            died = Some(i);
        }
        if latched.is_none() && s.carving.is_dead {
            latched = Some(i);
        }
        if e.timer == DeathTimerEvent::EventEnds {
            ended = Some(i);
            break;
        }
    }
    let (died, latched, ended) = (died.expect("died"), latched.expect("IsDead"), ended.expect("event end"));
    assert!(latched == died, "the carving latches IsDead in the update that read ¬IsAlive");
    assert!((119..=121).contains(&(ended - died)), "4 s of 30 Hz executions: {died} → {ended}");
    assert!(x.event_ended && s.death.dead_by_death_plane);
    let events = &x.objectives.as_ref().expect("objectives").runner.events;
    assert!(!events.global(ssx_lua::game::events::RACE_ENDED), "no RaceEnded on the death timer's end");
    assert!(x.pads_released);
    let (mut a, mut b) = ((x.clone(), s.clone()), (x.clone(), s.clone()));
    let pushed = Input { steer: 1.0, tuck: 1.0, jump: true, ..Input::default() };
    let tricks = TrickInput { y: true, tweak: true, ..TrickInput::default() };
    let rewind = RewindPad { rewind_held: true, takeover: false, flip_accel: 1.0 };
    for _ in 0..60 {
        a.0.execute(&mut a.1, &w, &pushed, &tricks, 1.0, &rewind, None);
        b.0.execute(&mut b.1, &w, &Input::default(), &TrickInput::default(), 0.0, &RewindPad::default(), None);
    }
    assert_eq!((a.1.position, a.1.traj, a.1.velocity), (b.1.position, b.1.traj, b.1.velocity));
    assert!(!a.0.rewinding() && a.0.buffer.playback().is_none(), "no rewind after the end");
    x.reset();
    assert!(!x.pads_released, "a restart releases the latch");
}

#[test]
fn a_trigger_entity_drives_weather_flags_and_the_boundary_force() {
    use crate::triggers::{IG_TRIGGER, TriggerEntity};
    const SCRIPT: &str = r#"
local s = CreateStateSet( "T" )
local st = s:AddState( "Wait" )
local tr = st:AddTransition( "In" )
tr:AddCondition( TriggerVolumeCondition( "", "Player1", "Touch" ) )
st = s:AddState( "In" )
st:AddUpdateTrack( PrecipitationTrack( 0.5 ) )
st:AddUpdateTrack( OutOfBoundsTrack( "Player1", 0.2, 2.0 ) )
st:AddEnterTrack( SetNamedFlagTrack( "inside", 1, TEMPORARY ) )
tr = st:AddTransition( "Wait" )
tr:AddCondition( NOT( TriggerVolumeCondition( "", "Player1", "Touch" ) ) )
"#;
    let mut w = world(plane(400.0, 0.3));
    let (object, instance) = (7, 9);
    w.triggers.volumes.push(TriggerVolume {
        name: "box".into(),
        type_text: "trigger".into(),
        groups: IG_TRIGGER,
        interacts: crate::triggers::interaction_mask(IG_TRIGGER),
        shape: VolumeShape::Box { half_extents: Vec3::new(50.0, 200.0, 4.0) },
        centre: Vec3::new(0.0, 0.0, 12.0),
        rotation: glam::Quat::IDENTITY,
        object,
        instance,
        placement: 0,
        scope: TriggerScope::Track,
    });
    let rotation = glam::Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
    w.triggers.entities.push(TriggerEntity {
        object,
        instance,
        placement: 0,
        scope: TriggerScope::Track,
        position: Vec3::new(0.0, 0.0, 12.0),
        rotation,
    });
    w.triggers.scripts.insert(object, ("scripts\\t.lua".into(), std::sync::Arc::from(SCRIPT)));
    let mut d = crate::objectives::DropScripts { list: vec!["o.lua".into()], ..Default::default() };
    d.sources.insert("o.lua".into(), std::sync::Arc::from(&b"BeginObjective(\"X\") EndObjective()"[..]));
    w.survival.objectives = Some(std::sync::Arc::new(d));
    let mut s = RiderState::spawn(&w, Vec3::new(0.0, 50.0, 0.0), Vec3::Z).expect("ground");
    s.velocity = Vec3::Z * 0.4;
    let mut x = Session::default();
    let input = Input { tuck: 1.0, ..Input::default() };
    let (mut inside, mut forced, mut left) = (false, false, false);
    for _ in 0..200 {
        x.execute(&mut s, &w, &input, &TrickInput::default(), 0.0, &RewindPad::default(), None);
        let c = x.controllers.as_ref().expect("controllers");
        let flag = x.objectives.as_ref().is_some_and(|o| o.runner.flags.check(&crate::objectives::Name::new("inside")));
        if c.weather.targets().0 == 0.5 {
            inside = true;
            assert!(flag, "the enter track set the flag");
            forced |=
                s.boundary.force_strength == 2.0 && (s.boundary.force_direction - rotation * Vec3::Z).length() < 1e-6;
        } else if inside {
            left = true;
            break;
        }
    }
    assert!(inside && forced && left, "inside {inside}, forced {forced}, left {left}");
    assert_eq!(x.controllers.as_ref().unwrap().weather.targets().0, 0.0, "popped back to the default");
}

#[test]
fn the_profiles_named_flags_seed_the_store_and_come_back() {
    use crate::objectives::{LevelObjectives, Name};
    let (seen, hint, temp) = (Name::new("HintSeen"), Name::new("HintFromScript"), Name::new("Temp"));
    let mut s = Session::default();
    s.seed_named_flags([seen.hash, 7]);
    s.set_named_flag(7, false);
    assert_eq!(s.permanent_named_flags(), [seen.hash]);
    let script = br#"
BeginObjective("A")
	WAIT_CheckNamedFlag("HintSeen")
	DO_SetPermanentNamedFlag("HintFromScript", true)
	DO_SetTemporaryNamedFlag("Temp", true)
EndObjective()
"#;
    let batch = [("t.lua".to_string(), script.to_vec())];
    let seed: Vec<u32> = s.named_flag_seed.iter().copied().collect();
    let mut o = LevelObjectives::load(&[], &batch, crate::objectives::LoadWorld::default(), false, None, &seed);
    assert!(o.report.problems.is_empty(), "{:?}", o.report.problems);
    let mut load = crate::objectives::LoadWorld::default();
    o.runner.update(0.1, &mut load);
    s.objectives = Some(o);
    let mut want = vec![seen.hash, hint.hash];
    want.sort_unstable();
    assert_eq!(s.permanent_named_flags(), want, "the seeded flag let the objective pass");
    let flags = &s.objectives.as_ref().expect("objectives").runner.flags;
    assert!(flags.check(&temp) && !want.contains(&temp.hash));
    s.set_named_flag(seen.hash, false);
    s.reset();
    assert_eq!(s.permanent_named_flags(), [hint.hash], "a restart keeps the store");
}

#[test]
fn badge_progress_is_kept() {
    let mut stats = crate::stats::StatsStore::default();
    stats.badge_progress.insert(42, 0.25);
    for scope in [crate::stats::TRACK, crate::stats::TRICK, crate::stats::PERSISTENT] {
        stats.reset(scope);
    }
    assert_eq!(stats.badge_progress.get(&42), Some(&0.25));
}
