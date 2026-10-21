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
    let a = friction_accel(&t, &snow, v, 0.0, 0.0, 1.0, 0.0, false);
    assert!(a.distance(Vec3::new(-0.03 * 0.015, 0.0, 0.0)) < 1e-9, "{a:?}");
    let a = friction_accel(&t, &snow, v, 0.0, 0.0, 0.25, 1.0, false);
    assert!(close(a.x, -0.7 * 0.5 * 0.015));
    let side = Vec3::new(0.3, 0.0, 0.1);
    let lean = 0.5f32;
    let against = friction_accel(&t, &snow, side, -lean, 0.0, 1.0, 0.0, false);
    let k = side.normalize();
    assert!(close(against.z, -k.z * lean.sin() * 5.6 * 0.015));
    let towards = friction_accel(&t, &snow, side, lean, 0.0, 1.0, 0.0, false);
    assert_eq!(towards.z, 0.0);
    assert_eq!(friction_accel(&t, &snow, Vec3::ZERO, 0.0, 0.0, 1.0, 1.0, false), Vec3::ZERO);
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
    assert_eq!(flat_step(&settled(), Vec3::new(0.2, 0.0, 0.05), 0.83, false, false).deceleration, Vec3::ZERO);
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
