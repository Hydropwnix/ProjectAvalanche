use std::f32::consts::{FRAC_PI_2, PI};

use super::*;
use crate::actuator::off_multiple;
use crate::testutil::{aerial, world};

const DEG: f32 = PI / 180.0;

fn props() -> ResolvedCharacterProperties {
    world(crate::testutil::plane(10.0, 0.0)).props
}

fn level_board() -> Quat {
    frame(Vec3::Z, Vec3::Y)
}

fn fly(mut a: AirRotation, ticks: f32, stick: impl Fn(f32) -> Vec2) -> (AirRotation, Quat, f32) {
    let (t, r) = (aerial(), props());
    let v = Vec3::Z * 0.3;
    let mut board = a.orientation();
    let mut goofy = false;
    let (mut elapsed, mut turned) = (0.0, 0.0);
    while elapsed < ticks {
        a.prediction = Some(LandingPrediction { ticks: ticks - elapsed, normal: Vec3::Y });
        let inp = AirInputs {
            stick: stick(elapsed),
            lstick: Vec2::ZERO,
            classic: false,
            velocity: v,
            allow_stance_switch: true,
            grind_steering: None,
            tutorial_mode: false,
            heli_interact: None,
        };
        let before = yaw(board);
        board = a.step(&t, &r, &inp, board, &mut goofy, 2.0);
        turned += crate::actuator::wrap_angle(yaw(board) - before);
        elapsed += 2.0;
    }
    (a, board, turned)
}

fn yaw(b: Quat) -> f32 {
    let f = b * Vec3::X;
    (-f.x).atan2(f.z)
}

fn inputs(board: Quat, omega: Vec3, prewind: f32, trigger: bool, ttl: Option<f32>) -> TakeoffInputs {
    TakeoffInputs {
        board,
        wf: Quat::from_scaled_axis(omega),
        velocity: Vec3::Z * 0.3,
        ground_normal: Vec3::Y,
        prewind,
        trigger,
        prediction: ttl.map(|ticks| LandingPrediction { ticks, normal: Vec3::Y }),
        intercept: None,
    }
}

fn takeoff(kind: Takeoff, board: Quat, omega: Vec3, stick: Vec2, prewind: f32, ttl: Option<f32>) -> AirRotation {
    let (kind, trigger) = if kind == Takeoff::AirJump { (Takeoff::RollOff, true) } else { (kind, false) };
    AirRotation::takeoff(&aerial(), &props(), kind, stick, &inputs(board, omega, prewind, trigger, ttl))
}

#[test]
fn compose_and_decompose_round_trip() {
    for (psi, theta, phi) in [(0.0, 0.3, -1.2), (1.0, -2.5, 3.0), (4.0, 1.5, 0.1), (0.0, PI - 0.01, FRAC_PI_2)] {
        let res = Quat::from_rotation_x(0.05);
        let q = compose(psi, theta, res, phi);
        let (t2, p2, r2) = decompose(q, psi);
        assert!(compose(psi, t2, r2, p2).angle_between(q) < 2e-3, "{psi} {theta} {phi}");
        assert!((t2 - theta).abs() < 1e-3 && (p2 - phi).abs() < 1e-3, "{t2} {p2}");
    }
    let a = takeoff(Takeoff::Jump, level_board(), Vec3::ZERO, Vec2::ZERO, 0.0, None);
    assert!(a.flip.angle.abs() < 1e-5 && a.spin.angle.abs() < 1e-5);
    assert!(a.orientation().angle_between(level_board()) < 1e-4);
}

#[test]
fn takeoff_prewind_launch_and_caps() {
    let a = takeoff(Takeoff::Jump, level_board(), Vec3::ZERO, Vec2::new(1.0, 0.0), 1.0, None);
    assert!((a.spin.vel - 15.0 * DEG).abs() < 1e-6, "{}", a.spin.vel / DEG);
    let a = takeoff(Takeoff::Jump, level_board(), Vec3::ZERO, Vec2::new(0.5, 1.0), 0.6, None);
    assert!((a.spin.vel - 15.0 * DEG * 0.3).abs() < 1e-6 && (a.flip.vel - 13.0 * DEG * 0.6).abs() < 1e-6);
    let a = takeoff(Takeoff::RollOff, level_board(), Vec3::ZERO, Vec2::new(1.0, 0.0), 1.0, None);
    assert_eq!(a.spin.vel, 0.0);
    let a = takeoff(Takeoff::AirJump, level_board(), Vec3::ZERO, Vec2::new(-1.0, 0.0), 1.0, None);
    assert!((a.spin.vel + 15.0 * DEG).abs() < 1e-6);
    let yaw_right = -Vec3::Y * (5.0 * DEG);
    let a = takeoff(Takeoff::Jump, level_board(), yaw_right, Vec2::new(1.0, 0.0), 1.0, None);
    assert!((a.spin.vel - 15.0 * DEG).abs() < 1e-6, "capped: {}", a.spin.vel / DEG);
    let a = takeoff(Takeoff::Jump, level_board(), yaw_right, Vec2::ZERO, 0.0, None);
    assert!((a.spin.vel - 5.0 * DEG).abs() < 1e-5, "{}", a.spin.vel / DEG);
    let a = takeoff(Takeoff::RollOff, level_board(), Vec3::ZERO, Vec2::ZERO, 0.0, Some(50.0));
    assert_eq!(a.flip.stick_scale, 0.0);
    let a = takeoff(Takeoff::RollOff, level_board(), Vec3::ZERO, Vec2::ZERO, 0.0, Some(90.0));
    assert!((a.flip.stick_scale - 3.0 * DEG).abs() < 1e-6);
    let a = takeoff(Takeoff::Jump, level_board(), Vec3::ZERO, Vec2::ZERO, 0.0, Some(50.0));
    assert!((a.flip.stick_scale - 6.0 * DEG).abs() < 1e-6, "aerial_jump has no strength curve");
}

#[test]
fn leaning_takeoff_rolls_by_stick_x() {
    let n = Quat::from_axis_angle(Vec3::Z, 60.0 * DEG) * Vec3::Y;
    let board = frame(Vec3::Z, n);
    let (t, r) = (aerial(), props());
    let i = TakeoffInputs { ground_normal: n, ..inputs(board, Vec3::ZERO, 0.0, false, None) };
    let a = AirRotation::takeoff(&t, &r, Takeoff::Jump, Vec2::ZERO, &i);
    assert!(a.swivel && a.flip_dir == Vec3::Z);
    let a = takeoff(Takeoff::Jump, level_board(), Vec3::ZERO, Vec2::ZERO, 0.0, None);
    assert!(!a.swivel && a.flip_dir == Vec3::X);
}

#[test]
fn dead_zones_before_landing() {
    let t = aerial();
    let mut a = takeoff(Takeoff::Jump, level_board(), Vec3::ZERO, Vec2::ZERO, 0.0, None);
    let set_with = |a: &mut AirRotation, ttl: f32, spin_idle: f32, flip_idle: f32, classic: bool| {
        a.prediction = Some(LandingPrediction { ticks: ttl, normal: Vec3::Y });
        (a.spin_idle, a.flip_idle) = (spin_idle, flip_idle);
        a.read_stick_inputs(&t, Vec2::new(1.0, 1.0), classic);
        assert_eq!(a.input, Vec2::new(a.spin_input, a.flip_input), "+0x40 / +0x44 keep the dead-zoned input");
        (a.spin_input, a.flip_input)
    };
    let set =
        |a: &mut AirRotation, ttl: f32, spin_idle: f32, flip_idle: f32| set_with(a, ttl, spin_idle, flip_idle, false);
    assert_eq!(set(&mut a, 50.0, 2.0, 2.0), (0.0, 1.0));
    assert_eq!(set(&mut a, 70.0, 2.0, 2.0), (1.0, 1.0));
    assert_eq!(set(&mut a, 15.0, 2.0, 2.0), (0.0, 0.0));
    assert_eq!(set(&mut a, 25.0, 2.0, 2.0), (0.0, 1.0));
    assert_eq!(set(&mut a, 15.0, 0.0, 0.0), (1.0, 1.0));
    assert_eq!(set_with(&mut a, 15.0, 2.0, 2.0, true), (1.0, 1.0));
}

fn step_once(a: &mut AirRotation, board: Quat, stick: Vec2, ttl: f32) -> Quat {
    let (t, r) = (aerial(), props());
    a.prediction = Some(LandingPrediction { ticks: ttl, normal: Vec3::Y });
    let inp = AirInputs {
        stick,
        lstick: stick,
        classic: false,
        velocity: Vec3::Z * 0.3,
        allow_stance_switch: true,
        grind_steering: None,
        tutorial_mode: false,
        heli_interact: None,
    };
    a.step(&t, &r, &inp, board, &mut false, 2.0)
}

#[test]
fn take_off_tick_restarts_the_air_time_and_idles_from_activate() {
    let mut a = takeoff(Takeoff::Jump, level_board(), Vec3::ZERO, Vec2::ZERO, 0.0, Some(100.0));
    assert_eq!((a.flip_idle, a.spin_idle, a.air_idle, a.air_time), (NO_LANDING, NO_LANDING, 0.0, 0.0));
    let b = step_once(&mut a, level_board(), Vec2::ZERO, 100.0);
    assert_eq!(a.air_time, 0.0, "the take-off tick");
    assert_eq!(a.air_idle, 2.0, "both idle past their thresholds from the first tick");
    step_once(&mut a, b, Vec2::ZERO, 98.0);
    assert_eq!(a.air_time, 2.0);
    let a = takeoff(Takeoff::Jump, level_board(), -Vec3::Y * (5.0 * DEG), Vec2::ZERO, 0.0, Some(100.0));
    assert_eq!((a.flip_idle, a.spin_idle), (NO_LANDING, 0.0));
}

#[test]
fn a_still_roll_take_off_leaves_swivel_mode_on_its_first_tick() {
    let n = Quat::from_axis_angle(Vec3::Z, 60.0 * DEG) * Vec3::Y;
    let board = frame(Vec3::Z, n);
    let (t, r) = (aerial(), props());
    let i = TakeoffInputs { ground_normal: n, ..inputs(board, Vec3::ZERO, 0.0, false, Some(100.0)) };
    let mut a = AirRotation::takeoff(&t, &r, Takeoff::Jump, Vec2::ZERO, &i);
    assert!(a.swivel && a.flip_dir == Vec3::Z);
    step_once(&mut a, board, Vec2::ZERO, 100.0);
    assert!(!a.swivel, "swivel off once both actuators are idle");
}

#[test]
fn an_air_jump_keeps_the_air_time_idle_timers_target_and_input() {
    let (t, r) = (aerial(), props());
    let mut a = takeoff(Takeoff::RollOff, level_board(), Vec3::ZERO, Vec2::ZERO, 0.0, Some(100.0));
    let mut b = level_board();
    for k in 0..5 {
        b = step_once(&mut a, b, Vec2::new(0.6, 0.0), 100.0 - 2.0 * k as f32);
    }
    let before = a;
    assert_eq!(before.air_time, 8.0);
    a.air_jump(&t, &r, &inputs(b, Vec3::ZERO, 1.0, true, Some(80.0)));
    assert!(!a.first_tick, "SelectLandingTarget runs on the air jump's tick");
    assert_eq!(
        (a.air_time, a.flip_idle, a.spin_idle, a.air_idle),
        (before.air_time, before.flip_idle, before.spin_idle, before.air_idle)
    );
    assert_eq!((a.ticks_to_land, a.target_valid, a.land_weight), (before.ticks_to_land, true, before.land_weight));
    assert_eq!((a.input, a.flip_input, a.spin_input), (before.input, before.flip_input, before.spin_input));
    assert!((a.spin.vel - 0.6 * 15.0 * DEG).abs() < 1e-6, "{}", a.spin.vel / DEG);
}

#[test]
fn no_spin_assist_after_a_trajectory_correction() {
    let (t, r) = (aerial(), props());
    let mut a = takeoff(Takeoff::Jump, level_board(), Vec3::ZERO, Vec2::ZERO, 0.0, None);
    (a.target_valid, a.ticks_to_land, a.land_weight) = (true, 50.0, 1.0);
    assert_eq!(a.assist(&t, &r).spin, 1.0);
    a.correction = true;
    let s = a.assist(&t, &r);
    assert_eq!((s.flip, s.spin), (1.0, 0.0));
    (a.target_valid, a.land_weight) = (false, 0.0);
    assert_eq!(a.assist(&t, &r).spin, 0.0);
}

#[test]
fn flip_assist_factor_window() {
    let (t, r) = (aerial(), props());
    let mut a = takeoff(Takeoff::Jump, level_board(), Vec3::ZERO, Vec2::ZERO, 0.0, None);
    (a.target_valid, a.ticks_to_land, a.flip_input, a.spin_input) = (true, 50.0, 1.0, 1.0);
    a.air_time = 10.0;
    let s = a.assist(&t, &r);
    assert_eq!((s.flip, s.spin), (1.0, 1.0), "first 30 ticks: full assist");
    a.air_time = 50.0;
    let s = a.assist(&t, &r);
    assert!((s.flip - 0.8).abs() < 1e-6 && s.spin == 1.0, "after 48 ticks: lerp(1, 0.8, |stick|) = {}", s.flip);
    a.flip_input = 0.0;
    assert_eq!(a.assist(&t, &r).flip, 1.0);
    a.air_time = 39.0;
    a.flip_input = 1.0;
    assert!((a.assist(&t, &r).flip - 0.8f32.max(0.5)).abs() < 1e-6, "curve 0.5 at 39 ticks < lerp 0.8");
    a.flip.vel = 0.1;
    assert_eq!(a.assist(&t, &r).flip_ticks, 56.0);
    a.flip.vel = -0.1;
    assert_eq!(a.assist(&t, &r).flip_ticks, 52.0);
    assert_eq!(a.assist(&t, &r).spin_ticks, 52.0);
    (a.target_valid, a.land_weight) = (false, 0.0);
    assert_eq!(a.assist(&t, &r).flip_ticks, NO_LANDING);
}

#[test]
fn spin_lands_on_a_multiple_of_pi_with_assist() {
    for (ticks, hold) in [(80.0f32, 80.0f32), (64.0, 30.0), (100.0, 100.0)] {
        let a = takeoff(Takeoff::Jump, level_board(), Vec3::ZERO, Vec2::ZERO, 0.0, Some(ticks));
        let (a, board, turned) = fly(a, ticks, |e| if e < hold { Vec2::new(1.0, 0.0) } else { Vec2::ZERO });
        let done = turned + a.spin.predict_delta(a.spin.chosen_target, 2.0);
        assert!(off_multiple(done, PI) < 1.0 * DEG, "{ticks}/{hold}: {}° + 2 ticks = {}°", turned / DEG, done / DEG);
        assert!(turned > 150.0 * DEG && (board * Vec3::Y).y > 0.999, "spun {}°, upright", turned / DEG);
        eprintln!(
            "air {ticks} ticks, stick {hold}: turned {:.1}° at touchdown, {:.1}° 2 ticks later",
            turned / DEG,
            done / DEG
        );
    }
}

#[test]
fn auto_upright_without_input() {
    let board = level_board() * Quat::from_rotation_z(-100.0 * DEG);
    let mut a = takeoff(Takeoff::Jump, board, Vec3::ZERO, Vec2::ZERO, 0.0, None);
    let (t, r) = (aerial(), props());
    let mut b = a.orientation();
    let mut goofy = false;
    for i in 1..=150 {
        a.prediction = None;
        let inp = AirInputs {
            stick: Vec2::ZERO,
            lstick: Vec2::ZERO,
            classic: false,
            velocity: Vec3::Z * 0.3,
            allow_stance_switch: true,
            grind_steering: None,
            tutorial_mode: false,
            heli_interact: None,
        };
        b = a.step(&t, &r, &inp, b, &mut goofy, 2.0);
        assert!(a.ticks_to_land == 40.0 || a.air_time <= 2.0);
        if i == 60 {
            assert!(off_multiple(a.flip.angle, std::f32::consts::TAU) < 10.0 * DEG, "{}°", a.flip.angle / DEG);
        }
    }
    assert!((b * Vec3::Y).y > 0.9999, "upright: {:?}", b * Vec3::Y);
    assert!(off_multiple(a.flip.angle, std::f32::consts::TAU) < 0.5 * DEG);
}

#[test]
fn tutorial_mode_auto_uprights_to_one_facing() {
    let (t, r) = (aerial(), props());
    let turned = Quat::from_rotation_y(150.0 * DEG) * level_board();
    for (board, tutorial, want, goofy_want) in [
        (level_board(), false, 0.0, false),
        (level_board(), true, 0.0, false),
        (turned, false, PI, true),
        (turned, true, 0.0, false),
    ] {
        let mut a = takeoff(Takeoff::Jump, board, Vec3::ZERO, Vec2::ZERO, 0.0, None);
        let (mut b, mut goofy) = (a.orientation(), false);
        for _ in 0..300 {
            a.prediction = None;
            let inp = AirInputs {
                stick: Vec2::ZERO,
                lstick: Vec2::ZERO,
                classic: false,
                velocity: Vec3::Z * 0.3,
                allow_stance_switch: true,
                grind_steering: None,
                tutorial_mode: tutorial,
                heli_interact: None,
            };
            b = a.step(&t, &r, &inp, b, &mut goofy, 2.0);
        }
        let off = crate::actuator::wrap_angle(yaw(b) - want).abs();
        assert!(off < 1.0 * DEG, "tutorial {tutorial}, start {}°: yaw {}°", yaw(board) / DEG, yaw(b) / DEG);
        assert!((b * Vec3::Y).y > 0.9999, "upright");
        assert_eq!(
            goofy,
            goofy_want,
            "the stance follows the facing (tutorial {tutorial}, start {}°)",
            yaw(board) / DEG
        );
        assert_eq!((a.land_spin_period, a.ticks_to_land), (if tutorial { std::f32::consts::TAU } else { PI }, 40.0));
    }
}

#[test]
fn stance_switches_only_once_air_idle() {
    let a = takeoff(Takeoff::Jump, level_board(), Vec3::ZERO, Vec2::ZERO, 0.0, Some(200.0));
    let (t, r) = (aerial(), props());
    let (mut a, mut goofy) = (a, false);
    let mut b = a.orientation();
    let mut held = true;
    let mut switched_at = None;
    for i in 0..100 {
        a.prediction = Some(LandingPrediction { ticks: 200.0 - 2.0 * i as f32, normal: Vec3::Y });
        if held && yaw(b).abs() > 100.0 * DEG {
            held = false;
        }
        let stick = if held { Vec2::new(1.0, 0.0) } else { Vec2::ZERO };
        let inp = AirInputs {
            stick,
            lstick: Vec2::ZERO,
            classic: false,
            velocity: Vec3::Z * 0.3,
            allow_stance_switch: true,
            grind_steering: None,
            tutorial_mode: false,
            heli_interact: None,
        };
        let idle_before = a.air_idle;
        b = a.step(&t, &r, &inp, b, &mut goofy, 2.0);
        if goofy && switched_at.is_none() {
            assert!(idle_before > 0.0, "switched while rotating (step {i})");
            switched_at = Some(i);
        }
    }
    assert!(!held, "the spin passed 100°");
    assert!(switched_at.is_some(), "switch once idle after a half turn (yaw {}°)", yaw(b) / DEG);
    assert!(yaw(b).abs() > FRAC_PI_2, "the board ends switch: yaw {}°", yaw(b) / DEG);
}

#[test]
fn a_held_spin_keeps_the_stance() {
    let a = takeoff(Takeoff::Jump, level_board(), Vec3::ZERO, Vec2::ZERO, 0.0, Some(240.0));
    let (t, r) = (aerial(), props());
    let (mut a, mut goofy) = (a, false);
    let mut b = a.orientation();
    let mut turned = 0.0;
    for i in 0..60 {
        a.prediction = Some(LandingPrediction { ticks: 240.0 - 2.0 * i as f32, normal: Vec3::Y });
        let inp = AirInputs {
            stick: Vec2::new(1.0, 0.0),
            lstick: Vec2::ZERO,
            classic: false,
            velocity: Vec3::Z * 0.3,
            allow_stance_switch: true,
            grind_steering: None,
            tutorial_mode: false,
            heli_interact: None,
        };
        let before = yaw(b);
        b = a.step(&t, &r, &inp, b, &mut goofy, 2.0);
        turned += crate::actuator::wrap_angle(yaw(b) - before);
        assert!(!goofy, "stance switched mid-spin at step {i}");
    }
    assert!(turned.abs() > 1.5 * std::f32::consts::TAU, "turned {}°", turned / DEG);
}

#[test]
fn frame_builders_as_emulated() {
    let fb = Quat::from_xyzw(0.1, 0.2, 0.3, 0.927).normalize();
    let axes = |q: Quat| (q * Vec3::X, q * Vec3::Y, q * Vec3::Z);
    let (x, y, z) = axes(frame_from_velocity(Vec3::new(0.0, -0.5, 1.0), fb));
    assert!(x.distance(Vec3::new(0.0, -0.4472, 0.8944)) < 1e-4 && y.distance(Vec3::new(0.0, 0.8944, 0.4472)) < 1e-4);
    assert!(z.distance(Vec3::NEG_X) < 1e-5);
    assert_eq!(frame_from_velocity(Vec3::new(0.049, 1.0, 0.049), fb), fb);
    assert_ne!(frame_from_velocity(Vec3::new(0.051, 1.0, 0.0), fb), fb);
    let s = 0.5f32.sqrt();
    let (_, y, z) = axes(frame_heading_normal(Vec3::Z, Vec3::new(s, s, 0.0), fb));
    assert!(y.distance(Vec3::Y) < 1e-5 && z.distance(Vec3::NEG_X) < 1e-5, "a side-tilted normal is ignored");
    let (x, y, _) = axes(frame_heading_normal(Vec3::new(0.0, -0.5, 1.0), Vec3::new(0.0, s, s), fb));
    assert!(y.distance(Vec3::new(0.0, s, s)) < 1e-5 && x.distance(Vec3::new(0.0, -s, s)) < 1e-5);
    let (x, y, z) = axes(frame_heading_normal(Vec3::new(0.3, -0.2, 0.5), Vec3::new(0.2, 0.9, -0.1), fb));
    assert!(
        x.distance(Vec3::new(0.5144, -0.0191, 0.8573)) < 1e-3 && y.distance(Vec3::new(0.0098, 0.9998, 0.0163)) < 1e-3
    );
    assert!(z.distance(Vec3::new(-0.8575, 0.0, 0.5145)) < 1e-3);
    assert_eq!(frame_heading_normal(Vec3::new(0.04, 0.1, 0.0), Vec3::Y, fb), fb);
    assert_ne!(frame_heading_normal(Vec3::new(0.06, 0.1, 0.0), Vec3::Y, fb), fb);
    assert_eq!(frame_heading_normal(Vec3::Z, Vec3::new(-1.0, 0.04, 0.0), fb), fb);
}

#[test]
fn landing_frames_fall_back_to_the_last_landing_frame() {
    let (t, r) = (aerial(), props());
    let mut a = takeoff(Takeoff::Jump, level_board(), Vec3::ZERO, Vec2::ZERO, 0.0, Some(40.0));
    let mut goofy = false;
    let mut inp = AirInputs {
        stick: Vec2::ZERO,
        lstick: Vec2::ZERO,
        classic: false,
        velocity: Vec3::Z * 0.3,
        allow_stance_switch: true,
        grind_steering: None,
        tutorial_mode: false,
        heli_interact: None,
    };
    let mut board = level_board();
    for _ in 0..3 {
        board = a.step(&t, &r, &inp, board, &mut goofy, 2.0);
    }
    let last = a.q_land;
    let turned = Quat::from_rotation_y(FRAC_PI_2) * board;
    inp.velocity = Vec3::new(0.01, -0.4, 0.02);
    for (prediction, heli) in [(Some(20.0), None), (None, None), (Some(20.0), Some(true))] {
        a.prediction = prediction.map(|ticks| LandingPrediction { ticks, normal: Vec3::Y });
        inp.heli_interact = heli;
        a.step(&t, &r, &inp, turned, &mut goofy, 2.0);
        assert_eq!(a.q_land, last, "prediction {prediction:?}, heli {heli:?}");
    }
}

#[test]
fn the_jump_assistance_latch_targets_its_flight_time_until_a_sooner_contact() {
    let i = TakeoffInputs { intercept: Some(40.0), ..inputs(level_board(), Vec3::ZERO, 0.0, false, Some(100.0)) };
    let mut a = AirRotation::takeoff(&aerial(), &props(), Takeoff::Jump, Vec2::ZERO, &i);
    assert!(a.intercept && a.intercept_ticks == 40.0);
    step_once(&mut a, level_board(), Vec2::ZERO, 100.0);
    assert_eq!(a.intercept_ticks, 40.0);
    step_once(&mut a, level_board(), Vec2::ZERO, 100.0);
    assert!(a.intercept && a.intercept_ticks == 38.0 && a.target_valid && a.land_weight == 1.0);
    step_once(&mut a, level_board(), Vec2::ZERO, 38.0);
    assert!(a.intercept && a.intercept_ticks == 36.0);
    step_once(&mut a, level_board(), Vec2::ZERO, 20.0);
    assert!(!a.intercept && a.intercept_ticks == NO_LANDING && a.land_weight == 0.0);
    let mut b = AirRotation::takeoff(&aerial(), &props(), Takeoff::Jump, Vec2::ZERO, &i);
    b.air_jump(&aerial(), &props(), &inputs(level_board(), Vec3::ZERO, 0.0, true, Some(100.0)));
    assert!(!b.intercept && b.intercept_ticks == NO_LANDING);
}

fn tilt(board: Quat) -> f32 {
    (board * Vec3::Y).angle_between(Vec3::Y) / DEG
}

fn falling_takeoff(pitch: f32, roll: f32, ttl: f32) -> (AirRotation, Quat, Vec3) {
    let v = Vec3::new(0.0, -pitch.sin(), pitch.cos()) * 0.5;
    let board = frame_from_velocity(v, Quat::IDENTITY) * Quat::from_rotation_x(roll);
    let i = TakeoffInputs { velocity: v, ..inputs(board, Vec3::ZERO, 0.0, false, Some(ttl)) };
    (AirRotation::takeoff(&aerial(), &props(), Takeoff::RollOff, Vec2::ZERO, &i), board, v)
}

fn fall_once(a: &mut AirRotation, board: Quat, v: Vec3, ttl: f32) -> Quat {
    a.prediction = Some(LandingPrediction { ticks: ttl, normal: Vec3::Y });
    let inp = AirInputs {
        stick: Vec2::ZERO,
        lstick: Vec2::ZERO,
        classic: false,
        velocity: v,
        allow_stance_switch: true,
        grind_steering: None,
        tutorial_mode: false,
        heli_interact: None,
    };
    a.step(&aerial(), &props(), &inp, board, &mut false, 2.0)
}

#[test]
fn the_reference_frame_drifts_without_a_flip() {
    let (mut a, board, v) = falling_takeoff(27.0 * DEG, 0.0, 180.0);
    assert!(a.flip.vel.abs() < 1e-6);
    let before = a.q_ref;
    fall_once(&mut a, board, v, 180.0);
    let turned = before.angle_between(a.q_ref) / DEG;
    assert!((turned - 3.0).abs() < 0.01, "q_ref turned {turned}°");
    a.q_ref = a.q_land * Quat::from_rotation_z(2.0 * DEG);
    a.drift_reference_frame(&aerial(), 2.0);
    assert!(a.q_ref.angle_between(a.q_land) < 1e-3);
}

#[test]
fn the_residual_decays_by_wobble_max_velocity_per_update() {
    let (mut a, mut board, v) = falling_takeoff(27.0 * DEG, 2.8 * DEG, 180.0);
    assert!((a.q_res.angle_between(Quat::IDENTITY) / DEG - 2.8).abs() < 0.05, "the roll is the residual");
    board = fall_once(&mut a, board, v, 180.0);
    assert!((a.q_res.angle_between(Quat::IDENTITY) / DEG - 1.3).abs() < 0.05);
    fall_once(&mut a, board, v, 178.0);
    assert_eq!(a.q_res, Quat::IDENTITY);
}

#[test]
fn a_tilted_roll_off_is_righted_as_the_original() {
    let board = Quat::from_xyzw(0.092_551_26, -0.565_290_6, -0.182_672_63, 0.799_069_1);
    let mut v = Vec3::new(0.160_303_67, -0.219_742_5, 0.424_512_4);
    let i = TakeoffInputs {
        board,
        wf: Quat::from_xyzw(-0.002_430_416_4, 0.001_015_306_1, 0.000_297_057_15, 0.999_996_5),
        velocity: v,
        ground_normal: Vec3::new(0.355_898_7, 0.502_391_4, 0.787_996_8),
        prewind: 0.0,
        trigger: false,
        prediction: Some(LandingPrediction { ticks: 181.0, normal: Vec3::Y }),
        intercept: None,
    };
    let mut a = AirRotation::takeoff(&aerial(), &props(), Takeoff::RollOff, Vec2::ZERO, &i);
    let mut b = board;
    let original = [(0, 23.2), (15, 14.5), (30, 5.4), (45, 2.0), (60, 0.8)];
    for update in 0..=60 {
        b = fall_once(&mut a, b, v, 181.0 - 2.0 * update as f32);
        v.y -= 2.0 * 24.0 / 3600.0;
        if let Some(&(_, want)) = original.iter().find(|(u, _)| *u == update) {
            let got = tilt(b);
            eprintln!("{update:2} updates after the take-off tick: {got:5.1}° (the original {want}°)");
            assert!(
                (got - want).abs() < 0.7,
                "{update} updates after the take-off tick: {got:.1}° (the original {want}°)"
            );
        }
    }
}

#[test]
fn a_wall_hug_roll_take_off_at_the_dam_is_righted() {
    let board = Quat::from_xyzw(0.468_093_43, 0.828_706_9, -0.305_014_94, 0.033_156_846);
    let mut v = Vec3::new(-0.109_635_54, 0.095_279_33, 0.151_669_53);
    let i = TakeoffInputs {
        board,
        wf: Quat::from_xyzw(-0.009_074_293, -0.012_942_168, -0.014_569_676, 0.999_768_9),
        velocity: v,
        ground_normal: Vec3::new(0.358_784_88, 0.460_394_1, 0.811_979_5),
        prewind: 0.0,
        trigger: false,
        prediction: Some(LandingPrediction {
            ticks: 121.748_49,
            normal: Vec3::new(-0.498_232_13, 0.681_836_2, 0.535_596_97),
        }),
        intercept: None,
    };
    let mut a = AirRotation::takeoff(&aerial(), &props(), Takeoff::RollOff, Vec2::X, &i);
    assert!(a.swivel && a.flip_dir == Vec3::Z, "a roll take-off");
    let roll = |b: Quat| (-(b * Vec3::Z).y).clamp(-1.0, 1.0).asin() / DEG;
    let mut b = board;
    assert!((roll(b) - 32.5).abs() < 0.1 && (tilt(b) - 67.9).abs() < 0.1, "{:+.1}° {:.1}°", roll(b), tilt(b));
    let mut worst = 0.0_f32;
    for update in 0..=35 {
        b = fall_once(&mut a, b, v, 121.7 - 2.0 * update as f32);
        v.y -= 2.0 * 24.0 / 3600.0;
        worst = worst.max(roll(b));
        if update % 5 == 0 {
            eprintln!("{update:2} updates after the take-off tick: tilt {:5.1}°, roll {:+5.1}°", tilt(b), roll(b));
        }
    }
    assert!(worst < 40.0, "the roll grew to {worst:.1}°");
    assert!(roll(b).abs() < 9.0 && tilt(b) < 15.0, "35 updates on: roll {:+.1}°, tilt {:.1}°", roll(b), tilt(b));
}

#[test]
fn a_still_flip_takes_the_forward_balance_horizon() {
    let (t, r) = (aerial(), props());
    let mut a = takeoff(Takeoff::Jump, level_board(), Vec3::ZERO, Vec2::ZERO, 0.0, None);
    (a.target_valid, a.ticks_to_land) = (true, 50.0);
    assert_eq!(a.flip.vel, 0.0, "a take-off without board rotation");
    assert_eq!(a.assist(&t, &r).flip_ticks, 56.0);
    a.flip_dir = -Vec3::X;
    assert_eq!(a.assist(&t, &r).flip_ticks, 52.0);
    a.flip.vel = f32::NAN;
    assert_eq!(a.assist(&t, &r).flip_ticks, 56.0, "NaN fails vcmpgefp: −1");
}

#[test]
fn a_vanishing_swivel_direction_takes_the_flip_axis() {
    let n = Quat::from_axis_angle(Vec3::Z, 60.0 * DEG) * Vec3::Y;
    let board = frame(Vec3::Z, n);
    let (t, r) = (aerial(), props());
    let i = TakeoffInputs { ground_normal: n, ..inputs(board, Vec3::ZERO, 0.0, false, Some(50.0)) };
    let mut a = AirRotation::takeoff(&t, &r, Takeoff::RollOff, Vec2::ZERO, &i);
    assert!(a.swivel && a.flip_dir == Vec3::Z, "a roll take-off");
    assert_eq!((a.flip.vel, a.flip.stick_scale), (0.0, 0.0), "still, no stick strength");
    step_once(&mut a, board, Vec2::new(0.0, 1.0), 50.0);
    assert_eq!(a.flip_dir, Vec3::X);
}

#[test]
fn a_swivel_stick_within_epsilon_is_no_input() {
    let t = aerial();
    let mut a = takeoff(Takeoff::Jump, level_board(), Vec3::ZERO, Vec2::ZERO, 0.0, Some(100.0));
    (a.swivel, a.flip_idle, a.spin_idle, a.flip_dir) = (true, 0.0, 0.0, Vec3::Z);
    a.read_stick_inputs(&t, Vec2::new(0.0, 1.0e-7), false);
    assert_eq!((a.flip_axis, a.flip_input, a.spin_input), (Vec3::Z, 0.0, 0.0));
    a.read_stick_inputs(&t, Vec2::new(0.0, 0.5), false);
    assert_eq!((a.flip_axis, a.flip_input, a.spin_input), (Vec3::X, 0.5, 0.0));
}

fn chasm_flip(stick: f32, updates: usize) -> Vec<f32> {
    let (t, r) = (aerial(), props());
    let game = Quat::from_xyzw(-0.063_720_9, -0.964_086_7, -0.196_127_8, 0.167_363);
    let board = game * Quat::from_rotation_y(FRAC_PI_2);
    let wall = Vec3::new(-0.2374, 0.2001, -0.9506).normalize();
    let i = TakeoffInputs {
        board,
        wf: Quat::from_xyzw(-0.002_045_451, 0.001_032_283, 0.000_248_966, 0.999_997_3),
        velocity: Vec3::new(0.158_657_2, -0.214_588, 0.415_670_3),
        ground_normal: Vec3::new(0.356_288_4, 0.471_600_8, 0.806_629_5),
        prewind: 0.0,
        trigger: false,
        prediction: Some(LandingPrediction { ticks: 122.08, normal: wall }),
        intercept: None,
    };
    let mut a = AirRotation::takeoff(&t, &r, Takeoff::RollOff, Vec2::ZERO, &i);
    let mut v = Vec3::new(0.155_721_6, -0.227_463, 0.407_979_4);
    let (mut b, mut tilts) = (board, Vec::new());
    for update in 0..=updates {
        let ticks = if update == 0 { 122.08 } else { 180.83 - 2.0 * (update - 1) as f32 };
        a.prediction = Some(LandingPrediction { ticks, normal: wall });
        let s = if (1..=15).contains(&update) { Vec2::new(0.0, stick) } else { Vec2::ZERO };
        let inp = AirInputs {
            stick: s,
            lstick: s,
            classic: false,
            velocity: v,
            allow_stance_switch: true,
            grind_steering: None,
            tutorial_mode: false,
            heli_interact: None,
        };
        b = a.step(&t, &r, &inp, b, &mut false, 2.0);
        tilts.push(tilt(b));
        v = Vec3::new(v.x * 0.9822, v.y - 0.012_75, v.z * 0.9822);
    }
    tilts
}

#[test]
fn the_chasm_flips_follow_the_original_from_its_take_off() {
    let front = [(1, 24.3), (2, 26.6), (3, 30.3), (5, 41.0), (10, 82.7), (15, 135.6), (20, 175.4), (25, 135.8)];
    let back = [(1, 22.8), (2, 21.7), (3, 19.8), (5, 12.5), (7, 0.7), (10, 24.1), (15, 78.1), (20, 117.8), (25, 116.9)];
    let late = [(30, 103.0, 97.1), (40, 56.7, 55.8), (50, 30.5, 30.2), (60, 16.4, 16.2), (70, 8.8, 8.7)];
    let (f, bk) = (chasm_flip(1.0, 70), chasm_flip(-1.0, 70));
    for (name, got, want) in [
        ("front", &f, front.iter().copied().chain(late.iter().map(|&(u, x, _)| (u, x))).collect::<Vec<_>>()),
        ("back", &bk, back.iter().copied().chain(late.iter().map(|&(u, _, x)| (u, x))).collect()),
    ] {
        for (u, w) in want {
            eprintln!("{name} update {u:2}: {:6.1}° (the original {w}°)", got[u]);
            assert!((got[u] - w).abs() < 0.3, "{name} update {u}: {:.1}° against the original's {w}°", got[u]);
        }
    }
}
