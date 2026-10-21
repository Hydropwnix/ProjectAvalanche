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

fn takeoff(kind: Takeoff, board: Quat, omega: Vec3, stick: Vec2, prewind: f32, ttl: Option<f32>) -> AirRotation {
    let pred = ttl.map(|ticks| LandingPrediction { ticks, normal: Vec3::Y });
    let wf = Quat::from_scaled_axis(omega);
    AirRotation::takeoff(&aerial(), &props(), kind, board, wf, Vec3::Z * 0.3, Vec3::Y, stick, prewind, pred)
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
    let a = AirRotation::takeoff(&t, &r, Takeoff::Jump, board, Quat::IDENTITY, Vec3::Z * 0.3, n, Vec2::ZERO, 0.0, None);
    assert!(a.swivel && a.flip_dir == Vec3::Z);
    let a = takeoff(Takeoff::Jump, level_board(), Vec3::ZERO, Vec2::ZERO, 0.0, None);
    assert!(!a.swivel && a.flip_dir == Vec3::X);
}

#[test]
fn dead_zones_before_landing() {
    let t = aerial();
    let mut a = takeoff(Takeoff::Jump, level_board(), Vec3::ZERO, Vec2::ZERO, 0.0, None);
    let set = |a: &mut AirRotation, ttl: f32, spin_idle: f32, flip_idle: f32| {
        a.prediction = Some(LandingPrediction { ticks: ttl, normal: Vec3::Y });
        (a.spin_idle, a.flip_idle) = (spin_idle, flip_idle);
        a.read_stick_inputs(&t, Vec2::new(1.0, 1.0));
        (a.spin_input, a.flip_input)
    };
    assert_eq!(set(&mut a, 50.0, 2.0, 2.0), (0.0, 1.0));
    assert_eq!(set(&mut a, 70.0, 2.0, 2.0), (1.0, 1.0));
    assert_eq!(set(&mut a, 15.0, 2.0, 2.0), (0.0, 0.0));
    assert_eq!(set(&mut a, 25.0, 2.0, 2.0), (0.0, 1.0));
    assert_eq!(set(&mut a, 15.0, 0.0, 0.0), (1.0, 1.0));
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
