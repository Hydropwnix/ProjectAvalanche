use super::*;
use crate::jump::PrewindGestures;
use crate::tricks::PadSample;

fn stick(angle: f32, mag: f32) -> PadSample {
    PadSample { angle, mag, ..PadSample::default() }
}

#[test]
fn the_prewind_mappings_are_the_right_sticks_lower_hold_and_upper_enter() {
    let down = 3.0 * std::f32::consts::FRAC_PI_2;
    let up = std::f32::consts::FRAC_PI_2;
    let mut g = PrewindGestures::default();
    g.tick(stick(0.0, 0.0));
    assert!(!g.hold && !g.release);
    g.tick(stick(down, 0.9));
    assert!(g.hold && !g.release, "pulled down: the crouch");
    g.tick(stick(down, 0.9));
    assert!(g.hold && !g.release, "held: no release (it is an enter edge)");
    g.tick(stick(up, 0.8));
    assert!(!g.hold && g.release, "flicked up: the release");
    g.tick(stick(up, 0.8));
    assert!(!g.hold && !g.release, "one step only");
    g.tick(stick(down, 0.84));
    assert!(!g.hold);
    g.tick(stick(0.0, 0.0));
    g.tick(stick(up, 0.74));
    assert!(!g.release);
    g.tick(stick(0.0, 0.6));
    assert!(!g.hold && !g.release, "a weak sideways stick is neither");
}

#[test]
fn the_right_stick_flick_jumps_under_the_standard_scheme_only() {
    let w = world(plane(4000.0, 0.0));
    let mut standard = moving(&w, Vec3::new(0.0, 1.0, -3000.0), Vec3::Z, 60.0 / KMH_PER_M_PER_TICK);
    standard.jump.on_ground_ticks = 20.0;
    let down = TrickInput { right_stick: Vec2::new(0.0, -1.0), ..TrickInput::default() };
    let up = TrickInput { right_stick: Vec2::new(0.0, 1.0), ..TrickInput::default() };
    for _ in 0..10 {
        step_with(&mut standard, &w, &Input::default(), &down, DT_TICKS);
    }
    assert!(standard.prewind > 0.5, "the crouch builds FLT_Prewind: {}", standard.prewind);
    assert!(standard.jump.compression > 0.5, "and the compression: {}", standard.jump.compression);
    assert_eq!(standard.state, RideState::Riding);
    step_with(&mut standard, &w, &Input::default(), &up, DT_TICKS);
    assert_eq!(standard.state, RideState::Jump, "the up flick requests the jump");

    let mut classic = moving(&w, Vec3::new(0.0, 1.0, -2000.0), Vec3::Z, 60.0 / KMH_PER_M_PER_TICK);
    classic.tricks.classic = true;
    classic.jump.on_ground_ticks = 20.0;
    for _ in 0..10 {
        step_with(&mut classic, &w, &Input::default(), &down, DT_TICKS);
    }
    step_with(&mut classic, &w, &Input::default(), &up, DT_TICKS);
    step_with(&mut classic, &w, &Input::default(), &up, DT_TICKS);
    assert!(classic.state != RideState::Jump, "the classic scheme's up flick is its nose press: {:?}", classic.state);
}
