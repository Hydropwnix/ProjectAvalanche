use std::f32::consts::{FRAC_PI_2, PI};

use super::*;
use crate::actuator::crit_damped;
use crate::testutil::{balance, tuning};

const DEG: f32 = PI / 180.0;

fn board(yaw: f32, pitch: f32) -> Quat {
    let level = Quat::from_mat3(&glam::Mat3::from_cols(Vec3::Z, Vec3::Y, Vec3::Z.cross(Vec3::Y)));
    level * Quat::from_rotation_y(yaw) * Quat::from_rotation_z(pitch)
}

fn land(yaw: f32, pitch: f32, uber: bool) -> (LandingState, Quat) {
    land_rotating(yaw, pitch, 0.0, uber)
}

fn land_rotating(yaw: f32, pitch: f32, pitch_rate: f32, uber: bool) -> (LandingState, Quat) {
    let t = balance();
    let mut steering = SteeringState::new(&tuning().carving);
    let mut a = board(yaw, pitch);
    let w = Quat::from_scaled_axis((a * Vec3::Z) * pitch_rate);
    let mut l = LandingState::default();
    l.touchdown(
        &t,
        &tuning().carving,
        &mut a,
        &mut steering,
        Vec3::Z * 0.3,
        Vec3::Y,
        w,
        uber,
        true,
        &ImpactKick::default(),
        2.0,
    );
    (l, a)
}

#[test]
fn touchdown_snaps_yaw_to_the_velocity_and_pitch_to_the_ground() {
    let (l, a) = land(20.0 * DEG, 0.0, false);
    assert!((a * Vec3::X).distance(Vec3::Z) < 1e-4 && (a * Vec3::Y).distance(Vec3::Y) < 1e-4);
    assert!((l.spin - 20.0 * DEG).abs() < 1e-4 && l.pitch.abs() < 1e-4);
    let (l, a) = land(0.0, 30.0 * DEG, false);
    assert!((a * Vec3::X).distance(Vec3::Z) < 1e-4 && (a * Vec3::Y).distance(Vec3::Y) < 1e-4);
    assert!(l.spin.abs() < 1e-4 && (l.pitch - 30.0 * DEG).abs() < 1e-4);
    assert!(!l.bail_on_landing && l.quality == LandingQuality::Sloppy && !l.landed_switch);
    let (l, a) = land(20.0 * DEG, 30.0 * DEG, false);
    let roll = (a * Vec3::Z).dot(Vec3::Y).asin();
    assert!((l.spin - 20.0 * DEG).abs() < 1e-4, "spin {}°", l.spin / DEG);
    assert!(twist(crate::steering::ground_frame(a, Vec3::Y).conjugate() * a, Vec3::Z).abs() < 1e-4);
    assert!(roll.abs() < 15.0 * DEG && l.pitch > 25.0 * DEG, "roll {}°, pitch {}°", roll / DEG, l.pitch / DEG);
    eprintln!("20° slip + 30° nose up: residual roll {:.1}°, kept pitch {:.1}°", roll / DEG, l.pitch / DEG);
    let (l, a) = land(PI - 10.0 * DEG, 0.0, false);
    assert!(l.landed_switch && (a * Vec3::X).distance(-Vec3::Z) < 1e-4 && (l.spin.abs() - 10.0 * DEG).abs() < 1e-4);
}

#[test]
fn bail_only_past_135_degrees() {
    let (l, a) = land(0.0, 150.0 * DEG, false);
    assert!(l.bail_on_landing && l.quality == LandingQuality::Bail);
    assert!((a * Vec3::Y).distance(Vec3::Y) < 1e-4, "the board is snapped flat even when bailing");
    let (l, _) = land(0.0, -140.0 * DEG, false);
    assert!(l.bail_on_landing);
    let (l, _) = land(0.0, 130.0 * DEG, false);
    assert!(!l.bail_on_landing && l.quality == LandingQuality::Sloppy);
    let (l, a) = land(0.0, 40.0 * DEG, true);
    assert!(l.uber_crash && l.quality == LandingQuality::UberCrash && l.is_bailing());
    assert!(l.pitch == 0.0 && l.spin == 0.0 && (a * Vec3::Y).distance(Vec3::Y) > 0.5, "not snapped");
}

fn settle(l: &mut LandingState, stick_y: f32, steps: usize) -> Option<usize> {
    let t = balance();
    let mut stable_at = None;
    for i in 0..steps {
        let _ = l.balance_update(
            &t,
            (1.0, 1.0),
            stick_y,
            0.3,
            false,
            true,
            false,
            false,
            false,
            false,
            false,
            (0.0, false),
            2.0,
        );
        if l.stable && stable_at.is_none() {
            stable_at = Some(i);
        }
    }
    stable_at
}

#[test]
fn small_pitch_settles_to_a_clean_landing() {
    let (mut l, _) = land(5.0 * DEG, 15.0 * DEG, false);
    let at = settle(&mut l, 0.0, 120).expect("stable");
    assert!(l.quality == LandingQuality::Clean && l.landing_score == 1.0, "{:?}", l.quality);
    assert!(l.pitch.abs() < 25.0 * DEG && l.spin_vel.abs() < 1.0 * DEG && at <= 2, "stable at step {at}");
    eprintln!("15° nose up, 5° yaw: stable after {} ticks, pitch {:.1}°", 2 * at, l.pitch / DEG);
}

#[test]
fn over_pitching_tumbles_and_recovers() {
    let (mut l, _) = land(0.0, -70.0 * DEG, false);
    assert!(!l.bail_on_landing);
    let t = balance();
    let _ =
        l.balance_update(&t, (1.0, 1.0), 0.0, 0.3, false, true, false, false, false, false, false, (0.0, false), 2.0);
    assert_eq!(l.landing_score, 0.0);
    assert_eq!(l.quality, LandingQuality::Tumble);
    assert!(l.pitch > PI && l.tumbling, "{}", l.pitch / DEG);
    assert!(!l.is_upright(&t) && t.tumble_friction_curve.eval(l.pitch) > 0.2, "tumbling brakes by TumbleFrictionCurve");
    for _ in 0..60 {
        let _ = l.balance_update(
            &t,
            (1.0, 1.0),
            0.0,
            0.3,
            false,
            true,
            false,
            false,
            false,
            false,
            false,
            (0.0, false),
            2.0,
        );
    }
    assert!((l.pitch - PI / 2.0).abs() < 3.0 * DEG && !l.stable, "on the back: {}°", l.pitch / DEG);
    let _ =
        l.balance_update(&t, (1.0, 1.0), 0.0, 0.3, true, true, false, false, false, false, false, (0.0, false), 2.0);
    assert!(l.auto_recover_backslide);
    let at = settle(&mut l, 0.0, 600).expect("recovers");
    assert_eq!(l.quality, LandingQuality::Tumble, "the landing stays a tumble");
    eprintln!("70° nose down: tumble, stable again after {} ticks", 2 * (at + 1));
}

#[test]
fn a_backslide_ends_by_itself_unless_the_stick_holds_it() {
    let t = balance();
    let on_back = || {
        let (mut l, _) = land(0.0, -70.0 * DEG, false);
        let _ = l.balance_update(
            &t,
            (1.0, 1.0),
            0.0,
            0.3,
            false,
            true,
            false,
            false,
            false,
            false,
            false,
            (0.0, false),
            2.0,
        );
        for _ in 0..90 {
            let _ = l.balance_update(
                &t,
                (1.0, 1.0),
                -1.0,
                0.3,
                false,
                true,
                false,
                false,
                false,
                false,
                false,
                (0.0, false),
                2.0,
            );
        }
        assert!(l.backslide && (l.pitch - PI / 2.0).abs() < 3.0 * DEG, "on the back: {}°", l.pitch / DEG);
        l
    };
    let mut l = on_back();
    for _ in 0..600 {
        let _ = l.balance_update(
            &t,
            (1.0, 1.0),
            -1.0,
            0.0,
            false,
            true,
            false,
            false,
            false,
            false,
            false,
            (0.0, false),
            2.0,
        );
    }
    assert!(l.backslide && !l.auto_recover_backslide && !l.stable && l.backslide_ticks <= 2.0, "{}°", l.pitch / DEG);
    let mut l = on_back();
    let mut recovered_at = None;
    for k in 0..600 {
        let _ = l.balance_update(
            &t,
            (1.0, 1.0),
            0.0,
            0.0,
            false,
            true,
            false,
            false,
            false,
            false,
            false,
            (0.0, false),
            2.0,
        );
        if l.auto_recover_backslide && recovered_at.is_none() {
            recovered_at = Some(2.0 * (k + 1) as f32);
        }
    }
    let at = recovered_at.expect("recovers without input");
    assert!((at - t.max_ticks_in_auto_backslide).abs() <= 2.0, "at {at} ticks");
    assert!(!l.backslide && l.pitch.abs() < 25.0 * DEG, "upright again: {}°", l.pitch / DEG);
    let mut l = on_back();
    for _ in 0..600 {
        let _ = l.balance_update(
            &t,
            (1.0, 1.0),
            1.0,
            0.0,
            false,
            true,
            false,
            false,
            false,
            false,
            false,
            (0.0, false),
            2.0,
        );
    }
    assert!(!l.backslide && l.pitch.abs() < 25.0 * DEG, "upright with the helping stick: {}°", l.pitch / DEG);
}

#[test]
fn bail_springs_onto_the_back_until_a_fresh_controller() {
    let (mut l, _) = land_rotating(0.0, 150.0 * DEG, 0.1, false);
    let t = balance();
    assert_eq!(l.adhesion_factor(&tuning().carving.balance_surface_adhesion_modifier_curve), 0.0);
    for _ in 0..30 {
        let _ = l.balance_update(
            &t,
            (1.0, 1.0),
            0.0,
            0.3,
            false,
            true,
            false,
            false,
            false,
            false,
            false,
            (0.0, false),
            2.0,
        );
    }
    assert!(l.bail_on_landing && (l.pitch - PI).abs() < 5.0 * DEG, "pitch {}°", l.pitch / DEG);
    for _ in 30..600 {
        let _ = l.balance_update(
            &t,
            (1.0, 1.0),
            0.0,
            0.3,
            true,
            true,
            false,
            false,
            false,
            false,
            false,
            (0.0, false),
            2.0,
        );
    }
    assert!(l.bail_on_landing && l.tumbling && (l.pitch - PI).abs() < 1.0 * DEG, "pitch {}°", l.pitch / DEG);
    l.fresh_controller();
    assert!(!l.bail_on_landing && !l.is_bailing() && l.pitch == 0.0 && l.landing_score == 1.0);
    assert_eq!(l.quality, LandingQuality::Bail);
}

#[test]
fn the_bounce_rotational_force_turns_a_bailing_pitch() {
    let t = balance();
    let (bail, _) = land_rotating(0.0, 150.0 * DEG, 0.1, false);
    assert!(bail.bail_on_landing);
    let mut l = bail;
    let p0 = l.pitch;
    let _ = l.balance_update(
        &t,
        (1.0, 1.0),
        0.0,
        0.3,
        false,
        true,
        false,
        false,
        false,
        false,
        false,
        (-0.125, false),
        2.0,
    );
    assert_eq!((l.pitch_vel, l.pitch), (-0.125, (-0.125f32).mul_add(2.0, p0)));
    let mut back = bail;
    let _ = back.balance_update(
        &t,
        (-1.0, -1.0),
        0.0,
        0.3,
        false,
        true,
        false,
        false,
        false,
        false,
        false,
        (-0.125, false),
        2.0,
    );
    assert_eq!(back.pitch_vel, 0.125, "x·fwd");
    let mut spring = bail;
    let _ = spring.balance_update(
        &t,
        (1.0, 1.0),
        0.0,
        0.3,
        false,
        true,
        false,
        false,
        false,
        false,
        false,
        (0.0, false),
        2.0,
    );
    assert_ne!(spring.pitch_vel, -0.125);
    let mut rock = LandingState { pitch: 0.2, ..LandingState::default() };
    let _ = rock.balance_update(
        &t,
        (1.0, 1.0),
        0.0,
        0.3,
        false,
        true,
        false,
        false,
        false,
        false,
        false,
        (-0.125, true),
        2.0,
    );
    assert_eq!((rock.pitch_vel, rock.pitch), (-0.125, (-0.125f32).mul_add(2.0, 0.2)));
    let mut riding = LandingState { pitch: 0.2, ..LandingState::default() };
    let _ = riding.balance_update(
        &t,
        (1.0, 1.0),
        0.0,
        0.3,
        false,
        true,
        false,
        false,
        false,
        false,
        false,
        (-0.125, false),
        2.0,
    );
    assert_ne!(riding.pitch_vel, -0.125, "no bail: the spring");
}

#[test]
fn spring_acceleration_prediction_is_the_springs_second_derivative() {
    let (k, e0, rate, dt) = (0.012f32.sqrt(), 0.7, -0.05, 2.0);
    let h = 1e-2;
    let r = |t: f32| crit_damped(e0, rate, k, t).1;
    let numeric = (r(dt + h) - r(dt - h)) / (2.0 * h);
    let decoded = spring_accel_after(k, e0, rate, 0.0, dt);
    assert!((numeric - decoded).abs() < 1e-5, "{numeric} vs {decoded}");
}

#[test]
fn spin_target_takes_the_calmer_spring() {
    let (k, dt) = (0.012f32.sqrt(), 2.0);
    let (spin, target) = spin_target(k, 20.0 * DEG, -0.05, 0.0, 1.0, dt);
    assert!(spin == 20.0 * DEG && target == 0.0);
    let (s, v) = (80.0 * DEG, 10.0 * DEG);
    let (_, target) = spin_target(k, s, v, 0.0, 1.0, dt);
    let calmer =
        if spring_accel_after(k, s, v, PI, dt).abs() <= spring_accel_after(k, s, v, 0.0, dt).abs() { PI } else { 0.0 };
    assert_eq!(target, calmer);
    let (spin, target) = spin_target(k, -80.0 * DEG, -10.0 * DEG, 0.0, 1.0, dt);
    if spring_accel_after(k, -80.0 * DEG, -10.0 * DEG, -PI, dt).abs()
        <= spring_accel_after(k, -80.0 * DEG, -10.0 * DEG, 0.0, dt).abs()
    {
        assert!((target - PI).abs() < 1e-5 && (spin - (TAU - 80.0 * DEG)).abs() < 1e-5, "{spin} {target}");
    }
}

#[test]
fn twist_is_the_rotation_about_the_axis() {
    let q = Quat::from_rotation_z(0.4) * Quat::from_rotation_x(0.0);
    assert!((twist(q, Vec3::Z) - 0.4).abs() < 1e-6);
    assert!(twist(Quat::from_rotation_y(-2.0), Vec3::Y) + 2.0 < 1e-6);
    assert!((twist(Quat::from_rotation_z(3.5), Vec3::Z) - (3.5 - TAU)).abs() < 1e-5);
}

#[test]
fn ground_frame_projects_the_board_side() {
    use crate::steering::ground_frame;
    let s = 0.5f32.sqrt();
    let n = Vec3::new(0.0, s, s);
    let a = Quat::from_rotation_y(0.5);
    let g = ground_frame(a, n);
    assert!((g * Vec3::Z).distance(Vec3::new(0.6114, -0.5596, 0.5596)) < 1e-3);
    assert!((g * Vec3::X).distance(Vec3::new(0.7913, 0.4323, -0.4323)) < 1e-3);
    assert!((g * Vec3::Y).distance(n) < 1e-5);
    let a = Quat::from_rotation_x(-60f32.to_radians());
    let g = ground_frame(a, Vec3::Y);
    assert!((g * Vec3::X).distance(Vec3::X) < 1e-5 && (g * Vec3::Y).distance(Vec3::Y) < 1e-5);
}

#[test]
fn an_impact_kicks_the_spin_sideways_and_softens_the_springs() {
    let t = balance();
    let c = tuning().carving;
    let kick = |n: Vec3, s: f32| {
        let mut steering = SteeringState::new(&c);
        let mut a = board(0.0, 0.0);
        let mut l = LandingState::default();
        let k = ImpactKick { pre_orientation: Quat::IDENTITY, pre_velocity: Vec3::Z * 0.3, velocity: n * s };
        l.touchdown(&t, &c, &mut a, &mut steering, Vec3::Z * 0.3, Vec3::Y, Quat::IDENTITY, false, true, &k, 2.0);
        l
    };
    let l = kick(Vec3::X, 0.2);
    assert!(l.use_impact_tuning && (l.spin_kick + t.spin_impact.eval(0.2)).abs() < 1e-6, "{}", l.spin_kick);
    assert!(l.spin_kick < -0.05 && l.spin_vel == l.spin_kick && l.pitch_kick == 0.0);
    let r = kick(-Vec3::X, 0.2);
    assert!((r.spin_kick + l.spin_kick).abs() < 1e-6);
    let small = kick(Vec3::X, 0.04);
    assert!(small.use_impact_tuning && small.spin_kick == 0.0);
    let none = kick(Vec3::X, 0.0);
    assert!(!none.use_impact_tuning && none.spin_kick == 0.0);
    let mut a = l;
    let mut b = LandingState { use_impact_tuning: false, ..l };
    for _ in 0..5 {
        let _ = a.balance_update(
            &t,
            (1.0, 1.0),
            0.0,
            0.3,
            false,
            true,
            false,
            false,
            false,
            false,
            false,
            (0.0, false),
            2.0,
        );
        let _ = b.balance_update(
            &t,
            (1.0, 1.0),
            0.0,
            0.3,
            false,
            true,
            false,
            false,
            false,
            false,
            false,
            (0.0, false),
            2.0,
        );
    }
    assert!((a.spin - a.spin_target).abs() > (b.spin - b.spin_target).abs());
}

#[test]
fn a_fall_onto_a_slope_snaps_the_board_onto_the_slope_travel() {
    let (t, c) = (balance(), tuning().carving);
    let n = Vec3::new(0.0, 0.92, 0.39).normalize();
    let v = Vec3::new(0.1, -0.8, 0.0);
    let mut steering = SteeringState::new(&c);
    let mut a = board(FRAC_PI_2, 0.0);
    assert!((a * Vec3::X).distance(Vec3::X) < 1e-5);
    let mut l = LandingState::default();
    l.touchdown(&t, &c, &mut a, &mut steering, v, n, Quat::IDENTITY, false, true, &ImpactKick::default(), 2.0);
    let slip_after = |a: Quat| {
        let g = crate::steering::ground_frame(a, n);
        let vl = g.conjugate() * v;
        vl.z.atan2(vl.x).abs()
    };
    assert!(l.spin.abs() > 60.0 * DEG, "spin {}°", l.spin / DEG);
    assert!(slip_after(a) < 10.0 * DEG, "slip after the snap {}°", slip_after(a) / DEG);
    assert!(slip_after(board(FRAC_PI_2, 0.0)) > 60.0 * DEG);
}

#[test]
fn a_landing_bail_has_no_ground_end_of_its_own() {
    let (mut l, _) = land_rotating(0.0, 150.0 * DEG, 0.1, false);
    let t = balance();
    assert!(l.bail_on_landing);
    for _ in 0..450 {
        let _ = l.balance_update(
            &t,
            (1.0, 1.0),
            0.0,
            0.3,
            false,
            true,
            false,
            false,
            false,
            false,
            false,
            (0.0, false),
            2.0,
        );
    }
    assert!(l.bail_on_landing && l.is_bailing() && l.tumbling);
}

#[test]
fn level_complete_clears_a_landing_bail() {
    let (mut l, _) = land(0.0, 150.0 * DEG, false);
    let t = balance();
    assert!(l.bail_on_landing);
    let _ =
        l.balance_update(&t, (1.0, 1.0), 0.0, 0.3, false, true, false, false, false, true, false, (0.0, false), 2.0);
    assert!(!l.bail_on_landing && !l.is_bailing());
}

#[test]
fn the_landing_snaps_do_not_normalise() {
    let (t, c) = (balance(), tuning().carving);
    let mut steering = SteeringState::new(&c);
    let mut a = board(0.4, 0.1) * 1.001;
    let k = ImpactKick::default();
    let mut l = LandingState::default();
    l.touchdown(&t, &c, &mut a, &mut steering, Vec3::Z * 0.3, Vec3::Y, Quat::IDENTITY, false, true, &k, 2.0);
    assert!((a.length() - 1.001).abs() < 1e-5, "{}", a.length());
}

struct BalCase {
    want: Vec<u32>,
    want_flags: Vec<u8>,
    got: Vec<u32>,
    got_flags: Vec<u8>,
}

fn run_bal_case(line: &str) -> BalCase {
    use crate::curve::Curve;
    let h = |s: &str| f32::from_bits(u32::from_str_radix(s, 16).unwrap());
    let (ins, outs) = line.split_once(" | ").unwrap();
    let (fl, ints) = ins.split_once(" ; ").unwrap();
    let x: Vec<f32> = fl.split(' ').map(h).collect();
    let k: Vec<u8> = ints.split(' ').map(|v| v.parse().unwrap()).collect();
    let mut o = outs.split(" ; ");
    let want: Vec<u32> = o.next().unwrap().split(' ').map(|v| u32::from_str_radix(v, 16).unwrap()).collect();
    let want_flags: Vec<u8> = o.next().unwrap().split(' ').map(|v| v.parse().unwrap()).collect();
    let curve = |i: usize| Curve::new(vec![[x[13 + 4 * i], x[14 + 4 * i]], [x[15 + 4 * i], x[16 + 4 * i]]]);
    let t = BalanceTuning {
        stabilized_spin_vel: x[1],
        stabilized_spin_angle: x[2],
        stabilized_pitch_vel: x[3],
        stabilized_pitch_angle: x[4],
        input_dead_zone: x[5],
        impact_stiffness_multiplier: x[6],
        spin_spring_stiffness: x[7],
        impact_spin_spring_stiffness: x[8],
        back_slide_threshold: x[9],
        pitch_balance_stable_threshold: x[10],
        max_ticks_in_auto_backslide: x[11],
        max_pitch_angle_vel_to_prevent_braking: x[12],
        bail_angle_curve: curve(0),
        bail_tumble_angular_velocity_curve: curve(1),
        recovery: curve(2),
        restore: curve(3),
        recovery_helping: curve(4),
        restore_helping: curve(5),
        recovery_hindering: curve(6),
        restore_hindering: curve(7),
        ..BalanceTuning::default()
    };
    let f = &x[45..65];
    let flag = |i: usize| k[i] != 0;
    let mut l = LandingState {
        landing_pitch_vel: f[0],
        pitch_balance: f[1],
        pitch_balance_vel: f[2],
        pitch_vel: f[3],
        pitch: f[4],
        spin: f[6],
        spin_vel: f[7],
        spin_target: f[8],
        pitch_target: f[9],
        stable_ticks: f[13],
        backslide_ticks: f[14],
        landing_score: f[18],
        auto_recover_backslide: flag(0),
        bailed_because_of_rock: flag(1),
        stable: flag(2),
        backslide: flag(4),
        tumbling: flag(5),
        bail_on_landing: flag(7),
        use_impact_tuning: flag(8),
        ..LandingState::default()
    };
    let (forward_sign, bounce_x) = (f[5], f[19]);
    let (fwd, dt, stick, speed) = (x[72], x[73], x[74], x[75]);
    let (mut uls, mut rock_ticks, mut is_dead) = (f[10], f[15], flag(3));
    let leg = flag(6);
    let grind = k[9];
    let [not_alive, recovery, press, level, uber, rock_bail] = std::array::from_fn(|i| k[10 + i] != 0);
    let mut fs = crate::press::CarvingPress { force_stop_mag: f[11], force_stop_vel: f[12], ..Default::default() };
    uls = if leg { uls + dt } else { 0.0 };
    let clear = l.balance_update(
        &t,
        (fwd, forward_sign),
        stick,
        speed,
        recovery,
        leg,
        uber,
        grind == 2,
        is_dead,
        level,
        press,
        (bounce_x, rock_bail),
        dt,
    );
    if clear {
        rock_ticks = 0.0;
    }
    if not_alive && !level {
        is_dead = true;
    }
    let up = l.is_upright(&BalanceTuning { max_pitch_angle_vel_to_prevent_braking: x[12], ..t.clone() });
    crate::press::force_stop_update(&mut fs, x[0], up, level, dt);
    let got = [
        l.pitch_balance,
        l.pitch_balance_vel,
        l.pitch_vel,
        l.pitch,
        l.spin,
        l.spin_vel,
        l.pitch_target,
        uls,
        fs.force_stop_mag,
        fs.force_stop_vel,
        l.stable_ticks,
        l.backslide_ticks,
        rock_ticks,
        f[16],
        f[17],
        l.landing_score,
    ]
    .map(f32::to_bits)
    .to_vec();
    let got_flags = [
        l.auto_recover_backslide,
        l.bailed_because_of_rock,
        l.stable,
        is_dead,
        l.backslide,
        l.tumbling,
        leg,
        l.bail_on_landing,
        l.use_impact_tuning,
    ]
    .map(u8::from)
    .to_vec();
    BalCase { want, want_flags, got, got_flags }
}

const BAL_CASES: [&str; 12] = [
    "3d1793b3 3b0abda4 3dd198e8 3c194bbb 3e996d5d 3e4c6345 40278e46 3d48069b 3ce9d5b5 3f868a24 3e1b4bf0 42374639 \
         3dbac0cf 3f927e87 40108be6 3ff5e3ee 3fd584aa 3f0336c8 3d76e11a 3f30417a 3e1988f2 3fef871a 3c0113ac 4028fede \
         3d4fb266 401648f4 3d52ad3c 401d9af6 3cf379ee 3f5c2dcd 3d1fa5da 3fb7ca8b 39839037 3fbf73ff 3ca14f6d 404cabfa \
         3c2d580c 3fa9c67a 3d4eb0be 40135b28 3bf7ae65 3fd32309 3d543be2 402af4da 3d291ecb bda95495 be7e94d6 bc2ad3a9 \
         bdf1c94e c04c8f0c 3f800000 c0515845 3e499453 3f8d9824 bd4a4e88 4162ca80 3f68031a 3d28a689 41205e69 41da4183 \
         41ef1420 bf3a73c3 bdaa2e7c 3c942335 3e0e67fc bf0a2547 3e7849dd bea45d8d beb72cbf bf5bb453 bd5781cb beba820d \
         bf800000 3f800000 3da8431d 3d5241e2 bda139fc ; 0 0 0 0 1 0 0 0 0 2 0 1 0 1 0 0 | be838684 bbcc836a 3d76e11a \
         c048b388 c041552d 3e96b1a5 bd4a4e88 00000000 3f711959 3cf82d82 41305e69 00000000 00000000 bd22309c bdf2524a \
         3c942335 ; 1 0 1 0 0 0 0 0 0 ; X",
    "3c910fd9 3d1d1c97 3e509f8f 3bdb7618 3eead289 3dd91d4f 40356845 3bf6a990 3c8c9eed 3fb92bc9 3da46730 41fca852 \
         3cd7c5d9 3f176706 400f0d82 3f296f23 3fabe8c4 3dce915c 3e4b1044 3f635720 3e3addd4 3e226e2c 3d6c4798 3f5f9f1e \
         3c92e618 3f30fc83 3c32e013 3fa659f4 3b970fcd 3fcbd09e 3c519a41 3fd2d202 3beb29e9 3fa1f206 3be1a4f3 3feb7c03 \
         3bd47315 3f422d05 3cf819ab 3fb5b7b4 3d3b192f 3d0d4c74 3c7e2c27 4004b65d 3c87820b bd8260a4 3cc70116 bc0a2354 \
         3c077e75 7fc00000 3f800000 bee3f78e 3d6d0096 3e4a6803 3f2c78b6 41e54855 3ec5d93d 3db3a6e3 41393bb2 4287cf4e \
         41cd5c83 beea850c 3c9e3524 3edbf1b0 bb0f4f94 3dd2e1b4 3e236bda bde01e71 bdff4736 be91ca08 bec1abe0 3f5f3838 \
         3e276ac1 3f800000 3f185dee 3e6512eb 3e06a159 ; 0 0 0 0 0 0 0 0 1 0 0 0 1 1 0 0 | 3c8793b1 bbe83067 bd032522 \
         7fc00000 bec763b7 3d5c501d 3f2c78b6 00000000 00000000 00000000 00000000 00000000 41cd5c83 be059f0f 3a812500 \
         3edbf1b0 ; 0 0 0 0 0 0 0 0 1 ; -",
    "3bf16e49 3d3526d1 3ebbc513 3cbe1abd 3e59f790 3e9c1702 3f0f4fc4 3c1a7a8a 3cb91ef1 3fab869e 3de2e44a 418882f8 \
         3d94149a 3f3d1117 3fcf3400 3f7adbc8 3faeb234 3e12aa09 3e23a26d 3ea6115a 3e239c81 3f260be1 3d531c05 4033f6c1 \
         3d2a31b7 3e413460 3d5326c0 3f81621b 3d5d04ec 3d5231db 3d2b8560 4002201d 3d099572 3f418251 3d6956e0 3fda5b42 \
         3d6feda4 3f7abd8d 3d48f4ae 400524b6 3d6c4361 3f605f57 3d58050a 3fac2dd3 3ca4a07f 3e3b2a71 3e2b5f2e bc0afbf5 \
         be076a9b 40391b06 bf800000 7fc00000 bdbd46c4 3f616d87 3bbb2eeb 410dd265 3e8bedcc 3d78cd97 417bb749 4101a69a \
         4177a35c bcbb50b3 3da1fd55 3e38e5b0 bde08018 3eba4c8f befda548 be840066 3c95b2ae bf02fe58 be1ff0ec 3f583b39 \
         3f800000 3f800000 3e79ad60 3f6a99d7 3d50a3c4 ; 0 0 1 0 1 0 1 0 0 1 1 0 1 1 0 0 | 3e22b5ca bc09fe13 be58c506 \
         402dd719 7fc00000 ffc00000 00000000 411dd265 00000000 00000000 4185dba4 00000000 4177a35c 3ced824f 3ce54fbe \
         3e38e5b0 ; 0 0 0 0 0 1 1 0 0 ; E0",
    "3d3f9aa0 3d0f6a7e 3e9a8108 3cc1e46b 3eb2043a 3e6d19df 3fe4a667 3c84b90a 3bada23a 3f53a5cf 3e4482cc 417a4ed6 \
         3da43d5b 3fc72ff8 401a77a8 400731e8 3fbb2d0e 3e12a85d 3de80f9d 3f1c9c2e 3e1f6a46 3f8c4c46 3cf4ee0a 403e8696 \
         3c6d635d 3f4d4a7c 3d1bdab9 3fa5c9de 3d45854c 3f241441 3c528e80 401a8c54 3c57da79 3f2bf1a8 3c04454e 403c949e \
         3cac65f4 3e04f2a1 3b7c3472 40376e02 3d70eeeb 3eb8dc12 3ce7adc5 40285102 3d638203 3e0788fa bd90532c 3b02e905 \
         3d8d05e8 4042b76d bf800000 3f7f8abd bd932c21 3fa84ef4 bf63338a 4191d027 3f2da21a bd3358e0 412a05e7 4235ccd9 \
         41875e8e beb7d0be bc9b5698 3e4279fe 3ddc8a16 be95e33e 3e839e5a 3e9cb40c be1a61ae bd096aa0 3e151d65 3f7a2a72 \
         00000000 3f800000 be5a142c 3f4abc12 3e79978e ; 0 0 1 1 0 0 0 1 1 1 1 0 0 1 0 0 | bd8bf2af 3b146570 bd1936ba \
         40438ae6 3f6ea442 bd775dd1 bf63338a 00000000 3f26462d bc7e3fde 413a05e7 00000000 41875e8e bec18628 bc9b5698 \
         3e4279fe ; 0 1 0 1 0 0 0 0 1 ; -",
    "3d3abacd 3c9f7193 3eb9c72d 3d349447 3ef9f83b 3ec2cfa0 3fd462f0 3d06d1ad 3c34e061 3fd54a11 3e297512 422852c1 \
         3d98c26b 3f2ebb89 4024271f 40385111 4023caad 3f0975fd 3d9f1200 3f4a7106 3e3c3595 3f8ebf9d 3ba2bcce 402f43b3 \
         3cd8b636 3fe16757 3cef9717 401d558f 3adf6d9d 3e51e6ca 3d4492d5 4025b63f 3c29f6c8 3f8937c0 3c0a1e08 40215cf6 \
         3c12276e 3fd39190 3d4e6ec0 40142fb8 3d296b82 3fc9c2e5 3d6943a1 4041b067 3ba9210d bde437af 3c830e54 bcabe49c \
         3dbb77e6 3f8e343d 3f800000 402fef68 3cc48365 bf40d25b bef34483 41cadccf 3f6688cf bd6f0299 41cc2f62 428794e7 \
         417ba0e2 bedaa12d bd74f7b8 3f0930fc 3aa66a1a bd49d7e9 3f0143b8 3eb88fe9 bf064af6 3eb89ca5 bf453f4c 3d0a2b53 \
         bf800000 3f800000 bf1dfa0a 3f1e2e5c 3e82a4a6 ; 1 0 0 0 0 0 0 1 1 2 1 0 1 1 0 0 | bb48bd57 bc8d2ae5 3d6d0670 \
         3f97b12a 403029d6 bc7da260 00000000 00000000 3f5af8ed bd08d574 41d42f62 00000000 417ba0e2 bf4b42d7 bf09f084 \
         3f0930fc ; 0 0 1 0 0 0 0 0 0 ; -",
    "3cbdc7e1 3ce8e39d 3f0eedc9 3cc31b90 3ea89bad 3e82dbff 3f762ec0 3cd5ad1d 3d028417 3fe03385 3ce46bd9 41c95c4a \
         3c94c503 400e0426 3f601935 4025d0b1 40245943 3f276778 3e05f0c6 3f76fa5e 3d460989 3d449e1f 3b6a2a1b 3fd86ccb \
         3c3afb7a 3dc52636 3ce40877 3f463321 3cd887a5 3fd4a21c 3d1d5bac 402c8772 3cf5a604 3fbb5284 3c88b8fe 4007ab6f \
         3d752f1a 402c137e 3d2df388 404beaeb 3c9af70e 3f3c246e 3b8a10a5 3f6cc812 3d3c52aa bd232f5a 3e54f0e4 bc39efad \
         3e3b9d3a 4031d296 bf800000 c0149feb 3e280c21 bdf5dd3e 3ff5f19b 413ec382 3d959521 3cd41953 41bad7b5 41971307 \
         4027511e bf2adbab 3dbe15eb 3f420ef1 be1c787a 3e3098de beeb9e14 bdc2c041 3dbe94a6 3ec87c0b 3f667398 be2a45f5 \
         bf800000 40000000 3f7dde2e 3f1a1de2 3d3d22f1 ; 0 1 0 0 0 1 0 0 0 1 1 1 0 1 0 0 | 3e3e2e3e bc319318 3d67cc06 \
         c051b36c bffef16c 3e2787f7 3ff5f19b 00000000 00000000 00000000 00000000 00000000 00000000 bef6ac60 3dbe15eb \
         00000000 ; 1 1 0 0 0 1 0 0 0 ; -",
    "3c3883fa 3c9cacf8 3e03b77b 3ca69aea 3eea7e19 3da2476e 3f6ac8ed 3d0657b4 3d01227d 3fd86039 3d6e2bd8 424a3f40 \
         3d9b31d5 3ec418cd 3fd2615e 401eed25 3f5414b0 3f5c1479 3e3998d5 3f71cd14 3cc77fde 3f8fd008 3d032784 3fdb7f93 \
         3bdaad35 3f770953 3cf95b52 4021a213 3bf4f48e 3ed487e9 3badb95b 3fe899f1 3cfbc1d0 3f70d243 3c729705 401f3a2e \
         3ccb7386 3e31ed60 3cde03b4 3f324a67 3d0929d8 3fbc2992 3bcd267f 402e3b46 3d50ec35 00000000 3e907b34 3b239dfa \
         bd1948c0 3f65cb53 bf800000 406d7303 bd0debb2 bffa849a bfb14818 41c46c98 3f3c0414 3d2e2b5b 40964aa1 4214b36c \
         3f5b2b7e bf63c655 bd233851 3f775941 3d6a5eed 3f09261c 3da6e6b6 3a94cd61 3edba8d5 be979602 be855144 3f500d35 \
         3f800000 40000000 be6c3930 3f26125d bdcc246d ; 0 0 1 0 0 0 0 1 0 2 1 0 0 1 0 0 | 3e900ae8 bb43a8ae bcb5da9a \
         3f56c5cd 40579aac be8c6137 00000000 00000000 3f4ef463 3d02c7b5 40d64aa1 00000000 3f5b2b7e bf0bdd38 bea561c6 \
         3f775941 ; 0 0 1 0 0 0 0 0 0 ; -",
    "3c91f4c0 3d0f8590 3f0682c9 3cf54983 3f0999f2 3e58fe87 3fbdbcf3 3d2d4c8f 3ca12e5f 3fde6a25 3d50963d 41daf6ff \
         3cd94aa4 3f063dc3 3f9551d6 3fe11886 3f97cf65 3e9df914 3dcdc85c 3eef77da 3e4b917d 400b196d 3c491d2e 4030af9d \
         3cbc3b9b 3e6e7ef6 3cb254fb 400d96eb 3c86ee82 3d735797 3c80f18e 3f152261 3cc125b1 401278af 3c84087a 403d254a \
         3cb17c99 3efb8678 3cb0d17d 403fd275 3d3c2ce4 401429d1 3aa06937 4039c4a4 3c9e4f71 00000000 be800d03 3d1ca022 \
         bd8f27c9 400a9afb 3f800000 c063e348 bd2db72c bf85fd83 bfeb192c 4090d59a 3f186b9a bdbfc906 4115e4f8 41ed7852 \
         41828e8f bf5d2ea6 3d29408a 3e855203 3db8b6a8 3e08c684 bedcff48 3ed5f098 bc27579a bf0fa70c 3f535714 3d71d5be \
         3ef8e265 3f800000 3dee0fc3 3eb4ed5c be682e31 ; 1 1 0 0 0 0 0 1 0 0 0 0 0 0 0 1 | be5b7927 3d08d15b 3d33944d \
         400d694c c06618f5 bce0a909 3f43792b 00000000 00000000 00000000 4125e4f8 41f57852 00000000 3c3ba8d0 be5c73a4 \
         3e855203 ; 1 1 0 0 1 1 0 1 0 ; E1",
    "3d017461 3d29bac3 3f0590ca 3c96b4df 3f15c2eb 3e0ccc16 3f38aa07 3cfcb001 3d28afbc 3fd6239f 3db623ba 41aad6ae \
         3cc24bed 3f7a5e85 3fae908c 40101f11 4006fc97 3e25d209 3daf7ad1 3e997764 3db7f35a 3f21aeb6 3cac77f2 3fd642ea \
         3d6cb14d 3fb4f8db 3cd900ad 400efb17 3b0601da 3fdf2fc7 3d1bc10e 400e0ac3 3d279a60 401eecb7 3d40e220 40456690 \
         3d4de541 4038495f 3cfaa815 403a5b80 3cf56873 3ecd67ef 3c74498f 3f125eec 3d0b7a00 bd2714d3 be33e207 bd055579 \
         3e316e8b 3e4e53cf 3f800000 401824c9 bd65d887 bfb0fd79 bf38b0ca 40d696ff 3e8c4c30 3dc5feb3 41d02c69 4185b8ce \
         41c55749 bf243ac4 bd32eeca 3de988ae bb2981ac be799793 bf1246b2 3ef53020 be47b749 be9d8fa5 3f3cc3e4 3f119e0c \
         3e9df4dd 3f800000 bf7a95c5 3f0932c8 bcad1c9a ; 0 1 0 1 1 0 0 1 0 1 0 0 1 0 0 0 | be5133e2 bccc2407 ba512d51 \
         3e4d82a2 4015217d bd1ef56e bf38b0ca 00000000 00000000 00000000 41d82c69 00000000 41c55749 be6bc6cd be80b530 \
         3de988ae ; 0 1 1 1 0 1 0 1 0 ; E0",
    "3c897a78 3c96d4c7 3ec8204f 3bc92cef 3ee76b70 3dbe693e 3fe359a0 3c598850 3c2f1fb7 3fb7dcaf 3dbe7033 41e64b63 \
         3d415b45 3f02d97f 3f957683 3fd8d221 3ff7dae0 3f07951d 3e316248 3f237296 3e053b08 3f3e9800 3d360d51 402f771f \
         3d4730c5 3f8161c5 3c9ad2b8 4038de4d 3d62c77a 3f32b42e 3d5a1f4b 404c767e 3c03a926 3f4412ce 3c7f18b0 4014cd89 \
         3bbebdb0 3facb32c 3d422277 402a6d99 3bf7bb86 3fa4fa6b 3a8ba8f0 400c54ff 3c4586b7 3d956884 3e7cbf60 3d3fd905 \
         be1d8729 3d3a270a bf800000 3cb68c00 3d981c78 bf9f3a70 bfdbe097 404bda50 3d19565c 3c295133 41771d9d 421f3f70 \
         408cb150 bf50c23b bd728dbb 3f571849 3e48d3d3 bf1587e4 3e770699 3ed383de 3e9d2ce0 3f2a861b bd3a7282 bf2da01f \
         3f800000 3f800000 beb488d9 3dadb8ca 3e521138 ; 0 1 0 1 1 0 0 0 1 0 0 0 0 0 1 0 | 3e937b3c 3d131587 be1042b7 \
         bdd09149 3da9de05 3d43fc49 bfdbe097 00000000 00000000 00000000 41838ece 00000000 408cb150 bf5feb17 bd728dbb \
         3f571849 ; 0 1 0 1 0 1 0 0 1 ; E0",
    "3ca33426 3b980933 3e28c234 3d03dc94 3d6fbd25 3e37554c 40038b3c 3bed09cb 3cf3ed69 3fe64019 3d1298c2 41ea7056 \
         3d881802 3f3b1777 3ff30be7 3f7e9dda 4006c3da 3e224775 3df52944 3f2104a8 3e131a61 3f9f02c3 3b9a553e 3fc5a37e \
         3b3a63fc 3eb2a9bd 3c7ad9d0 3fd1dc67 3d35d1ff 3fa06763 3d5e5e9c 3fac5daf 3cf15609 3fd505bc 3d7026bd 403e4ebd \
         3bf8834a 3fc4523d 3d175b3a 4005d69e 3b91c3ec 3f2dff90 3d381212 403b619f 3b87f683 00000000 be1a2cd1 bd39b6a2 \
         bdb29de8 3e7dfb4e bf800000 c050cc32 be141ba3 be4b6242 bf2d1786 41afe5b1 3f059dcb 3db70812 418d91f5 42814d5f \
         4163e68d bf247cb6 bd1a2b6a 3d09632e 3d212c81 3eb42edc 3efefa43 be3610f7 beb6b09f bf539e34 3e99a316 bea15ad4 \
         bf800000 3fc23c0a 3ea628dd 3ec22369 be95c6b7 ; 0 0 0 0 1 0 0 0 0 0 1 1 1 0 0 0 | be551a7d bcfe9fd1 bd707c4d \
         3e0da8f3 c05ba4f5 bda635c0 bf2d1786 00000000 00000000 00000000 4199b5b6 00000000 00000000 bf331bd8 bd1a2b6a \
         3d09632e ; 1 0 0 1 0 0 0 0 0 ; X",
    "3c04483c 3d44c32b 3e3d49a2 3d11dfe1 3d9f2b97 3e48b59c 3f4b7f5e 3d30ed95 3c975f81 3f7c54cd 3da1d91d 4187e94b \
         3d42ac4b 3ff630c3 4001c463 4002fd1a 3fba452b 3f3e6e0f 3d36e865 3f50ac7c 3d93e9c6 3ed324ac 3cf73590 3fc50f65 \
         3c97af23 3fcfcfb7 3c870c35 402d6036 3d15ddd7 400f29cf 3c3acd44 40147ed7 3c88f51c 3f442560 3cf6d0fa 4042f2c0 \
         3d207752 3f208b85 3d1a701c 400be9d0 3a150a8c 3db0511f 3b15998d 3f54990f 3c5ac95f bdb5a04c 3e1d2800 3d291e46 \
         be0ae9b2 c007ce50 bf800000 be02fe6e bd6f0b33 bf25a063 bcb74ff4 4080072b 3f5e9ea7 3d779af6 4156b566 42662bf9 \
         41c91e11 be6c9b82 bc366424 3f3f506a bd3404a7 bd3ab29f be581480 3d567703 bf308552 3ea0c5c1 3e854980 bf193598 \
         bf800000 3f800000 3f52452b 3f1a8d54 3defbebb ; 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 | 3e42ad55 3d0408b8 bdb27f55 \
         c00ee331 be3d8bde bd646da7 00000000 00000000 00000000 00000000 00000000 00000000 41c91e11 be7801c4 bc366424 \
         3f3f506a ; 0 0 0 0 0 1 0 0 0 ; E0",
];

#[test]
fn balance_update_matches_the_executable() {
    for line in BAL_CASES {
        let c = run_bal_case(line);
        for i in (0..16).filter(|i| !matches!(i, 13 | 14)) {
            let (g, w) = (f32::from_bits(c.got[i]), f32::from_bits(c.want[i]));
            assert!(c.got[i] == c.want[i] || (g.is_nan() && w.is_nan()), "value {i}: {g} vs {w}\n{line}");
        }
        assert_eq!(c.got_flags, c.want_flags, "{line}");
    }
}
