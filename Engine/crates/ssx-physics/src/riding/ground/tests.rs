use super::*;
use crate::testutil::{balance, tuning};

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-5
}

#[test]
fn riding_height_target_blends_and_keeps_the_board_sphere_clear() {
    let c = tuning().carving;
    let h = |b: HeightBlends, lean: f32| riding_height_target(&c, b, lean, 0.0, false, 0.855, false);
    assert!(close(h(HeightBlends::default(), 0.0), -0.83));
    assert!(close(h(HeightBlends { tuck: 1.0, ..HeightBlends::default() }, 0.0), -0.73));
    assert!(close(h(HeightBlends { tuck: 0.5, ..HeightBlends::default() }, 0.0), -0.78));
    assert!(close(h(HeightBlends { tuck: 1.0, pre_jump: 1.0, boost: 0.0 }, 0.0), -0.6));
    let lean = 60f32.to_radians();
    assert!(close(h(HeightBlends::default(), lean), -0.415));
    let deep = 85f32.to_radians();
    let rad = 0.13 + 0.25 * deep.cos();
    assert!(close(h(HeightBlends::default(), deep), (deep.cos() * -0.83).min(-rad)));
    assert!(close(riding_height_target(&c, HeightBlends::default(), lean, 0.0, true, 0.855, true), -0.855));
    let at = |ticks: f32| riding_height_target(&c, HeightBlends::default(), 0.0, ticks, true, 0.855, false);
    assert!(close(at(0.0), -0.855) && close(at(4.0), -0.8425) && close(at(8.0), -0.83) && close(at(50.0), -0.83));
}

#[test]
fn riding_height_rate_limit_floor_and_range() {
    let c = tuning().carving;
    assert!(close(update_riding_height(&c, -0.9, -0.6, None, true, 2.0), -0.6));
    assert!(close(update_riding_height(&c, -0.9, -0.6, None, false, 2.0), -0.7));
    assert!(close(update_riding_height(&c, -0.6, -0.95, None, false, 1.0), -0.7));
    assert!(close(update_riding_height(&c, -0.83, -0.83, Some(-0.5), false, 2.0), -0.5));
    assert!(close(update_riding_height(&c, -0.83, -0.83, Some(0.2), false, 2.0), -0.38));
    assert!(close(update_riding_height(&c, -1.0, -1.4, None, false, 20.0), -1.0));
}

#[test]
fn snow_compression_restores_with_distance_carved() {
    let t = tuning();
    let snow = t.surfaces.records[0];
    assert_eq!(snow_compression(&t.carving, &snow, 0.0), 0.0);
    assert!(close(snow_compression(&t.carving, &snow, 100.0), 0.05));
    assert!(close(snow_compression(&t.carving, &snow, 1000.0), 0.05));
    let ice = SurfaceProperties { snow_depth: 0.0, riding_depth: 0.0, ..snow };
    assert_eq!(snow_compression(&t.carving, &ice, 100.0), 0.0);
}

#[test]
fn brake_factor_priorities() {
    let t = tuning();
    let tumble = balance().tumble_friction_curve;
    let base = BrakeInputs { can_brake: true, slip_angle: 0.0, slip_mag: 0.0, ..BrakeInputs::default() };
    assert_eq!(brake_factor(&t, &tumble, &base), 0.0);
    let full = t.carving.uber_crash_ticks_to_full_friction;
    let uber = BrakeInputs { uber: true, ride_ticks: full, ..base };
    assert_eq!(brake_factor(&t, &tumble, &uber), 1.0);
    assert_ne!(brake_factor(&t, &tumble, &BrakeInputs { vertical: true, ..uber }), 1.0);
    assert_eq!(brake_factor(&t, &tumble, &BrakeInputs { dead_on_ground: true, ..base }), 1.0);
    assert_eq!(brake_factor(&t, &tumble, &BrakeInputs { board_press: true, ..base }), 0.1);
    let rock = BrakeInputs { grind_or_rock: true, ..base };
    assert_eq!(brake_factor(&t, &tumble, &rock), t.carving.board_press_friction_modifier);
    let fallen = BrakeInputs { can_brake: false, pitch: 1.0, ..base };
    assert_eq!(brake_factor(&t, &tumble, &fallen), tumble.eval(1.0));
    let stick = BrakeInputs { slip_angle: -2.0, slip_mag: 0.5, ..base };
    assert!(close(brake_factor(&t, &tumble, &stick), t.carving.slip_friction_input_curve.eval(2.0) * 0.5));
}

#[test]
fn friction_opposes_the_velocity_and_never_reverses_it() {
    let t = tuning();
    let snow = t.surfaces.records[0];
    let v = Vec3::new(0.3, 0.0, 0.0);
    let a = friction_accel(&t, &snow, v, 0.0, 0.0, 1.0, 0.0, 0.0, false);
    assert!(a.distance(Vec3::new(-0.03 * 0.015, 0.0, 0.0)) < 1e-9, "{a:?}");
    let a = friction_accel(&t, &snow, v, 0.0, 0.0, 0.25, 0.0, 1.0, false);
    assert!(close(a.x, -0.7 * 0.5 * 0.015));
    let side = Vec3::new(0.3, 0.0, 0.1);
    let lean = 0.5f32;
    let against = friction_accel(&t, &snow, side, -lean, 0.0, 1.0, 0.0, 0.0, false);
    let k = side.normalize();
    assert!(close(against.z, -k.z * lean.sin() * 5.6 * 0.015));
    let towards = friction_accel(&t, &snow, side, lean, 0.0, 1.0, 0.0, 0.0, false);
    assert_eq!(towards.z, 0.0);
    assert_eq!(friction_accel(&t, &snow, Vec3::ZERO, 0.0, 0.0, 1.0, 0.0, 1.0, false), Vec3::ZERO);
    let slow = Vec3::new(1e-3, 0.0, -1e-3);
    let a = clamp_not_reversing(Vec3::new(-0.01, 0.0, 0.01), slow, 2.0);
    assert!((slow + a * 2.0).abs().max_element() < 1e-9);
    let a = clamp_not_reversing(Vec3::new(-1e-4, 0.0, 0.0), slow, 2.0);
    assert!(close(a.x, -1e-4));
}

#[test]
fn adhesion_fades_with_height_and_stops_while_bailing() {
    let t = tuning();
    let (c, snow) = (&t.carving, t.surfaces.records[0]);
    let a = |kin34: f32, stable: bool, pitch: f32, bail: bool, first: bool| {
        adhesion_accel(c, &snow, 1.0, 1.0, kin34, stable, pitch, bail, first)
    };
    assert!(close(a(0.0, true, 0.0, false, true), -0.0125));
    assert!(close(a(-0.45, true, 0.0, false, true), -0.00625));
    assert_eq!(a(-0.9, true, 0.0, false, true), 0.0);
    assert!(close(a(0.0, false, std::f32::consts::FRAC_PI_4, false, true), -0.0125 * 0.5));
    assert_eq!(a(0.0, true, 0.0, true, true), 0.0);
    assert_eq!(a(0.0, true, 0.0, false, false), 0.0);
}

fn flat_step(st: &CarvingState, v: Vec3, d: f32, contact: bool, stopped: bool) -> GroundOutput {
    flat_step_boost(st, v, d, contact, stopped, BoostThrust::default())
}

fn flat_step_boost(
    st: &CarvingState,
    v: Vec3,
    d: f32,
    contact: bool,
    stopped: bool,
    boost: BoostThrust,
) -> GroundOutput {
    let t = tuning();
    let r = ResolvedCharacterProperties::resolve(&t.cp_max).expect("test properties");
    let sp = t.surfaces.records[0];
    let kin = Kinematics { fwd: 1.0, ..Kinematics::default() };
    let (kin30, f210) = (-0.83, -d);
    let i = GroundInputs {
        t: &t,
        r: &r,
        sp: &sp,
        st,
        balance: BalanceView { stable: true, ..BalanceView::default() },
        q: Quat::IDENTITY,
        n: Vec3::Y,
        contact,
        v_world: v,
        kin: &kin,
        kin30,
        kin34: f210 - kin30,
        f210,
        f218: t.carving.on_ground_distance_threshold - sp.snow_depth,
        stick_y: 0.0,
        brake: 0.0,
        slip_dig: 0.0,
        uber: false,
        board_press: false,
        stopped,
        height_above_lowest: 0.0,
        boost,
        grind: GrindGround::default(),
        wind: Vec3::ZERO,
        ice_axe: crate::ice_axe::IceAxeFrame::default(),
        avalanche_drag: 1.0,
        avalanche_friction: 1.0,
        rock_weight: 0.0,
        dt: 2.0,
    };
    integrate_ground(&i)
}

fn settled() -> CarvingState {
    CarvingState {
        have_made_first_contact: true,
        use_leg_spring: true,
        uls_ticks: 120.0,
        on_ground_ticks: 120.0,
        riding_height: -0.83,
        initialized: true,
        ..CarvingState::default()
    }
}

#[test]
fn without_a_ground_ray_only_gravity_and_drag() {
    let t = tuning();
    let r = ResolvedCharacterProperties::resolve(&t.cp_max).expect("test properties");
    let v = Vec3::new(0.3, -0.05, 0.0);
    let out = flat_step(&settled(), v, 0.83, false, false);
    let a = r.g_ground + drag_accel(r.k_h_ground, r.k_v_ground, v, Vec3::ZERO, 2.0);
    assert!(out.a.distance(a) < 1e-9 && out.v.distance(v + a * 2.0) < 1e-9 && out.dp.distance(out.v * 2.0) < 1e-9);
    assert!(!out.grounded && out.normal_force == 0.0 && out.is_uphill.is_none());
    assert_eq!(flat_step(&settled(), v, 0.83, false, true).v, Vec3::ZERO, "stopped");
}

#[test]
fn the_spring_holds_the_riding_level_and_friction_slows() {
    let v = Vec3::new(0.3, 0.0, 0.0);
    let out = flat_step(&settled(), v, 0.83, true, false);
    assert!(out.grounded && out.dp.y.abs() < 0.03, "{:?}", out.dp);
    let dv = v.x - out.v.x;
    assert!(dv > 2.0 * 0.03 * 0.015 * 0.99 && dv < 2.0 * 0.03 * 0.015 + 2e-3, "Δv {dv}");
    assert!(out.v.z == 0.0 && out.is_uphill == Some(false));
    let (mut d, mut vv) = (0.83f32, v);
    for _ in 0..60 {
        let o = flat_step(&settled(), vv, d, true, false);
        (d, vv) = (d + o.dp.y, o.v);
    }
    assert!((d - 0.83).abs() < 0.06, "P settles {d} m above the floor");
}

#[test]
fn the_floor_keeps_the_board_sphere_above_the_snow() {
    let st = CarvingState { use_leg_spring: false, uls_ticks: 0.0, on_ground_ticks: 0.0, ..settled() };
    let out = flat_step(&st, Vec3::new(0.3, -0.2, 0.0), 0.2, true, false);
    assert!(0.2 + out.dp.y >= 0.38 - 1e-5, "P at {} m", 0.2 + out.dp.y);
    assert!(out.v.y.abs() < 1e-6 && out.use_leg_spring && out.pitch_balance_vel > 0.0, "{out:?}");
    assert!(out.normal_force > 0.0);
}

#[test]
fn boost_thrust_lerps_fades_with_speed_and_height() {
    let c = tuning().carving;
    let b =
        BoostThrust { is_boosting: true, modifier: 1.0, accel_low: 0.0, accel_high: 30.0 / 3600.0, max_speed: 0.67 };
    let high = 30.0 / 3600.0;
    assert!(close(b.accel(&c, 0.0, 0.0, 1.0), high));
    assert!(close(b.accel(&c, 0.0, 0.0, -1.0), -high), "against a backwards board");
    assert!(close(b.accel(&c, 0.335, 0.0, 1.0), high * 0.75), "half the max speed: 1 − ¼");
    assert!(close(b.accel(&c, 0.67, 0.0, 1.0), 0.0) && close(b.accel(&c, 2.0, 0.0, 1.0), 0.0));
    assert!(close(b.accel(&c, 0.0, 20.0, 1.0), high) && close(b.accel(&c, 0.0, 30.0, 1.0), high * 0.5));
    assert!(close(b.accel(&c, 0.0, 45.0, 1.0), 0.0));
    let half = BoostThrust { modifier: 0.5, accel_low: 0.001, ..b };
    assert!(close(half.accel(&c, 0.0, 0.0, 1.0), 0.001 + (high - 0.001) * 0.5));
    assert!(close(BoostThrust { modifier: -1.0, ..half }.accel(&c, 0.0, 0.0, 1.0), 0.001));
    assert_eq!(BoostThrust { is_boosting: false, ..b }.accel(&c, 0.0, 0.0, 1.0), 0.0);
}

#[test]
fn boosting_accelerates_the_grounded_rider() {
    let c = tuning().carving;
    let v = Vec3::new(0.2, 0.0, 0.0);
    let b =
        BoostThrust { is_boosting: true, modifier: 1.0, accel_low: 0.0, accel_high: 30.0 / 3600.0, max_speed: 0.67 };
    let plain = flat_step(&settled(), v, 0.83, true, false);
    let boosted = flat_step_boost(&settled(), v, 0.83, true, false, b);
    let dv = boosted.v.x - plain.v.x;
    assert!((dv - b.accel(&c, 0.2, 0.0, 1.0) * 2.0).abs() < 1e-6, "Δv {dv}");
    let air = flat_step_boost(&settled(), v, 0.83, false, false, b);
    assert_eq!(air.v, flat_step(&settled(), v, 0.83, false, false).v);
}

#[test]
fn deceleration_keeps_the_braking_part() {
    let out = flat_step(&settled(), Vec3::new(0.2, 0.0, 0.05), 0.83, true, false);
    assert!(out.a.x < 0.0 && close(out.deceleration.x, out.a.x) && out.deceleration.y == 0.0, "{out:?}");
    assert_eq!(out.deceleration.z, 0.0, "no slip, no edge friction");
    let air = flat_step(&settled(), Vec3::new(0.2, 0.0, 0.05), 0.83, false, false);
    assert!(air.a.x < 0.0 && air.deceleration == Vec3::new(air.a.x, 0.0, 0.0), "{air:?}");
    let b =
        BoostThrust { is_boosting: true, modifier: 1.0, accel_low: 0.0, accel_high: 30.0 / 3600.0, max_speed: 0.67 };
    let pushed = flat_step_boost(&settled(), Vec3::new(0.2, 0.0, 0.0), 0.83, true, false, b);
    assert!(pushed.a.x > 0.0 && pushed.deceleration.x == 0.0, "{pushed:?}");
}

fn spray_inputs(decel: Vec3, v: Vec3) -> SprayInputs {
    SprayInputs {
        contact: true,
        board_x: Vec3::Y,
        board_z: Vec3::X,
        n: Vec3::Y,
        velocity: v,
        deceleration: decel,
        new_velocity: v,
        normal_force: 0.02,
        snow_coverage: 1.0,
        responsiveness: 0.2,
        dt: 2.0,
        ..SprayInputs::default()
    }
}

#[test]
fn spray_source_mirrors_the_edge_part_and_caps_by_the_speed() {
    let v = Vec3::new(0.3, 0.0, 0.0);
    let d = spray_source(&spray_inputs(Vec3::new(-0.01, 0.0, 0.0), v));
    assert!(d.distance(Vec3::new(-0.01, 0.0, 0.0)) < 1e-6, "{d}");
    let d = spray_source(&spray_inputs(Vec3::new(0.0, 0.0, -0.02), v));
    assert!(d.distance(Vec3::new(0.0, 0.0, 0.02)) < 1e-6, "{d}");
    let tilt = Quat::from_rotation_x(0.5);
    let i = SprayInputs { board_x: tilt * Vec3::Y, ..spray_inputs(Vec3::new(0.0, 0.0, -0.02), v) };
    let d = spray_source(&i);
    let s = Vec3::new(0.0, 0.0, 0.02);
    let h = i.board_x.cross(s.cross(Vec3::Y));
    let h = if h.dot(Vec3::Y) < 0.0 { -h } else { h };
    let want = (s.normalize() + h.normalize()).normalize() * 0.02;
    assert!(d.distance(want) < 1e-6 && d.y > 0.0, "{d} vs {want}");
    let d = spray_source(&spray_inputs(Vec3::new(-1.0, 0.0, 0.0), Vec3::new(0.01, 0.0, 0.0)));
    assert!((d.length() - 0.005).abs() < 1e-6);
    assert_eq!(spray_source(&SprayInputs { stopped: true, ..spray_inputs(Vec3::X, v) }), Vec3::ZERO);
    assert_eq!(spray_source(&spray_inputs(Vec3::new(-0.01, 0.0, 0.0), Vec3::ZERO)), Vec3::ZERO);
}

#[test]
fn spray_filter_snaps_then_follows_the_spring() {
    let v = Vec3::new(0.3, 0.0, 0.0);
    let mut st = CarvingState::default();
    let first = SprayInputs { first_contact: true, ..spray_inputs(Vec3::new(0.0, 0.0, -0.02), v) };
    let out = spray_filter(&mut st, &first);
    assert!(close(st.spray_magnitude, 0.02) && st.spray_derivative == 0.0);
    assert!(out.carving.distance(Vec3::new(0.0, 0.0, 0.02)) < 1e-6 && out.ice == Vec3::ZERO);
    assert!(out.velocity.distance(-v) < 1e-7 && out.impact.distance(Vec3::Y * 0.02) < 1e-7);
    let next = spray_inputs(Vec3::new(0.0, 0.0, -0.01), v);
    spray_filter(&mut st, &next);
    let (w, mut x, mut xv) = (0.2f32.sqrt(), 0.02f64, 0.0f64);
    for _ in 0..20_000 {
        let a = -(w as f64).powi(2) * (x - 0.01) - 2.0 * w as f64 * xv;
        xv += a * 1e-4;
        x += xv * 1e-4;
    }
    assert!((f64::from(st.spray_magnitude) - x).abs() < 1e-6, "{} vs {x}", st.spray_magnitude);
    assert!(st.spray_magnitude > 0.01 && st.spray_magnitude < 0.02 && st.spray_derivative < 0.0);
    let mut st2 = st;
    let ice = spray_filter(&mut st2, &SprayInputs { snow_coverage: 2.0, ice_coverage: 0.25, ..next });
    assert!((ice.ice.length() * 4.0 - ice.carving.length()).abs() < 1e-7);
    let kept = st;
    let none = spray_filter(&mut st, &SprayInputs { contact: false, ..next });
    assert_eq!((none.carving, none.normal_acceleration, st), (Vec3::ZERO, 0.02, kept));
}

#[test]
fn riding_height_adds_the_change_and_floors_at_the_maximum() {
    let mut c = tuning().carving;
    let rh = update_riding_height(&c, 0.3, -0.47, None, true, 1.0);
    assert_eq!(rh, 0.3f32 + (-0.47f32 - 0.3));
    assert_ne!(rh, -0.47);
    c.riding_height_max = 0.9;
    assert_eq!(update_riding_height(&c, -0.95, -0.95, None, false, 1.0), -0.9);
}

#[test]
fn spray_filter_matches_the_executable() {
    let cases: [([u32; 16], bool, [u32; 14]); 8] = [
        (
            [
                0xbd43_0da8,
                0x3d74_d3e4,
                0x3ed9_3221,
                0x3e8f_0c36,
                0x3a4d_8de5,
                0x3e34_ddfd,
                0x3f80_0000,
                0x3e3a_b763,
                0x3f57_bd16,
                0x3d8c_0859,
                0xbf20_add8,
                0xbdda_c123,
                0xbf37_66a1,
                0x3cdc_a61c,
                0x3f85_f4c8,
                0x3f22_3456,
            ],
            false,
            [
                0x3e94_84d7,
                0x3c92_94bc,
                0xbd03_43c7,
                0x3d24_c2e6,
                0x3e92_2a7b,
                0xbca6_5774,
                0x3cd0_ca09,
                0x3e39_3987,
                0x3f14_d934,
                0xbdda_ab99,
                0x3f32_f6ed,
                0x3ba0_eec0,
                0x3cb9_f27b,
                0x3af1_6412,
            ],
        ),
        (
            [
                0x0000_0000,
                0x0000_0000,
                0x0000_0000,
                0x3ec7_b3a9,
                0x3c49_bbd1,
                0x3e7f_abcc,
                0x3f80_0000,
                0xbe87_4e4f,
                0x3f41_ced9,
                0xbe1e_8cdb,
                0xbf70_9904,
                0xbd93_b975,
                0xbdf3_95a5,
                0x3d2c_8772,
                0x0000_0000,
                0x3eff_d660,
            ],
            false,
            [
                0x0000_0000,
                0x0000_0000,
                0x0000_0000,
                0x0000_0000,
                0x0000_0000,
                0x0000_0000,
                0x0000_0000,
                0x0000_0000,
                0x0000_0000,
                0x0000_0000,
                0x0000_0000,
                0x8000_0000,
                0x0000_0000,
                0x8000_0000,
            ],
        ),
        (
            [
                0x33d5_bde1,
                0x0000_0000,
                0x0000_0000,
                0x3f18_f030,
                0x3d0b_5a30,
                0x3e59_bd02,
                0x4000_0000,
                0x3e1e_8253,
                0x3f5a_96ec,
                0xbe90_75d0,
                0x3e01_50bb,
                0xbf48_bdce,
                0xbf48_ce08,
                0x3c6e_4ec2,
                0xbdd9_febb,
                0x3f7c_79d0,
            ],
            false,
            [
                0x33d5_bde1,
                0x0000_0000,
                0x0000_0000,
                0x0000_0000,
                0x0000_0000,
                0x33d2_cc9e,
                0x0000_0000,
                0x0000_0000,
                0x8000_0000,
                0x0000_0000,
                0x0000_0000,
                0x0000_0000,
                0x0000_0000,
                0x8000_0000,
            ],
        ),
        (
            [
                0xbeff_b893,
                0xbe94_9feb,
                0x3ed2_0f2a,
                0x3e90_614e,
                0x3d44_c149,
                0xbc23_d70a,
                0x4000_0000,
                0xbd45_1ee3,
                0x3f5e_aff7,
                0xbe39_3f67,
                0x3eb3_1b64,
                0xbea7_0590,
                0xbec1_5243,
                0x3a46_9153,
                0x3ebf_8646,
                0xbc3d_7ae4,
            ],
            false,
            [
                0x7fc0_0000,
                0xffc0_0000,
                0x7fc0_0000,
                0x7fc0_0000,
                0x7fc0_0000,
                0x7fc0_0000,
                0x7fc0_0000,
                0x7fc0_0000,
                0xbe01_b725,
                0x3d3e_385e,
                0x3e20_ba98,
                0xb764_c748,
                0x3981_39c1,
                0xb856_ffa7,
            ],
        ),
        (
            [
                0x3e53_fc03,
                0xbefa_641b,
                0xbd0f_9b3c,
                0x3e95_8d57,
                0x3c95_2868,
                0x3d6a_cffc,
                0x3f80_0000,
                0x3e95_1415,
                0x3f6e_4c9e,
                0xbd46_7fc7,
                0xbe6e_138b,
                0xbe57_571e,
                0x3f7a_e821,
                0x37c5_836a,
                0x0000_0000,
                0x3f99_396f,
            ],
            true,
            [
                0x3f08_3f0f,
                0x0000_0000,
                0x0000_0000,
                0x8000_0000,
                0x8000_0000,
                0x3e53_fc02,
                0xbefa_6419,
                0xbd0f_9b3b,
                0x0000_0000,
                0x8000_0000,
                0x8000_0000,
                0x0000_0000,
                0x0000_0000,
                0x8000_0000,
            ],
        ),
        (
            [
                0xbef5_f2f7,
                0xbea0_1940,
                0x3efd_de2e,
                0x3eb8_f0a9,
                0x3bfc_2e97,
                0x3c5e_a997,
                0x3f80_0000,
                0xbe30_2eeb,
                0x3f47_0925,
                0x3e27_8a61,
                0xbeaf_264f,
                0xbed0_903e,
                0xbf5a_6b81,
                0x3b93_a5e2,
                0x3f80_0000,
                0x7fc0_0000,
            ],
            false,
            [
                0x3ebd_b963,
                0x3c31_bd2a,
                0xbe70_81a0,
                0xbe1c_8e45,
                0x3e78_3ffb,
                0x7fc0_0000,
                0x7fc0_0000,
                0x7fc0_0000,
                0x3ed2_294c,
                0x3dc9_67ec,
                0x3f49_c5da,
                0xba4b_3a35,
                0x3b65_9672,
                0x3a41_420c,
            ],
        ),
        (
            [
                0xbef8_0f32,
                0xbe06_7a02,
                0x3dfa_2960,
                0x3d9c_544c,
                0x3c0e_fc39,
                0x3e7f_df57,
                0x3f80_0000,
                0x3e61_3181,
                0x3f41_3dbd,
                0xbe54_7fd0,
                0x3f51_1cea,
                0x3f22_b6f0,
                0xbf00_41b9,
                0x3c1b_7c16,
                0x3f80_0000,
                0x3f43_02f1,
            ],
            false,
            [
                0x3df8_79ab,
                0x3d8e_19aa,
                0xbde9_0584,
                0xbcfc_a614,
                0x3cea_ff12,
                0xbdb1_81e1,
                0xbcc0_7568,
                0x3cb3_02fe,
                0xbf26_22a9,
                0xbd73_2430,
                0x3eaf_6753,
                0x3b08_c620,
                0x3bea_bc18,
                0xbb01_1063,
            ],
        ),
        (
            [
                0xbde4_f230,
                0xbc8f_3a2a,
                0xbed7_cf23,
                0x3ce8_fcfb,
                0xbd20_225e,
                0x3e1d_e522,
                0x4000_0000,
                0xbe20_b8fe,
                0x3f69_4fcc,
                0xbe15_4f85,
                0x3f25_be58,
                0x3e45_9020,
                0xbed3_85af,
                0x3c0f_b70d,
                0x3f4e_f982,
                0xbcbc_bddf,
            ],
            false,
            [
                0x3d8c_9aea,
                0x3d5b_4e4b,
                0xbc68_f316,
                0xbb11_bb56,
                0xbd5b_953a,
                0x8000_0000,
                0x8000_0000,
                0x8000_0000,
                0xbf0a_5ee1,
                0xbd69_1e84,
                0x3ea2_e62f,
                0xba91_e5a4,
                0x3bd3_ca4e,
                0xba87_89a7,
            ],
        ),
    ];
    for (i, first_contact, want) in cases {
        let x = i.map(f32::from_bits);
        let mut st = CarvingState { spray_magnitude: x[3], spray_derivative: x[4], ..CarvingState::default() };
        let inp = SprayInputs {
            contact: true,
            first_contact,
            stopped: false,
            board_x: Vec3::X,
            board_z: Vec3::Z,
            n: Vec3::new(x[7], x[8], x[9]),
            velocity: Vec3::ZERO,
            deceleration: Vec3::ZERO,
            new_velocity: Vec3::new(x[10], x[11], x[12]),
            normal_force: x[13],
            snow_coverage: x[14],
            ice_coverage: x[15],
            responsiveness: x[5],
            dt: x[6],
        };
        let sp = spray_filter_from(&mut st, &inp, Vec3::new(x[0], x[1], x[2]));
        let got = [
            [st.spray_magnitude, st.spray_derivative].as_slice(),
            &sp.carving.to_array(),
            &sp.ice.to_array(),
            &sp.velocity.to_array(),
            &sp.impact.to_array(),
        ]
        .concat();
        for (g, w) in got.iter().zip(want.map(f32::from_bits)) {
            assert!(g.to_bits() == w.to_bits() || (g.is_nan() && w.is_nan()), "{i:x?}: {got:?} {want:x?}");
        }
    }
}
