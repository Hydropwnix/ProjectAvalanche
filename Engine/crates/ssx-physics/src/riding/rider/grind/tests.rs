use glam::{Quat, Vec3};

use super::state::{GrindCarving, MachineInputs, PathFrame, state_machine};
use super::surface_analysis::{DispatchInputs, SaFeature, SurfaceAnalysis};
use super::*;
use crate::collision::CollisionWorld;
use crate::testutil::{add_floor, add_quad, grind_tuning};

fn close(a: f32, b: f32, tol: f32) -> bool {
    (a - b).abs() <= tol
}

#[test]
fn grind_input_matches_the_executable() {
    let cases = [
        (0.0, 1.0, false, false, 0.0, 1.0),
        (0.3, 0.8, false, false, 0.0, 0.8),
        (1.2, 1.0, false, false, 0.849_270_4, 0.527_958_15),
        (-2.0, 0.6, false, false, -0.479_890_44, -0.360_146_08),
        (3.0, 1.0, false, false, 0.0, -1.0),
        (-0.9, 0.5, true, false, -0.267_539_7, 1.0),
        (2.2, 0.7, false, true, 0.410_697_37, -1.0),
    ];
    for (a, m, nose, tail, lat, press) in cases {
        let (l, p) = grind_input(std::f32::consts::FRAC_PI_6, a, m, nose, tail);
        assert!(close(l, lat, 1e-6) && close(p, press, 1e-6), "{a} {m}: {l} {p}");
    }
}

#[test]
fn press_matches_the_executable() {
    let t = grind_tuning();
    let cases = [
        ([0.0, 0.0, 0.0, 1.0, 1.2, 2.0, 0.8], [0.038_230_84, 0.033_946_674, 1.0]),
        ([0.3, 0.02, 1.0, 0.7, -1.5, 2.0, -0.6], [0.293_881_18, -0.021_307_338, 1.0]),
        ([0.01, 0.0, 1.0, 1.0, -1.8, 1.0, 0.02], [0.009_598_738, -0.000_756_868_6, -1.0]),
        ([0.0, 0.0, 0.0, 1.0, 0.2, 2.0, 1.0], [0.0, 0.0, 0.0]),
        ([0.5, -0.05, -1.0, 0.4, 2.9, 2.0, 0.9], [0.388_179_63, -0.059_604_347, -1.0]),
    ];
    for ([value, vel, sign, blend, board, dt, input], want) in cases {
        let mut p = Press { value, vel, sign };
        press_update(&mut p, &t, board, blend, input, dt);
        let got = [p.value, p.vel, p.sign];
        assert!(got.iter().zip(want).all(|(a, b)| close(*a, b, 1e-6)), "{got:?} {want:?}");
    }
}

#[test]
fn blend_matches_the_executable() {
    for (dt, b, grinding, want) in
        [(2.0, 0.3, true, 0.426_888_47), (2.0, 0.7, false, 0.573_111_5), (1.0, 0.0, true, 0.095_162_57)]
    {
        assert!(close(blend_update(b, grinding, 0.1, dt), want, 1e-6));
    }
}

#[test]
fn spin_controller_matches_the_executable() {
    let cases = [
        ([0.12, 0.1, 0.1, 1.0, 0.0, 2.0, 0.1], [1.036_253_8, 0.018_126_922]),
        ([0.12, 0.1, 0.1, 1.0, 0.05, 2.0, 0.0], [1.100_628_6, 0.050_314_303]),
        ([0.12, 0.1, 0.1, 0.4, 0.0, 2.0, 0.0], [0.386_856_6, -0.006_571_705_5]),
        ([0.12, 0.1, 0.1, 5.9, -0.08, 2.0, 0.0], [5.729_98, -0.085_010_03]),
        ([0.12, 0.1, 0.1, 3.0, 0.02, 2.0, -0.3], [2.989_244_7, -0.005_377_689]),
    ];
    for ([max_vel, vel_gain, angle_gain, angle, rate, dt, target], [a, r]) in cases {
        let mut s = SpinController { max_vel, vel_gain, angle_gain, angle, rate };
        s.update(dt, target);
        assert!(close(s.angle, a, 2e-6) && close(s.rate, r, 1e-6), "{s:?} {a} {r}");
    }
}

#[test]
fn veering_matches_the_executable() {
    let v = |x: [f32; 3]| Vec3::from(x);
    let cases = [
        ([0.0, 1.0, 0.0], [0.0, 0.0, 0.4], [0.02, 0.0, 0.4], 0.0, 0.0, 2.0, [-0.013_031_931, -0.012_424_816]),
        ([0.0, 1.0, 0.0], [0.0, 0.0, 0.4], [-0.02, 0.0, 0.4], 1.0, 0.1, 2.0, [1.177_551_2, 0.078_278_81]),
        ([0.196, 0.981, 0.0], [0.3, -0.06, 0.2], [0.29, -0.058, 0.22], -2.0, 0.3, 1.0, [-1.707_429, 0.285_244_9]),
    ];
    for (n, a, b, mut angle, mut vel, dt, [want_a, want_v]) in cases {
        veering_update(&mut angle, &mut vel, 0.005, v(n), v(a), v(b), dt);
        assert!(close(angle, want_a, 2e-4) && close(vel, want_v, 2e-4), "{angle} {vel}");
    }
}

#[test]
fn stick_direction_turns_to_the_right_of_travel() {
    let (v, n) = (Vec3::Z * 0.4, Vec3::Y);
    assert!(stick_direction(0.0, 1.0, v, n).distance(Vec3::Z) < 1e-6);
    assert!(stick_direction(std::f32::consts::FRAC_PI_2, 1.0, v, n).distance(-Vec3::X) < 1e-6);
    assert_eq!(stick_direction(0.3, 0.0, v, n), Vec3::ZERO);
}

fn rail_world() -> CollisionWorld {
    let mut w = CollisionWorld::default();
    add_floor(&mut w, [-20.0, 20.0], [-20.0, 60.0], 0.0, 0);
    let (z0, z1) = (-10.0, 50.0);
    let p = Vec3::new;
    add_quad(&mut w, [p(-0.15, 0.0, z0), p(-0.15, 0.0, z1), p(0.0, 0.4, z1), p(0.0, 0.4, z0)], 12);
    add_quad(&mut w, [p(0.15, 0.0, z0), p(0.0, 0.4, z0), p(0.0, 0.4, z1), p(0.15, 0.0, z1)], 12);
    w.finish();
    w
}

fn ground_dispatch(c: &CollisionWorld, f: &mut SaFeature, traj: Vec3, velocity: Vec3) -> SurfaceAnalysis {
    let i = DispatchInputs {
        traj,
        velocity,
        orientation: Quat::IDENTITY,
        in_air: false,
        grind_steering: None,
        ballistic_valid: false,
        ballistic_end: (Vec3::ZERO, Vec3::ZERO),
        ticks_to_land: 0.0,
        l2: true,
        super_grind: false,
        wind: Vec3::ZERO,
        k_h_air: 0.0,
        k_v_air: 0.0,
        g_air: 0.4,
        dt: 2.0,
    };
    f.dispatch(c, &i)
}

#[test]
fn the_rail_is_a_ridge_and_the_path_follows_it() {
    let c = rail_world();
    let v = Vec3::Z * 0.4;
    let joint = Vec3::new(0.5, 0.0, 0.0);
    let sa = ground_dispatch(&c, &mut SaFeature::default(), joint + Vec3::Y * 0.85, v);
    assert!(sa.ridgiprocity_at(Vec3::new(0.0, 0.4, 6.0)) > 0.25, "{}", sa.ridgiprocity_at(Vec3::new(0.0, 0.4, 6.0)));
    assert!(sa.ridgiprocity_at(Vec3::new(3.0, 0.0, 6.0)) < 0.05);
    let p = path::carving_path(&sa, joint, v, 2.0);
    assert!(p.valid);
    assert!(p.peak_b >= 0.25 && p.pos_b.x.abs() < 0.2, "{p:?}");
}

#[test]
fn riding_onto_the_rail_with_the_button_grinds_along_it() {
    let c = rail_world();
    let t = grind_tuning();
    let mut f = SaFeature::default();
    let mut g = GrindCarving::default();
    let (n, dt) = (Vec3::Y, 2.0);
    let mut joint = Vec3::new(0.0, 0.4, 0.0);
    let mut v = Vec3::Z * 0.4;
    for step in 0..12 {
        let traj = joint + n * 0.85;
        let sa = ground_dispatch(&c, &mut f, traj, v);
        let path: PathFrame = path::carving_path(&sa, joint, super::state::on_plane(v, n), dt);
        let i = MachineInputs {
            c: &c,
            t: &t,
            sa: &sa,
            dt,
            traj,
            joint,
            n,
            f210: 0.0,
            path,
            button: true,
            stick_dir: Vec3::ZERO,
            tail_dir: Some(Vec3::Z),
            depth: RAIL_MAX_DEPTH,
            follow_buried: FOLLOW_BURIED_RAILS,
            wiping_out: false,
        };
        v = super::state::on_plane(v, n);
        let mut dp = v * dt;
        state_machine(&mut g, &i, &mut dp, &mut v);
        assert_eq!(g.state, 2, "step {step}");
        assert!(g.super_grind, "a class-12 rail under the path");
        joint += dp;
        assert!(joint.x.abs() < 0.1 && (joint.y - 0.4).abs() < 0.2, "step {step}: {joint}");
    }
    assert!(joint.z > 9.0);
}

#[test]
fn the_grind_trick_flow_enters_loops_and_exits() {
    use super::tricks::{FlowEvents, GrindNode, GrindTricks};
    let mut g = GrindTricks::default();
    g.enter();
    assert!(g.is_grind_trick() && g.is_grind_trick_enter());
    g.flow(true, false, false, 10.0);
    g.flow(true, true, false, 2.0);
    assert_eq!((g.node, g.node_ticks), (GrindNode::DoubleTapEnter, 12.0));
    g.flow(true, true, false, 30.0);
    assert_eq!((g.node, g.node_ticks), (GrindNode::Loop, 2.0));
    let ev = g.flow(false, true, false, 2.0);
    assert_eq!(ev, FlowEvents { grab_reset: true, ground_trick_reset: false });
    assert_eq!((g.node, g.node_ticks), (GrindNode::Exit, 2.0));
    assert!(g.ending());
    let ev = g.flow(false, true, false, 30.0);
    assert_eq!(ev, FlowEvents { grab_reset: false, ground_trick_reset: true });
    assert!(!g.is_grind_trick());
}
