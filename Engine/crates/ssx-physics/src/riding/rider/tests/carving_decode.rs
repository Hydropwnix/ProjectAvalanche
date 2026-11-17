use super::*;
use crate::steering::{CACHED_SPIN_ANGLE, SteeringState, kinematics, lean_input, vmx_wrap_pi};

fn slope() -> (World, Vec3) {
    let theta = 10f32.to_radians();
    (world(plane(3000.0, theta)), Vec3::new(0.0, 2000.0 * theta.tan() + 1.0, -2000.0))
}

#[test]
fn lean_input_cuts_the_transform_only_while_upright() {
    let (w, _) = slope();
    let c = &w.tuning.carving;
    let max_in = 1.5f32;
    let up = lean_input(c, PI, 1.0, max_in, 0.0, true);
    assert_eq!(up, (f64::from(max_in).sin() as f32), "upright: sin(π/2) cut by sin(max_in)");
    let down = lean_input(c, PI, 1.0, max_in, 0.0, false);
    assert!(down.abs() < 1e-6, "not upright: sin(π) = {down}");
    let a = -2.403_291_2;
    let t = c.lean_input_angle_transform.eval(a);
    assert!(t < -FRAC_PI_2);
    assert_eq!(lean_input(c, a, 1.0, max_in, 0.0, false), f64::from(t).sin() as f32);
    assert_eq!(lean_input(c, a, 1.0, max_in, 0.0, true), -(f64::from(max_in).sin() as f32));
    assert_eq!(lean_input(c, 0.4, 0.7, max_in, 0.0, false), lean_input(c, 0.4, 0.7, max_in, 0.0, true));
}

#[test]
fn slow_slip_angle_is_the_never_written_cached_spin_angle() {
    let (w, _) = slope();
    let c = &w.tuning.carving;
    let st = SteeringState::new(c);
    let fast = kinematics(&st, c, Quat::IDENTITY, Quat::IDENTITY, Vec3::new(1.0, 0.0, 0.5), 2.0, 0.0);
    assert!(fast.slip > 0.4);
    let v = Vec3::new(0.5, 0.0, 0.5) * c.stopped_threshold;
    let slow = kinematics(&st, c, Quat::IDENTITY, Quat::IDENTITY, v, 2.0, 0.0);
    assert_eq!((slow.slip, CACHED_SPIN_ANGLE), (0.0, 0.0));
}

#[test]
fn build_kinematics_leaves_the_state_alone() {
    let (w, _) = slope();
    let c = &w.tuning.carving;
    let mut st = SteeringState::new(c);
    let a = Quat::from_rotation_x(0.6);
    let k1 = kinematics(&st, c, Quat::IDENTITY, a, Vec3::X, 2.0, 0.0);
    let k2 = kinematics(&st, c, Quat::IDENTITY, a, Vec3::X, 2.0, 0.0);
    assert_eq!(k1, k2);
    assert!(k1.lean_norm > 0.0 && k1.lean_norm < 0.6 / c.equilibrium_lean_angle, "rate-limited: {}", k1.lean_norm);
    st.store_kinematics(&k1);
    assert_eq!((st.forward_sign, st.lean_norm), (k1.fwd, k1.lean_norm));
}

#[test]
fn stop_test_wrap_counts_a_sideways_stick() {
    assert_eq!(vmx_wrap_pi(FRAC_PI_2 + PI), -FRAC_PI_2);
    assert_eq!(vmx_wrap_pi(-FRAC_PI_2 + PI), FRAC_PI_2);
    assert_eq!(vmx_wrap_pi(PI), PI);
    let (w, _) = slope();
    let c = &w.tuning.carving;
    let slow = c.stop_speed_threshold * 0.5;
    let calm = c.stop_accel_threshold * 10.0;
    let i = |angle: f32| SteeringInputs { angle, mag: 1.0, slip_angle: angle, slip_mag: 1.0, ..Default::default() };
    for a in [FRAC_PI_2, -FRAC_PI_2, PI, -2.0] {
        assert!(stop_test(&w.tuning, slow, calm, &i(a), false, false, false), "{a}");
    }
    for a in [0.0, 1.0, -1.5] {
        assert!(!stop_test(&w.tuning, slow, calm, &i(a), false, false, false), "{a}");
    }
}

#[test]
fn first_contact_wiggle_turns_the_velocity_about_the_normal() {
    let (w, _) = slope();
    let curve = &w.tuning.carving.wiggle_offset_angle_curve;
    let n = Vec3::new(0.4, 1.0, 0.0).normalize();
    let v = Vec3::new(1.0, -0.4, 0.3).normalize() * 0.5;
    let v = v - n * v.dot(n);
    let v0 = Vec3::new(1.0, -0.5, 0.0);
    let out = first_contact_wiggle(curve, 1.0, v0, v, n);
    assert!((out.length() - v.length()).abs() < 1e-6 && out.dot(n).abs() < 1e-6);
    let turned = v.angle_between(out).to_degrees();
    assert!((turned - 30.0).abs() < 1e-3, "{turned}");
    assert!(v.cross(out).dot(n) < 0.0);
    let half = v.angle_between(first_contact_wiggle(curve, -0.75, v0, v, n)).to_degrees();
    assert!((half - 27.5).abs() < 1e-3, "{half}");
    assert_eq!(first_contact_wiggle(curve, 1.0, Vec3::ZERO, v, n), v);
    assert_eq!(first_contact_wiggle(curve, 1.0, Vec3::Y, v, n), v);
}

#[test]
fn the_first_contact_turns_by_the_air_wiggle() {
    let (w, at) = slope();
    let run = |wiggle: f32, tumbling: bool| {
        let mut s = moving(&w, at, Vec3::Z, 60.0 / KMH_PER_M_PER_TICK);
        s.carving.have_made_first_contact = false;
        s.carving.initial_velocity = s.velocity;
        s.air.outputs.spin_wiggle = wiggle;
        s.landing.tumbling = tumbling;
        step(&mut s, &w, &Input::default(), DT_TICKS);
        assert!(s.carving.have_made_first_contact && s.air.outputs.spin_wiggle == 0.0);
        s.velocity
    };
    let (plain, turned) = (run(0.0, false), run(1.0, false));
    let deg = plain.angle_between(turned).to_degrees();
    assert!((20.0..40.0).contains(&deg), "turned {deg}°");
    let wiping = run(1.0, true);
    assert!(run(0.0, true).angle_between(wiping).to_degrees() < 1.0, "no turn while wiping out");
}

#[test]
fn a_late_sideways_stick_lands_the_rider_turned() {
    let w = ledge(30.0);
    let run = |from: Option<usize>| {
        let mut s = moving(&w, Vec3::new(0.0, 0.5, -20.0), Vec3::Z, 0.4);
        let (mut takeoff, mut landing, mut spun) = (None, None, false);
        for i in 0..200 {
            let steer = if from.is_some_and(|f| i >= f) { 1.0 } else { 0.0 };
            let was = s.state;
            step(&mut s, &w, &Input { steer, ..Input::default() }, DT_TICKS);
            if s.state == RideState::Airborne {
                spun |= s.air.spin_input.abs() > 0.5;
            }
            match (was, s.state) {
                (RideState::Riding, RideState::Airborne) => takeoff = Some(i),
                (RideState::Airborne, RideState::Riding) => {
                    landing = Some(i);
                    break;
                }
                _ => {}
            }
        }
        let heading = s.velocity.x.atan2(s.velocity.z).to_degrees();
        (takeoff.expect("take-off"), landing.expect("landing"), heading, spun)
    };
    let (takeoff, landing, straight, _) = run(None);
    assert!(straight.abs() < 0.5, "{straight}");
    assert!(landing - takeoff > 40, "air {} steps", landing - takeoff);
    let (_, _, late, late_spun) = run(Some(landing - 10));
    assert!(!late_spun, "no spin starts close to the ground");
    assert!((-31.0..-20.0).contains(&late), "the late push lands turned right: {late}°");
    let (_, _, early, early_spun) = run(Some(takeoff + 1));
    assert!(early_spun, "an early push starts a spin");
    assert!(early.abs() < 1.0, "the realised spin leaves no wiggle: {early}°");
}

#[test]
fn the_balance_reads_the_left_stick_alone() {
    let (w, at) = slope();
    let pitch_after = |input: Input, classic: bool| {
        let mut s = moving(&w, at, Vec3::Z, 40.0 / KMH_PER_M_PER_TICK);
        s.tricks.classic = classic;
        (s.landing.pitch, s.landing.pitch_vel, s.landing.stable) = (0.6, 0.0, false);
        s.landing.landing_pitch_vel = 0.05;
        ride(&w, &mut s, &input, 3, |_| {});
        s.landing.pitch
    };
    let none = pitch_after(Input::default(), true);
    let dpad = pitch_after(Input { dpad: Vec2::new(0.0, -1.0), ..Input::default() }, true);
    let stick = pitch_after(Input { brake: 1.0, ..Input::default() }, true);
    assert_eq!(none, dpad, "the D-pad does not balance");
    assert_ne!(none, stick, "the left stick does");
    assert_eq!(balance_inputs(-1.0, true, true, false), (0.0, false));
    assert_eq!(balance_inputs(-1.0, true, true, true), (-1.0, true));
    assert_eq!(balance_inputs(-1.0, true, false, false), (-1.0, true));
}

#[test]
fn the_right_stick_down_crouches_for_the_jump() {
    let (w, at) = slope();
    let height = |rstick: f32| {
        let mut s = moving(&w, at, Vec3::Z, 40.0 / KMH_PER_M_PER_TICK);
        s.jump.rstick_y_filtered = rstick;
        step(&mut s, &w, &Input::default(), DT_TICKS);
        s.carving.riding_height
    };
    let (neutral, down, up) = (height(0.0), height(-1.0), height(1.0));
    assert_eq!(neutral, up, "pushed up: −y < 0 clamps to 0");
    assert_ne!(neutral, down);
}

#[test]
fn the_carve_stick_axes_follow_the_lua() {
    let diag = Input { steer: 1.0, tuck: 1.0, ..Input::default() };
    let h = std::f32::consts::FRAC_1_SQRT_2;
    assert!((diag.carve_stick_y(false) - h).abs() < 1e-6);
    assert!((diag.carve_stick(false) - Vec2::splat(h)).length() < 1e-6);
    let classic = Input { dpad: Vec2::new(1.0, 0.0), ..diag };
    assert!((classic.carve_stick_y(true) - h).abs() < 1e-6);
    let c = classic.carve_stick(true);
    assert!((c - Vec2::new(1.0, h).normalize()).length() < 1e-6, "{c:?}");
}

#[test]
fn the_scalar_wrap_is_the_double_one() {
    use crate::steering::wrap_pi_f64;
    assert_eq!(wrap_pi_f64(4.0), (4.0f64 - 6.283_185_482_025_146_5) as f32);
    assert_eq!(wrap_pi_f64(-4.0), (-4.0f64 + 6.283_185_482_025_146_5) as f32);
    assert_eq!(wrap_pi_f64(0.25), 0.25);
    assert_eq!(wrap_pi_f64(-1.0), -1.0);
    assert!((wrap_pi_f64(1.5 * PI) + FRAC_PI_2).abs() < 1e-6);
    assert!((wrap_pi_f64(-1.5 * PI) - FRAC_PI_2).abs() < 1e-6);
    assert!((wrap_pi_f64(7.0 * 2.0 * PI + 0.5) - 0.5).abs() < 2e-5);
}

#[test]
fn the_pid_steps_like_the_executable() {
    use crate::character::PidGains;
    use crate::steering::Pid;
    let g = PidGains { p: 1.5, i: 0.25, d: 1.0 };
    let mut p = Pid::new(g, 0.32);
    p.update(2.5, 0.5);
    let (mut x, mut rate, mut integral, mut x_prev) = (0f32, 0f32, 0f32, 0f32);
    for h in [1.0f32, 1.0, 0.5] {
        let e = 0.5 - x;
        let d = (x_prev - x) / h;
        x_prev = x;
        integral = e.mul_add(h, integral);
        let u = g.i.mul_add(integral, g.d.mul_add(d, g.p * e));
        let out = if u.abs() <= 1.192_092_9e-7 { 0.0 } else { u.signum() * u.abs().min(1.0) };
        integral -= ((u - out) / g.i).max(integral.min(0.0)).min(integral.max(0.0));
        let old = x;
        x = (0.32 * out).mul_add(h, rate).mul_add(h, x).clamp(-1.0, 1.0);
        rate = (x - old) / h;
    }
    assert_eq!((p.x, p.rate), (x, rate));
    let before = p.x;
    p.update(1.525_878_9e-5, 1.0);
    assert_eq!(p.x, before);
}

#[test]
fn friction_follows_frictionaccel() {
    use crate::ground::friction_accel;
    let (w, _) = slope();
    let t = &w.tuning;
    let snow = t.surfaces.records[0];
    assert_ne!(friction_accel(t, &snow, Vec3::new(1e-6, 0.0, 0.0), 0.0, 0.0, 1.0, 0.0, 0.0, false), Vec3::ZERO);
    assert_eq!(friction_accel(t, &snow, Vec3::new(1e-7, 0.0, 0.0), 0.0, 0.0, 1.0, 0.0, 0.0, false), Vec3::ZERO);
    let plus = friction_accel(t, &snow, Vec3::new(1.0, 0.0, 0.5), 0.0, 0.3, 1.0, 0.0, 0.0, false);
    let minus = friction_accel(t, &snow, Vec3::new(1.0, 0.0, -0.5), 0.0, 0.3, 1.0, 0.0, 0.0, false);
    assert_eq!(plus.z, 0.0);
    assert!(minus.z > 0.0, "{minus:?}");
    let rock = friction_accel(t, &snow, Vec3::new(1.0, 0.0, 0.0), 0.0, 0.0, 1.0, 1.0, 0.0, false);
    assert_eq!(rock.x, 0.0, "no longitudinal friction on rock");
    let half = friction_accel(t, &snow, Vec3::new(1.0, 0.0, 0.0), 0.0, 0.0, 1.0, 0.5, 0.0, false);
    let none = friction_accel(t, &snow, Vec3::new(1.0, 0.0, 0.0), 0.0, 0.0, 1.0, 0.0, 0.0, false);
    assert!((half.x - 0.5 * none.x).abs() <= f32::EPSILON * none.x.abs(), "{half:?} {none:?}");
}

#[test]
fn a_straight_back_stick_is_plus_pi() {
    use crate::steering::stick_angle_and_magnitude;
    let m = crate::graph::stick_axes(Input { brake: 0.5, ..Input::default() }.stick());
    assert_eq!(m.x, 0.0);
    let (angle, _) = stick_angle_and_magnitude(m);
    assert_eq!(angle, PI);
}

#[test]
fn tiny_is_two_to_the_minus_16() {
    assert_eq!(crate::rider::TINY.to_bits(), 0x3780_0000);
}

#[test]
fn the_twist_is_the_executables() {
    use crate::steering::twist;
    for k in -179..=179 {
        let a = (k as f32 * 2.0).to_radians() * 0.5 + 0.013;
        let q = Quat::from_axis_angle(Vec3::X, a);
        let host = 2.0 * q.x.atan2(q.w);
        assert!((twist(q, Vec3::X) - host).abs() < 1e-6, "{a}: {} against {host}", twist(q, Vec3::X));
        let t = twist(q, Vec3::X);
        assert!(t > -PI && t <= PI);
    }
    assert_eq!(twist(Quat::from_xyzw(1.0, 0.0, 0.0, 0.0), Vec3::X), PI);
    assert_eq!(twist(Quat::from_xyzw(-1.0, 0.0, 0.0, 0.0), Vec3::X), PI);
    let q = Quat::from_xyzw(0.2, 0.0, 0.0, -0.9);
    assert!((twist(q, Vec3::X) - 2.0 * 0.2f32.atan2(-0.9) + 2.0 * PI).abs() < 1e-6);
}

#[test]
fn the_lean_bias_and_the_lean_angle() {
    use crate::steering::{lean_angle, lean_bias_angle, twist, wrap_pi_f64};
    let half = Quat::from_xyzw(std::f32::consts::FRAC_1_SQRT_2, 0.0, 0.0, 0.0);
    assert_eq!(lean_bias_angle(half, 0.3), 0.0);
    let q = Quat::from_axis_angle(Vec3::new(0.3, 0.2, 0.9).normalize(), 0.7);
    let up = q * Vec3::Y;
    let b = lean_bias_angle(q, 0.25);
    assert!((b - 0.75 * up.z.atan2(up.y)).abs() < 1e-6, "{b}");
    assert_eq!(lean_bias_angle(q, 1.0), 0.0);
    assert_eq!(lean_angle(q, b), wrap_pi_f64(twist(q, Vec3::X) - b));
    let r = Quat::from_axis_angle(Vec3::X, 3.0);
    assert!((lean_angle(r, -0.5) - (3.5 - 2.0 * PI)).abs() < 1e-5);
}

#[test]
fn the_vector_quaternion_steps_follow_the_host() {
    use crate::steering::{vmx_quat_mul, vmx_rotate};
    let qs = [
        Quat::from_axis_angle(Vec3::new(0.3, 0.2, 0.9).normalize(), 0.7),
        Quat::from_axis_angle(Vec3::new(-0.8, 0.5, 0.1).normalize(), 2.9),
        Quat::from_axis_angle(Vec3::Y, -1.3),
    ];
    for a in qs {
        for b in qs {
            let (p, h) = (vmx_quat_mul(a, b), a * b);
            assert!((p.x - h.x).abs() + (p.y - h.y).abs() + (p.z - h.z).abs() + (p.w - h.w).abs() < 1e-6);
        }
        let v = Vec3::new(-0.13, -0.61, 0.17);
        assert!((vmx_rotate(a, v) - a * v).length() < 1e-6);
    }
}

fn rock_slide_tag(value: bool, mode: u32, time: f32) -> ssx_assets::anim_graph::GameTag {
    ssx_assets::anim_graph::GameTag {
        name: "BLN_carving_state_is_in_rock_slide".into(),
        signal: 0,
        window: ssx_assets::tags::TagWindow::new(time, 0.0, false),
        value: ssx_assets::anim_graph::GameTagValue::Bool(value),
        mode,
        reset_on_leave: false,
        default: 0.0,
    }
}

#[test]
fn the_rock_bail_takes_raise_the_rock_slide() {
    assert!(carving::rock_slide_tags(false, &[rock_slide_tag(true, 1, 0.0)]));
    assert!(!carving::rock_slide_tags(true, &[rock_slide_tag(false, 1, 0.0)]), "Mode 1 writes the value");
    assert!(carving::rock_slide_tags(false, &[rock_slide_tag(false, 2, 0.0)]), "Mode 2: the saved value negated");
    assert!(!carving::rock_slide_tags(false, &[rock_slide_tag(true, 0, 0.0)]), "Mode 0 does nothing");
    assert!(!carving::rock_slide_tags(false, &[rock_slide_tag(true, 1, 5.0)]), "a later window is not at the take");
    let mut other = rock_slide_tag(true, 1, 0.0);
    other.name = "BLN_State_Bonk".into();
    assert!(!carving::rock_slide_tags(false, &[other]));
    assert!(!carving::rock_slide_tags(false, &[]));
}

#[test]
fn the_rock_slide_clears_with_the_rock_bail_and_holds_off_rock_damage() {
    let mut w = world(plane(3000.0, 0.0));
    w.survival.rock_damage = true;
    let mut s = RiderState::spawn(&w, Vec3::new(0.0, 1.0, 0.0), Vec3::Z).expect("ground");
    s.carving.have_made_first_contact = true;
    s.landing.stable = true;
    let rock = crate::surface::ROCK;
    s.weights[rock] = 0.0;
    s.landing.is_in_rock_slide = true;
    carving::rock_riding_update(&mut s, &w, 1.0, DT_TICKS);
    assert!(s.landing.is_in_rock_slide && !s.landing.bailed_because_of_rock);
    s.weights[rock] = 1.0;
    let damage = s.survival.carving_damage;
    carving::rock_riding_update(&mut s, &w, 1.0, DT_TICKS);
    assert!(s.carving.ticks_on_rock > 0.0 && !s.landing.bailed_because_of_rock);
    assert_eq!(s.survival.carving_damage, damage);
    s.landing.is_in_rock_slide = false;
    carving::rock_riding_update(&mut s, &w, 1.0, DT_TICKS);
    assert!(s.landing.bailed_because_of_rock);
    assert_eq!(s.survival.carving_damage, damage + 0.0625);
    s.landing.is_in_rock_slide = true;
    s.weights[rock] = 0.0;
    carving::rock_riding_update(&mut s, &w, 1.0, DT_TICKS);
    assert!(!s.landing.bailed_because_of_rock && !s.landing.is_in_rock_slide);
    assert_eq!(s.carving.ticks_on_rock, 0.0);
    s.weights[rock] = f32::NAN;
    carving::rock_riding_update(&mut s, &w, 1.0, DT_TICKS);
    assert!(s.carving.ticks_on_rock > 0.0);
}

#[test]
fn the_reach_value_is_the_wrapped_pitch_and_zero_while_bailing_or_uber_sliding() {
    use super::outputs::carving_reach_value;
    assert_eq!(carving_reach_value(0.3, false, false), 1.0);
    assert_eq!(carving_reach_value(FRAC_PI_2, false, false), 1.0, "`ble`: π/2 itself stays 1");
    assert_eq!(carving_reach_value(2.5, false, false), ssx_formats::vmx::sin(2.5));
    assert_eq!(carving_reach_value(-2.5, false, false), ssx_formats::vmx::sin(2.5), "the magnitude");
    assert_eq!(carving_reach_value(2.0 * PI - 0.3, false, false), 1.0);
    assert!(carving_reach_value(f32::NAN, false, false).is_nan());
    assert_eq!(carving_reach_value(2.5, true, false), 0.0);
    assert_eq!(carving_reach_value(0.3, false, true), 0.0);
    assert_eq!(carving_reach_value(f32::NAN, false, true), 0.0);
}

#[test]
fn the_velocity_angle_is_the_executables() {
    use crate::steering::velocity_angle_in_trajectory_frame as angle;
    let q = Quat::IDENTITY;
    assert!((angle(q, Vec3::new(1.0, 0.0, 1.0), 0.1, 9.0) - std::f32::consts::FRAC_PI_4).abs() < 1e-6);
    assert_eq!(angle(q, Vec3::new(0.0, 5.0, -2.0), 0.1, 9.0), -FRAC_PI_2, "x = 0: ±π/2 with z's sign");
    assert_eq!(angle(q, Vec3::new(-1.0, 0.0, 0.0), 0.1, 9.0), PI, "x < 0, z = +0: +π");
    assert_eq!(angle(q, Vec3::new(3.0, 0.0, 4.0), 5.0, 9.0), 9.0, "|l| = SpeedThreshold: the fallback");
    assert_ne!(angle(q, Vec3::new(3.0, 0.0, 4.0), 4.999, 9.0), 9.0);
    assert!(angle(q, Vec3::new(f32::NAN, 0.0, 1.0), 5.0, 9.0).is_nan());
    let r = Quat::from_rotation_y(0.7);
    let v = Vec3::new(0.3, -0.2, 0.9);
    let l = r.conjugate() * v;
    assert!((angle(r, v, 0.1, 9.0) - l.z.atan2(l.x)).abs() < 2e-6);
}

#[test]
fn the_board_angle_is_the_velocity_angle_without_a_turn_limit() {
    let (w, at) = slope();
    let mut s = moving(&w, at, Vec3::Z, 60.0 / KMH_PER_M_PER_TICK);
    s.grind.carving.board_angle = 2.0;
    let (q, v) = (s.traj_orientation * Quat::from_rotation_y(-FRAC_PI_2), s.velocity);
    let c = &w.tuning.carving;
    let expected = crate::steering::velocity_angle_in_trajectory_frame(q, v, c.speed_threshold, 2.0);
    step(&mut s, &w, &Input::default(), DT_TICKS);
    assert_eq!(s.grind.carving.board_angle, expected);
    assert!((expected - 2.0).abs() > c.max_spin_vel * DT_TICKS, "farther than one step's turn limit");
}

#[test]
fn the_ground_trajectory_wraps_the_slip_before_its_half_angle() {
    let w = world(plane(3000.0, 0.0));
    let c = &w.tuning.carving;
    let mut s = RiderState::spawn(&w, Vec3::new(0.0, 1.0, 0.0), Vec3::Z).expect("ground");
    (s.alignment, s.traj_orientation, s.landing.pitch, s.landing.spin) = (Quat::IDENTITY, Quat::IDENTITY, 0.0, 0.0);
    (s.steering.stance_override_sign, s.kin.fwd, s.kin.slip) = (1.0, 1.0, 0.0);
    let j = crate::rootmotion::trajectory_joint(s.traj_orientation);
    s.steering.slip_angle = 4.0;
    s.carving.traj_delta = super::graph_io::trajectory_delta_rotation(&s, c, j, Vec3::Y, false);
    let t = super::graph_io::carving_trajectory(&s);
    let wrapped = vmx_wrap_pi(4.0);
    assert!((wrapped - (4.0 - std::f32::consts::TAU)).abs() < 1e-6, "{wrapped}");
    assert!(t.w > 0.0, "{t:?}");
    assert!(t.abs_diff_eq(Quat::from_axis_angle(Vec3::Y, wrapped), 1e-6), "{t:?}");
    s.steering.slip_angle = 0.5;
    s.carving.traj_delta = super::graph_io::trajectory_delta_rotation(&s, c, j, Vec3::Y, false);
    let t = super::graph_io::carving_trajectory(&s);
    assert!(t.abs_diff_eq(Quat::from_axis_angle(Vec3::Y, 0.5), 1e-6), "{t:?}");
}

#[test]
fn the_ground_trajectory_turns_about_the_frame_normal() {
    let n = Vec3::new(0.3, 1.0, 0.0).normalize();
    let j = crate::rootmotion::trajectory_joint(Quat::IDENTITY);
    let stance = -FRAC_PI_2;
    let d = super::graph_io::ground_delta(j, Quat::IDENTITY, [0.0, 0.0, stance], 0.7, n);
    let t = crate::rootmotion::world_motion(Quat::IDENTITY, d.to_array(), [0.0; 3]).1;
    assert!(t.abs_diff_eq(Quat::from_axis_angle(n, 0.7), 2e-6), "{t:?}");
}

#[test]
fn the_uber_slide_turns_the_trajectory_joint_towards_the_normal() {
    use super::graph_io::uber_slide_delta;
    let n = Vec3::Y;
    let step = |q: Quat, max: f32, frames: i32| q * uber_slide_delta(q, n, max, frames);
    let t = step(Quat::IDENTITY, 0.1, 2);
    assert!(((t * Vec3::X).angle_between(n) - (FRAC_PI_2 - 0.1)).abs() < 1e-4, "{t:?}");
    assert_eq!(uber_slide_delta(Quat::IDENTITY, n, 0.1, 1), Quat::IDENTITY);
    assert_eq!(uber_slide_delta(Quat::IDENTITY, n, 0.1, 0), Quat::IDENTITY);
    assert_eq!(uber_slide_delta(Quat::from_rotation_z(FRAC_PI_2), n, 0.1, 5), Quat::IDENTITY);
    let mut q = Quat::IDENTITY;
    for _ in 0..20 {
        q = step(q, 0.1, 9).normalize();
    }
    assert!((q * Vec3::X).angle_between(n) < 1e-3, "{q:?}");
    let t = step(Quat::IDENTITY, -0.1, 2);
    assert!(((t * Vec3::X).angle_between(n) - (FRAC_PI_2 + 0.1)).abs() < 1e-4, "{t:?}");
    assert_eq!(uber_slide_delta(Quat::from_rotation_y(0.3), Vec3::NAN, 0.1, 9), Quat::IDENTITY);
}

#[test]
fn uber_bail_sliding_is_the_trigger_through_the_gate_or_the_board_particle() {
    let w = world(plane(3000.0, 0.0));
    let mut s = RiderState::spawn(&w, Vec3::new(0.0, 1.0, 0.0), Vec3::Z).expect("ground");
    s.landing.uber_crash = true;
    for (timer, open) in [(5.0, false), (0.0, true), (-1.0, true), (f32::NAN, true)] {
        s.uber_gate_timer = timer;
        assert_eq!(s.uber_bail_sliding(), open, "timer {timer}");
    }
    (s.landing.uber_crash, s.uber_gate_timer, s.board_particle.enabled) = (false, 5.0, true);
    assert!(s.uber_bail_sliding());
    s.board_particle.enabled = false;
    assert!(!s.uber_bail_sliding());
}

#[test]
fn the_carving_rotations_are_the_vector_units_within_rounding_of_glams() {
    use crate::steering::{apply_rotation, rates_to_rotation, vmx_axis_angle, vmx_frame_to_world};
    let w = world(plane(3000.0, 0.0));
    let c = &w.tuning.carving;
    let axis = Vec3::new(0.3, 0.8, -0.52).normalize();
    let mut differ = 0;
    for i in 0..200 {
        let x = (i as f32 - 100.0) * 0.0371;
        let (q, g) = (vmx_axis_angle(axis.to_array(), x), Quat::from_axis_angle(axis, x));
        assert!(q.abs_diff_eq(g, 1e-6), "{x}: {q:?} {g:?}");
        differ += usize::from(q.to_array().map(f32::to_bits) != g.to_array().map(f32::to_bits));
    }
    assert!(differ > 0, "the vector unit's series rounds differently from libm somewhere");
    let dt = 1.0 / 30.0;
    let r = rates_to_rotation(c, dt, 0.4, -0.7, 0.2);
    let g = Quat::from_rotation_y(c.max_spin_vel * dt * -0.7)
        * Quat::from_rotation_z(c.max_pitch_vel * dt * 0.2)
        * Quat::from_rotation_x(c.max_lean_vel * dt * 0.4);
    assert!(r.abs_diff_eq(g, 1e-6), "{r:?} {g:?}");
    let a = Quat::from_euler(glam::EulerRot::YXZ, 0.7, -0.2, 0.1);
    let q = Quat::from_euler(glam::EulerRot::YXZ, -1.1, 0.3, 0.05);
    let dq = Quat::from_rotation_x(0.04);
    assert!(vmx_frame_to_world(dq, q).abs_diff_eq(q * dq * q.conjugate(), 1e-6));
    let applied = apply_rotation(a, q, dq);
    assert!(applied.abs_diff_eq((q * dq * q.conjugate() * a).normalize(), 2e-6), "{applied:?}");
}

#[test]
fn the_stopped_latch_takes_full_friction_unless_below_the_limit() {
    use super::ride::full_friction;
    assert!(full_friction(5.0, 5.0) && full_friction(6.0, 5.0) && !full_friction(4.0, 5.0));
    assert!(full_friction(f32::NAN, 5.0) && full_friction(4.0, f32::NAN));
}

#[test]
fn a_new_controller_starts_from_the_state_init() {
    let w = world(plane(3000.0, 0.0));
    let mut s = RiderState::spawn(&w, Vec3::new(0.0, 1.0, 0.0), Vec3::Z).expect("ground");
    let ia = &mut s.ice_axe;
    (ia.steering_value, ia.noise, ia.effects_timer, ia.chopped) = (0.3, [0.2, 0.1], 5.0, true);
    (s.press.force_stop_mag, s.press.force_stop_vel, s.press.blend) = (0.8, 0.01, 0.5);
    crate::rider::carving::new_carving(&mut s, &w, DT_TICKS);
    let ia = &s.ice_axe;
    assert_eq!((ia.steering_value, ia.noise, ia.effects_timer, ia.chopped), (0.0, [0.0; 2], 0.0, false));
    assert_eq!((s.press.force_stop_mag, s.press.force_stop_vel, s.press.blend), (0.0, 0.0, 0.0));
    assert!(!s.carving.scoring_vertical);
}

#[test]
fn a_long_air_onto_rock_starts_sixteen_ticks_on_rock() {
    let w = world(plane(3000.0, 0.0));
    let cases = [
        (0.8, 25.0, 16.0),
        (0.7, 20.5, 16.0),
        (0.8, 20.0, 0.0),
        (0.69, 25.0, 0.0),
        (f32::NAN, 25.0, 16.0),
        (0.8, f32::NAN, 0.0),
    ];
    for (rock, air, want) in cases {
        let mut s = RiderState::spawn(&w, Vec3::new(0.0, 1.0, 0.0), Vec3::Z).expect("ground");
        (s.weights[crate::surface::ROCK], s.score.air_time) = (rock, air);
        crate::rider::carving::new_carving(&mut s, &w, DT_TICKS);
        assert_eq!(s.carving.ticks_on_rock, want, "rock {rock}, air {air}");
    }
}

#[test]
fn the_vertical_flags_are_the_vector_cosine_against_the_normal() {
    use crate::ground::{scoring_vertical_surface, vertical_surface};
    let c = &crate::testutil::tuning().carving;
    let n = |y: f32| Vec3::new(0.0, y, (1.0 - y * y).sqrt());
    assert!(vertical_surface(c, n(0.49)) && !vertical_surface(c, n(0.51)) && !vertical_surface(c, n(1.0)));
    assert!(scoring_vertical_surface(c, n(-0.45), true) && !scoring_vertical_surface(c, n(-0.45), false));
    assert!(!scoring_vertical_surface(c, n(-0.44), true) && !scoring_vertical_surface(c, n(0.3), true));
    let nan = Vec3::new(0.0, f32::NAN, 1.0);
    assert!(!vertical_surface(c, nan) && !scoring_vertical_surface(c, nan, true));
}
