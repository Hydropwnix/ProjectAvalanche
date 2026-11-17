use super::*;
use crate::levelstart::LevelStart;
use crate::tricks::TrickType;

const LEFT: usize = 4;
const LEFT_RIGHT: usize = 5;
const RIGHT: usize = 8;

fn in_level_start(height: f32) -> (World, RiderState) {
    let w = world(plane(500.0, 0.2));
    let mut s = RiderState::spawn(&w, Vec3::new(0.0, 1.0, 0.0), Vec3::Z).expect("ground");
    let ls = LevelStart::new(&w, s.traj, Quat::IDENTITY).expect("ground");
    s.traj += Vec3::Y * height;
    (s.traj_cached, s.traj_cached_prev) = (s.traj, s.traj);
    (s.start, s.gameplay, s.track_started, s.trigger_go) = (Some(ls), None, false, false);
    (w, s)
}

fn buttons(x: bool, b: bool) -> TrickInput {
    TrickInput { x, b, ..TrickInput::default() }
}

fn x_then_b(b_updates: usize) -> Vec<TrickInput> {
    let mut v = vec![buttons(false, false)];
    v.extend(std::iter::repeat_n(buttons(true, false), 4));
    v.extend(std::iter::repeat_n(buttons(false, false), 3));
    v.extend(std::iter::repeat_n(buttons(false, true), b_updates));
    v
}

#[test]
fn the_level_start_reads_the_game_states_default() {
    let (_, s) = in_level_start(0.0);
    assert!(!s.in_air() && s.bln_in_air(), "no SF_Ride: the BoolAsset's default, true");
    let (_, mut s) = in_level_start(0.0);
    s.start = None;
    assert!(!s.bln_in_air());
    let (_, mut n) = in_level_start(0.0);
    n.carry_game_state(&s);
    assert!(!n.bln_in_air());
    n.carry_game_state(&in_level_start(0.0).1);
    assert!(n.bln_in_air());
}

#[test]
fn a_button_double_in_the_level_start_is_matched() {
    let (w, mut s) = in_level_start(0.0);
    let mut rows = Vec::new();
    for inp in x_then_b(3) {
        step_with(&mut s, &w, &Input::default(), &inp, DT_TICKS);
        assert!(s.start.is_some(), "still in the level start");
        let (g, t) = (&s.tricks.gestures.button, &s.tricks);
        rows.push((g[LEFT], g[LEFT_RIGHT], g[RIGHT], t.valid_up_start, t.valid_left_start, t.valid_right_start));
    }
    let (n, x) = ((false, false, false, true, true, true), (true, false, false, true, true, true));
    let lr = (false, true, true, true, true, false);
    assert_eq!(rows, [vec![n], vec![x; 4], vec![n; 3], vec![lr; 3]].concat());
}

#[test]
fn the_double_from_the_level_start_is_the_trick_after_the_hand_over() {
    let (w, mut s) = in_level_start(30.0);
    let mut seen = Vec::new();
    for (i, inp) in x_then_b(40).iter().enumerate() {
        if i == 10 {
            s.start.as_mut().expect("level start").request_stop = true;
        }
        step_with(&mut s, &w, &Input::default(), inp, DT_TICKS);
        assert_eq!(s.start.is_some(), i < 10, "update {i}");
        if seen.last() != Some(&s.tricks.trick_type) {
            seen.push(s.tricks.trick_type);
        }
    }
    assert_eq!(s.state, RideState::Airborne);
    assert_eq!(seen, [TrickType::None, TrickType::Mute], "{seen:?}");
}

#[test]
fn the_level_start_speed_is_the_trajectorys() {
    let (w, mut s) = in_level_start(30.0);
    (s.traj_velocity, s.velocity) = (Vec3::new(0.0, -0.1, 0.1), Vec3::ZERO);
    step_with(&mut s, &w, &Input::default(), &TrickInput::default(), DT_TICKS);
    assert!(s.start.is_some());
    assert!((s.player_speed - 0.02_f32.sqrt() * 60.0).abs() < 1e-4, "{}", s.player_speed);
}

#[test]
fn a_held_through_the_level_start_does_not_prewind() {
    let (w, mut s) = in_level_start(0.0);
    let a = Input { jump: true, ..Input::default() };
    for i in 0..20 {
        step_with(&mut s, &w, &a, &TrickInput::default(), DT_TICKS);
        assert!(s.start.is_some(), "update {i}: still in the level start");
        assert_eq!((s.prewind, s.prewind_analog), (0.0, 0.0), "update {i}");
    }
}

#[test]
fn the_level_start_holds_the_last_safe_ground() {
    let (w, mut s) = in_level_start(0.0);
    let held = Vec3::new(1.0, 2.0, 3.0);
    s.last_safe_ground = held;
    for i in 0..20 {
        step_with(&mut s, &w, &Input::default(), &TrickInput::default(), DT_TICKS);
        assert!(s.start.is_some(), "update {i}");
        assert_eq!(s.last_safe_ground, held, "update {i}");
    }
}

#[test]
fn the_nis_holds_the_grinds_air_ticks() {
    let (w, mut s) = in_level_start(30.0);
    for i in 0..10 {
        step_with(&mut s, &w, &Input::default(), &TrickInput::default(), DT_TICKS);
        assert!(s.start.is_some() && s.bln_in_air(), "update {i}");
        assert_eq!(s.grind.ticks_airborne, 0.0, "update {i}");
        assert!(!s.grind.is_grinding() && !s.grind.steering.active, "update {i}");
    }
}

#[test]
fn the_level_starts_ballistic_ray_is_the_airs() {
    let (w, mut s) = in_level_start(30.0);
    step_with(&mut s, &w, &Input::default(), &TrickInput::default(), DT_TICKS);
    assert!(s.start.is_some());
    let air = w.ballistic_probe(s.traj, s.traj_velocity, s.grind.wind, true, DT_TICKS);
    let ground = w.ballistic_probe(s.traj, s.traj_velocity, s.grind.wind, false, DT_TICKS);
    assert_ne!(air, ground, "the test world's air and ground models differ");
    assert_eq!(s.ballistic, air);
}
