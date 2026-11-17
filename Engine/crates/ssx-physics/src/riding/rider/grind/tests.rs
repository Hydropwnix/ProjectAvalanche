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
    let cases: [([u32; 3], bool, bool, [u32; 2]); 16] = [
        ([0x0000_0000, 0x3f80_0000, 0x3f06_0a92], false, false, [0x0000_0000, 0x3f80_0000]),
        ([0x3e99_999a, 0x3f4c_cccd, 0x3f06_0a92], false, false, [0x0000_0000, 0x3f4c_cccd]),
        ([0x3f99_999a, 0x3f80_0000, 0x3f06_0a92], false, false, [0x3f59_69ca, 0x3f07_2842]),
        ([0xc000_0000, 0x3f19_999a, 0x3f06_0a92], false, false, [0xbef5_b433, 0xbeb8_6511]),
        ([0x4040_0000, 0x3f80_0000, 0x3f06_0a92], false, false, [0xb3bb_bd2e, 0xbf80_0000]),
        ([0xbf66_6666, 0x3f00_0000, 0x3f06_0a92], true, false, [0xbe88_faf7, 0x3f80_0000]),
        ([0x400c_cccd, 0x3f33_3333, 0x3f06_0a92], false, true, [0x3ed2_46ec, 0xbf80_0000]),
        ([0x7fc0_0000, 0x3f00_0000, 0x3f06_0a92], false, false, [0x333b_bd2e, 0xbf00_0000]),
        ([0x3f06_0a92, 0x3f80_0000, 0x3f06_0a92], false, false, [0x0000_0000, 0x3f80_0000]),
        ([0x8000_0000, 0x3f80_0000, 0x3f06_0a92], false, false, [0x0000_0000, 0x3f80_0000]),
        ([0x4049_0fdb, 0x3f66_6666, 0x3f06_0a92], false, false, [0xb3a8_f710, 0xbf66_6666]),
        ([0xbf90_50f2, 0x3e1a_7835, 0x3ef3_4297], false, true, [0xbdf8_9d56, 0xbf80_0000]),
        ([0xbf5c_0e27, 0x3d6d_9048, 0x3ece_9e5d], false, true, [0xbd08_d024, 0xbf80_0000]),
        ([0xc030_2fee, 0x3db9_c7bf, 0x3e3f_2429], false, false, [0xbca8_dee7, 0xbdb4_eb78]),
        ([0xbfe2_b8ff, 0x3f20_9f77, 0x3ec1_5ecb], false, false, [0xbf1b_0f8d, 0xbe27_9cf4]),
        ([0x4043_12f8, 0x3d3e_cd7b, 0x3e4d_cd53], false, false, [0xb18b_ed19, 0xbd3e_cd7b]),
    ];
    for (i, nose, tail, want) in cases {
        let [angle, mag, dz] = i.map(f32::from_bits);
        let (l, p) = grind_input(dz, angle, mag, nose, tail);
        assert_eq!([l.to_bits(), p.to_bits()], want, "{i:x?} {nose} {tail}");
    }
}

#[test]
fn press_matches_the_executable() {
    let cases: [[u32; 12]; 16] = [
        [
            0x0000_0000,
            0x0000_0000,
            0x0000_0000,
            0x3f80_0000,
            0x3f99_999a,
            0x3cf5_c28f,
            0x3f06_0a92,
            0x4000_0000,
            0x3f4c_cccd,
            0x3d1c_97f1,
            0x3d0b_0bab,
            0x3f80_0000,
        ],
        [
            0x3e99_999a,
            0x3ca3_d70a,
            0x3f80_0000,
            0x3f33_3333,
            0xbfc0_0000,
            0x3cf5_c28f,
            0x3f06_0a92,
            0x4000_0000,
            0xbf19_999a,
            0x3e96_7797,
            0xbcae_8cba,
            0x3f80_0000,
        ],
        [
            0x3c23_d70a,
            0x0000_0000,
            0x3f80_0000,
            0x3f80_0000,
            0xbfe6_6666,
            0x3cf5_c28f,
            0x3f06_0a92,
            0x3f80_0000,
            0x3ca3_d70a,
            0x3c1d_4406,
            0xba46_6897,
            0xbf80_0000,
        ],
        [
            0x0000_0000,
            0x0000_0000,
            0x0000_0000,
            0x3f80_0000,
            0x3e4c_cccd,
            0x3cf5_c28f,
            0x3f06_0a92,
            0x4000_0000,
            0x3f80_0000,
            0x0000_0000,
            0x8000_0000,
            0x0000_0000,
        ],
        [
            0x3f00_0000,
            0xbd4c_cccd,
            0xbf80_0000,
            0x3ecc_cccd,
            0x4039_999a,
            0x3cf5_c28f,
            0x3f06_0a92,
            0x4000_0000,
            0x3f66_6666,
            0x3ec6_bf7c,
            0xbd74_23b0,
            0xbf80_0000,
        ],
        [
            0x3e4c_cccd,
            0x3c23_d70a,
            0x3f80_0000,
            0x3f80_0000,
            0x7fc0_0000,
            0x3cf5_c28f,
            0x3f06_0a92,
            0x4000_0000,
            0x3f00_0000,
            0x3e69_f6ed,
            0x3c8e_267d,
            0x3f80_0000,
        ],
        [
            0x3ca3_d70a,
            0x3c23_d70a,
            0x7fc0_0000,
            0x3f80_0000,
            0x3f80_0000,
            0x3cf5_c28f,
            0x3f06_0a92,
            0x4000_0000,
            0x3f00_0000,
            0x3d69_cfd8,
            0x3ccc_b88a,
            0x3f80_0000,
        ],
        [
            0x3e99_999a,
            0x3c23_d70a,
            0x3f80_0000,
            0x3f80_0000,
            0x3f80_0000,
            0xbcf5_c28f,
            0x3f06_0a92,
            0x4000_0000,
            0x3f00_0000,
            0x7fc0_0000,
            0xffc0_0000,
            0x3f80_0000,
        ],
        [
            0x3e99_999a,
            0x3c23_d70a,
            0x3f80_0000,
            0x3f80_0000,
            0x3f80_0000,
            0x0000_0000,
            0x3f06_0a92,
            0x4000_0000,
            0x3f00_0000,
            0x3ea3_d70a,
            0x3c23_d70a,
            0x3f80_0000,
        ],
        [
            0x3e7b_b3e2,
            0x3dc6_1268,
            0x0000_0000,
            0x3f57_210e,
            0x3fe2_10b5,
            0x3d4f_198e,
            0x3e0b_9183,
            0x3f80_0000,
            0xbf46_09de,
            0x3e9b_66df,
            0x3cc2_1d24,
            0x3f80_0000,
        ],
        [
            0x3cf3_6c3e,
            0xbdcf_9b87,
            0x0000_0000,
            0x3f12_edcf,
            0xc047_6dad,
            0x3d34_c0da,
            0x3e97_5dd2,
            0x4000_0000,
            0xbd13_f5fb,
            0xbdd8_0058,
            0xbd25_3aef,
            0x0000_0000,
        ],
        [
            0x3e70_8af2,
            0xbe18_e76c,
            0x0000_0000,
            0x3f78_eb19,
            0xc04a_9fad,
            0x3e1e_a9f1,
            0x3f45_a79f,
            0x3f80_0000,
            0x3f3e_ac3b,
            0x3df5_fe0b,
            0xbdaf_612a,
            0x0000_0000,
        ],
        [
            0xbed7_c063,
            0x3e3d_0579,
            0x3f80_0000,
            0x3e3a_075a,
            0x403f_ebbd,
            0x3d24_92a9,
            0x3f46_8b97,
            0x4000_0000,
            0xbece_0a48,
            0xbe1c_4843,
            0x3dbe_1216,
            0x3f80_0000,
        ],
        [
            0xbe8e_243a,
            0xbe08_d30c,
            0x3f80_0000,
            0x3e87_125c,
            0xbf89_beab,
            0x3e27_972f,
            0x3f02_a2f3,
            0x4000_0000,
            0xbf5e_324e,
            0xbec6_3842,
            0xbb8b_d803,
            0x3f80_0000,
        ],
        [
            0xbe94_1ec6,
            0xbd9e_df46,
            0xbf80_0000,
            0x3e3d_3532,
            0xbe33_f901,
            0x3d13_a6f5,
            0x3e8f_03fc,
            0x3f80_0000,
            0x3f66_003a,
            0xbeb3_f767,
            0xbd45_d89d,
            0xbf80_0000,
        ],
        [
            0x3d3a_1eb7,
            0xbd1c_cdf9,
            0x322b_cc77,
            0x3ebb_7c88,
            0x3f00_a52f,
            0x3b37_eeeb,
            0x3e07_e4dd,
            0x3f80_0000,
            0xbd1c_0c97,
            0x3c14_4e97,
            0xbd0d_7b97,
            0x3f80_0000,
        ],
    ];
    for c in cases {
        let v = c.map(f32::from_bits);
        let mut t = grind_tuning();
        (t.press_stiffness, t.input_dead_zone_angle) = (v[5], v[6]);
        let mut p = Press { value: v[0], vel: v[1], sign: v[2] };
        press_update(&mut p, &t, v[4], v[3], v[8], v[7]);
        for (got, want) in [p.value, p.vel, p.sign].into_iter().zip(&v[9..]) {
            assert!(got.to_bits() == want.to_bits() || (got.is_nan() && want.is_nan()), "{c:x?}: {got} {want}");
        }
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
    let (mut angle, mut vel) = (0.5, 0.1);
    veering_update(&mut angle, &mut vel, 0.005, Vec3::Y, Vec3::ZERO, v([0.0, 0.0, 0.4]), 1.0);
    assert!(angle.is_nan() && vel.is_nan());
    veering_update(&mut angle, &mut vel, 0.005, Vec3::Y, v([0.0, 0.0, 0.4]), v([0.0, 0.0, 0.4]), 1.0);
    assert!(angle.is_nan() && vel.is_nan());
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
fn releasing_the_button_turns_the_exit_at_most_the_joystick_exit_angle() {
    let c = rail_world();
    let t = grind_tuning();
    for side in [1.0, -1.0, 0.0] {
        let mut f = SaFeature::default();
        let mut g = GrindCarving::default();
        let (n, dt) = (Vec3::Y, 2.0);
        let mut joint = Vec3::new(0.0, 0.4, 0.0);
        let mut v = Vec3::Z * 0.4;
        for step in 0..10 {
            let button = step < 9;
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
                button,
                stick_dir: if button { Vec3::ZERO } else { Vec3::X * side },
                tail_dir: Some(Vec3::Z),
                depth: RAIL_MAX_DEPTH,
                follow_buried: FOLLOW_BURIED_RAILS,
                wiping_out: false,
            };
            v = super::state::on_plane(v, n);
            let (along, mut dp) = (v, v * dt);
            let dbg = state_machine(&mut g, &i, &mut dp, &mut v);
            g.prev_input = button;
            joint += dp;
            if button {
                assert_eq!(g.state, 2, "side {side}, step {step}");
                continue;
            }
            assert_eq!(dbg.exit, Some(state::GrindExit::Button), "side {side}");
            assert_eq!(g.state, 0, "side {side}: the exit lasts one update");
            assert!(g.auto_grind_clearing, "side {side}");
            let turn = along.angle_between(v);
            if side == 0.0 {
                assert!(turn < 1e-3, "centred: {turn}");
            } else {
                let max = t.super_.max_joystick_exit_angle;
                assert!(close(turn, max, 1e-4), "side {side}: {}°", turn.to_degrees());
                assert!(v.x * side > 0.0, "side {side}: towards the stick, {v}");
            }
        }
    }
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

#[test]
fn stick_direction_matches_the_executable() {
    let cases: [([u32; 8], [u32; 3]); 12] = [
        (
            [0x3d34_179a, 0x3f1d_0693, 0x3f6b_c5f9, 0xbe38_1062, 0x3f77_d641, 0x3e32_add2, 0x3f88_561f, 0x3de1_9ca4],
            [0xbf3b_d488, 0xbe80_0c91, 0x3f21_baf6],
        ),
        (
            [0xbf72_3558, 0xbe6d_37b3, 0x3efc_4f06, 0xbe4a_d8e3, 0x3f4c_f6d0, 0xbe15_8ea3, 0x4012_49f3, 0x3f71_f83a],
            [0x3e90_e85c, 0xbdd4_cc72, 0xbf74_167e],
        ),
        (
            [0xbe49_8ab1, 0x3f7f_3472, 0xbf60_2844, 0x3e7a_b8c9, 0x3f6e_2268, 0xbe8b_fcf6, 0x3fad_aa14, 0x0000_0000],
            [0x0000_0000, 0x0000_0000, 0x0000_0000],
        ),
        (
            [0x3e2a_08c4, 0x3eed_b773, 0xbda9_a76f, 0x3eaa_08c4, 0x3f6d_b773, 0xbe29_a76f, 0xc030_9ee7, 0x3f70_952a],
            [0x0000_0000, 0x0000_0000, 0x0000_0000],
        ),
        (
            [0x0000_0000, 0x0000_0000, 0x0000_0000, 0x3e7b_d146, 0x3f69_3d88, 0x3ea9_5f20, 0xc04b_aab2, 0x3ebe_900f],
            [0x0000_0000, 0x0000_0000, 0x0000_0000],
        ),
        (
            [0xbf65_8ba4, 0x3f01_0ab6, 0xbe94_d8cc, 0x3ed9_f447, 0x3f5e_51a8, 0x3e82_247d, 0x7fc0_0000, 0x3e74_ae69],
            [0x7fc0_0000, 0x7fc0_0000, 0x7fc0_0000],
        ),
        (
            [0x3de8_b88c, 0x3f6a_2f4f, 0x3d5c_82a8, 0x3de3_84a7, 0x3ddb_0eb6, 0xbf3d_b405, 0xc001_c891, 0x3f1a_98f5],
            [0x3f55_9036, 0xbf0c_ace2, 0x3d3b_61f7],
        ),
        (
            [0x3c25_fb90, 0x0000_0000, 0xbbc7_3965, 0x0000_0000, 0x3f80_0000, 0x0000_0000, 0xbfc0_2058, 0x3c1f_f678],
            [0xbee8_33a8, 0x0000_0000, 0xbf64_28bd],
        ),
        (
            [0xbd86_8463, 0xbee8_f4fa, 0x3e88_4be4, 0x3e64_a043, 0x3f61_de8f, 0xbed4_2707, 0x404d_9ecd, 0x3f68_f874],
            [0xbf2d_bfae, 0xbe28_5999, 0xbf37_3cd1],
        ),
        (
            [0xbf75_6bdf, 0x3e03_7354, 0x3ebe_7659, 0xbda4_0e7d, 0x3f6b_ee19, 0x3ec2_7293, 0x3f18_4fc7, 0x3ec8_7c5e],
            [0xbf74_93f0, 0x3d1a_5937, 0xbe95_feba],
        ),
        (
            [0xbf1b_d401, 0xbefe_8c29, 0xbf32_ca26, 0xbe9f_93b4, 0x3f2f_5945, 0x3e8c_17d9, 0x3e79_e970, 0x3f4e_1045],
            [0xbf2c_d3ba, 0xbc4d_65f9, 0xbf3c_d450],
        ),
        (
            [0x3f57_2297, 0x3f1f_1f2c, 0xbf66_af6a, 0x3ee7_4aab, 0x3f58_37bc, 0x3e93_21be, 0x400f_2b75, 0x0000_0000],
            [0x0000_0000, 0x0000_0000, 0x0000_0000],
        ),
    ];
    for (i, want) in cases {
        let x = i.map(f32::from_bits);
        let got = stick_direction(x[6], x[7], Vec3::new(x[0], x[1], x[2]), Vec3::new(x[3], x[4], x[5]));
        for (g, w) in got.to_array().into_iter().zip(want.map(f32::from_bits)) {
            assert!(g.to_bits() == w.to_bits() || (g.is_nan() && w.is_nan()), "{i:x?}: {got:?}");
        }
    }
}
