use ssx_assets::anim_eval::curve_weight;

use super::cam3p::{AVALANCHE_STRENGTH, REVERSE_Y_OFFSET, avalanche_terms, determine_carving_weight};
use super::flow::{Blend, Node, Signals, choose};
use super::*;

const TARGET: Vec3 = Vec3::new(100.0, 50.0, -20.0);

fn inputs() -> Inputs {
    Inputs {
        target_pos: TARGET,
        target_up: Vec3::Y,
        velocity: Vec3::new(0.4, -0.1, 0.0),
        speed: 25.0,
        ground_normal: Vec3::Y,
        lean: 0.0,
        airborne: false,
        wipeout: false,
        boosting: false,
        grinding: false,
        dead: false,
        track_finished: false,
        aggregate_mass: 0.0,
        bailed_by_avalanche: false,
        alive: true,
        heli_position: Vec3::ZERO,
        heli_interact: false,
        heli_time: 0.0,
        back_press: false,
    }
}

#[test]
fn avalanche_terms_follow_the_aggregate_mass() {
    let (mut g, mut s) = (GraphState::default(), shared());
    let (mut pitch, mut added) = (0.0, 0.0);
    for _ in 0..600 {
        (pitch, added) = avalanche_terms(&mut g, &mut s, 8.0, AVALANCHE_STRENGTH, 2.0);
    }
    assert!((s.avalanche_intensity - 1.0).abs() < 1e-4);
    assert!((added - 9.0).abs() < 1e-2 && (pitch + 0.5).abs() < 1e-3, "{added} {pitch}");
    avalanche_terms(&mut g, &mut s, 0.0, AVALANCHE_STRENGTH, 2.0);
    assert!(s.avalanche_intensity >= 0.99 - 1e-4, "the decay keeps 99 %");
}

#[test]
fn reverse_camera_is_ahead_and_looks_back() {
    let mut s = shared();
    s.hinge = TARGET + Vec3::new(-60.0, 10.0, 0.0);
    let c = step_reverse(&mut s, &inputs());
    assert!((c.position - c.interest).length() - REVERSE_DISTANCE < 1e-3);
    assert!(c.position.x > TARGET.x + 30.0, "ahead: {:?}", c.position);
    let fwd = c.rotation * Vec3::X;
    assert!(fwd.dot((c.interest - c.position).normalize()) > 0.999);
    assert_eq!(s.y_offset, REVERSE_Y_OFFSET);
}

#[test]
fn reverse_flow_takes_ground_then_death() {
    let alive = Signals { in_air: false, grinding: false, wipeout: false, dead: false, heli_active: false };
    let dead = Signals { dead: true, ..alive };
    assert_eq!(choose(Node::ReverseConduit, &alive), Some((Node::ReverseGround, Blend::Cut)));
    assert_eq!(choose(Node::ReverseGround, &alive), None);
    assert_eq!(choose(Node::ReverseGround, &dead), Some((Node::ReverseDeath, Blend::Default)));
    assert_eq!(choose(Node::ReverseDeath, &alive), None);
}

fn shared() -> Shared {
    Shared::new(TARGET + Vec3::new(-12.0, 4.0, 0.0))
}

#[test]
fn carving_weight_follows_the_kernel() {
    assert_eq!(determine_carving_weight(0.5, 0.0, 0.8, false, false, true), 0.4);
    assert_eq!(determine_carving_weight(0.5, 0.0, 0.8, true, false, true), 1.0);
    assert_eq!(determine_carving_weight(-0.3, 0.0, 0.8, false, false, true), 0.0);
    assert_eq!(determine_carving_weight(3.0, 0.0, 0.8, false, false, true), 1.0);
    assert_eq!(determine_carving_weight(0.5, 0.1, 0.8, true, false, true), 0.0);
    assert_eq!(determine_carving_weight(0.5, 0.0, 0.8, true, true, true), 0.0);
    assert_eq!(determine_carving_weight(0.5, 0.0, 0.8, true, false, false), 0.0);
}

fn signals(in_air: bool, grinding: bool, wipeout: bool, dead: bool) -> Signals {
    Signals { in_air, grinding, wipeout, dead, heli_active: false }
}

#[test]
fn cam15p_flow_follows_the_lists() {
    use Blend::{Cut, Default};
    let none = signals(false, false, false, false);
    let heli = Signals { heli_active: true, ..signals(true, false, false, false) };
    assert_eq!(choose(Node::P15Conduit, &none), Some((Node::P15Ground, Cut)));
    assert_eq!(choose(Node::P15Conduit, &signals(true, false, false, false)), Some((Node::P15Airborne, Cut)));
    assert_eq!(choose(Node::P15Conduit, &signals(false, true, false, false)), Some((Node::P15Rails, Cut)));
    assert_eq!(choose(Node::P15Conduit, &signals(false, false, true, false)), Some((Node::P15Wipeout, Cut)));
    assert_eq!(choose(Node::P15Conduit, &heli), Some((Node::P15Heli, Default)));
    assert_eq!(choose(Node::P15Conduit, &signals(true, true, false, true)), Some((Node::P15Death, Cut)));
    assert_eq!(choose(Node::P15Airborne, &heli), Some((Node::P15Conduit, Default)));
    assert_eq!(choose(Node::P15Airborne, &none), Some((Node::P15Conduit, Cut)));
    assert_eq!(choose(Node::P15Heli, &heli), None);
    assert_eq!(choose(Node::P15Heli, &signals(true, false, false, false)), Some((Node::P15Conduit, Default)));
    assert_eq!(choose(Node::P15Ground, &none), None);
    assert_eq!(choose(Node::P15Ground, &signals(true, false, false, false)), Some((Node::P15Conduit, Cut)));
    assert_eq!(choose(Node::P15Rails, &none), Some((Node::P15Conduit, Cut)));
    assert_eq!(choose(Node::P15Wipeout, &none), Some((Node::P15Conduit, Cut)));
    assert_eq!(choose(Node::P15Death, &none), Some((Node::P15Conduit, Cut)));
    let mut i = inputs();
    (i.heli_interact, i.heli_time) = (true, 15.0);
    assert!(!Signals::of(&shared(), &i).heli_active, "the window is open: 15 ticks before the meeting");
    i.heli_time = -59.0;
    assert!(Signals::of(&shared(), &i).heli_active);
}

#[test]
fn cam15p_graph_frames_the_lookahead() {
    let mut i = inputs();
    i.velocity = Vec3::ZERO;
    let mut s = shared();
    let c = step_15p(Variant15::Air, &mut GraphState::default(), &mut s, &i, Vec3::ZERO, 2.0);
    let a = TARGET + Vec3::Y * 0.6;
    assert_eq!(s.y_offset, 0.6);
    assert!((s.hinge.distance(TARGET) - 8.0).abs() < 1e-3, "the air hinge 8 m behind the pivot");
    assert!((c.interest.distance(a) - 50.0).abs() < 1e-2, "the lookahead 50 m ahead");
    let d = c.position.distance(a);
    assert!(d > 5.0 && d < 7.0, "≈ 5.75 m from the interest at the resting FOV: {d}");
    assert!(c.position.y > a.y, "the pitch offset lifts the camera");
    let (mut s1, mut s2) = (shared(), shared());
    i.heli_position = TARGET + Vec3::new(5.0, 3.0, 0.0);
    let l = step_15p(Variant15::Heli, &mut GraphState::default(), &mut s1, &i, TARGET + Vec3::Z * 10.0, 2.0);
    let r = step_15p(Variant15::Heli, &mut GraphState::default(), &mut s2, &i, TARGET - Vec3::Z * 10.0, 2.0);
    let ang = |c: &Channels| (c.position - a).with_y(0.0).normalize();
    let apart = ang(&l).angle_between(ang(&r));
    assert!(apart > 1.98 && apart < 2.4, "the two sides about 2 × 0.99 rad apart: {apart}");
}

#[test]
fn far_flow_and_its_handycam_tag() {
    use Blend::Cut;
    let none = signals(false, false, false, false);
    assert_eq!(choose(Node::FarConduit, &none), Some((Node::FarGround, Cut)));
    assert_eq!(choose(Node::FarConduit, &signals(true, false, false, false)), Some((Node::FarAirborne, Cut)));
    assert_eq!(choose(Node::FarConduit, &signals(true, false, true, false)), Some((Node::FarGround, Cut)));
    assert_eq!(choose(Node::FarConduit, &signals(false, false, false, true)), Some((Node::FarDeath, Cut)));
    assert_eq!(choose(Node::FarAirborne, &none), Some((Node::FarConduit, Cut)));
    assert_eq!(choose(Node::FarGround, &signals(true, false, false, false)), Some((Node::FarAirborne, Cut)));
    let (mut s, i, rig) = (shared(), inputs(), rig());
    s.in_air = false;
    let mut flow = Flow::far();
    flow.update(&mut s, &i, &rig, 2.0);
    assert_eq!(flow.current.node, Node::FarGround);
    flow.monitor(&mut s);
    assert!(!s.handycam && s.transition_speed_set == 0.3);
    let mut air = i;
    air.airborne = true;
    flow.update(&mut s, &air, &rig, 2.0);
    flow.monitor(&mut s);
    flow.update(&mut s, &air, &rig, 2.0);
    flow.monitor(&mut s);
    assert_eq!(flow.current.node, Node::FarAirborne);
    assert!(s.handycam, "ResetOnLeave");
}

#[test]
fn far_graph_is_the_normal_graph_farther_out() {
    let i = inputs();
    let (mut sn, mut sf) = (shared(), shared());
    let n = step(Variant::Ground, &mut GraphState::default(), &mut sn, &i, 2.0);
    let f = step(Variant::FarGround, &mut GraphState::default(), &mut sf, &i, 2.0);
    assert_eq!((sn.y_offset, sf.y_offset), (0.8, 1.4));
    assert!(f.position.distance(f.interest) > n.position.distance(n.interest) + 2.5);
}

#[test]
fn hinge_reset_takes_the_reset_point() {
    let i = inputs();
    let mut s = shared();
    s.hinge_reset = TARGET + Vec3::new(0.0, 2000.0, -100.0);
    s.force_hinge_reset = true;
    step(Variant::Air, &mut GraphState::default(), &mut s, &i, 2.0);
    assert!(!s.force_hinge_reset);
    let d = (TARGET - s.hinge).normalize();
    assert!((d.y + HINGE_ANGLE.cos()).abs() < 1e-3 && d.z > 0.0, "from above and behind: {d}");
}

#[test]
fn transitions_follow_the_lists() {
    use Blend::{Cut, Default};
    let none = signals(false, false, false, false);
    assert_eq!(choose(Node::Conduit, &none), Some((Node::Ground, Cut)));
    assert_eq!(choose(Node::Conduit, &signals(true, false, false, false)), Some((Node::Airborne, Cut)));
    assert_eq!(choose(Node::Conduit, &signals(false, true, false, false)), Some((Node::Grind, Default)));
    assert_eq!(choose(Node::Conduit, &signals(true, true, false, false)), Some((Node::Ground, Cut)));
    assert_eq!(choose(Node::Conduit, &signals(false, true, false, true)), Some((Node::Grind, Default)));
    assert_eq!(choose(Node::Conduit, &signals(true, false, false, true)), Some((Node::Airborne, Cut)));
    assert_eq!(choose(Node::Ground, &none), None);
    assert_eq!(choose(Node::Ground, &signals(false, true, false, false)), Some((Node::Conduit, Default)));
    assert_eq!(choose(Node::Ground, &signals(true, false, false, false)), Some((Node::Conduit, Default)));
    assert_eq!(choose(Node::Ground, &signals(true, true, false, false)), None);
    assert_eq!(choose(Node::Ground, &signals(false, false, true, true)), Some((Node::Death, Cut)));
    assert_eq!(choose(Node::Airborne, &signals(true, false, false, true)), Some((Node::Death, Cut)));
    assert_eq!(choose(Node::Airborne, &signals(true, false, false, false)), None);
    assert_eq!(choose(Node::Airborne, &none), Some((Node::Conduit, Cut)));
    assert_eq!(choose(Node::Grind, &signals(false, true, false, false)), None);
    assert_eq!(choose(Node::Grind, &signals(false, true, false, true)), Some((Node::Death, Default)));
    assert_eq!(choose(Node::Grind, &none), Some((Node::Conduit, Default)));
    assert_eq!(choose(Node::Death, &signals(false, false, false, true)), None);
    assert_eq!(choose(Node::Death, &none), Some((Node::Conduit, Cut)));
}

fn rig() -> Channels {
    Channels {
        position: Vec3::new(90.0, 54.0, -20.0),
        rotation: Quat::from_rotation_z(-0.2),
        interest: TARGET,
        fov_deg: 97.0,
    }
}

#[test]
fn death_enters_and_leaves_through_the_conduit() {
    let (mut s, mut i, rig) = (shared(), inputs(), rig());
    let mut flow = Flow::default();
    flow.update(&mut s, &i, &rig, 0.0);
    assert_eq!(flow.current.node, Node::Airborne, "BLN_InAir starts at its default, true");
    assert!(!s.in_air, "the air graph's [75] wrote it");
    flow.update(&mut s, &i, &rig, 2.0);
    assert_eq!((flow.current.node, flow.current.depth()), (Node::Ground, 0));
    i.dead = true;
    let (fov, y) = (s.fov, s.y_offset);
    for target in [TARGET, TARGET - Vec3::Y * 30.0] {
        i.target_pos = target;
        let out = flow.update(&mut s, &i, &rig, 2.0);
        assert_eq!((flow.current.node, flow.current.depth()), (Node::Death, 0));
        assert_eq!(out, Channels { fov_deg: s.fov_base, ..rig }, "the camera stops following");
        assert!(s.up.distance(target.normalize()) < 1e-6, "[2], [4]: the up is the target's direction");
        assert_eq!((s.fov, s.y_offset, s.in_air), (fov, y, false));
    }
    i.dead = false;
    flow.update(&mut s, &i, &rig, 2.0);
    assert_eq!((flow.current.node, flow.current.depth()), (Node::Ground, 0));
    i.airborne = true;
    flow.update(&mut s, &i, &rig, 2.0);
    assert!(s.in_air);
    flow.update(&mut s, &i, &rig, 2.0);
    assert_eq!((flow.current.node, flow.current.depth()), (Node::Airborne, 0));
    i.dead = true;
    flow.update(&mut s, &i, &rig, 2.0);
    assert_eq!(flow.current.node, Node::Death);
    i.dead = false;
    flow.update(&mut s, &i, &rig, 2.0);
    assert_eq!(flow.current.node, Node::Airborne, "the death graph held BLN_InAir true");
}

#[test]
fn grind_fades_in_through_the_conduit() {
    let (mut s, mut i, rig) = (shared(), inputs(), rig());
    let mut flow = Flow::default();
    s.in_air = false;
    flow.update(&mut s, &i, &rig, 2.0);
    assert_eq!(flow.current.node, Node::Ground);
    i.grinding = true;
    let mut depths = Vec::new();
    for _ in 0..4 {
        flow.update(&mut s, &i, &rig, 2.0);
        assert_eq!(flow.current.node, Node::Grind);
        depths.push(flow.current.depth());
    }
    assert_eq!(depths, [2, 2, 2, 0]);
    let w: Vec<f32> = [0.25_f32, 0.5, 0.75].iter().map(|&t| curve_weight(t, 0)).collect();
    for (w, t) in w.iter().zip([0.25_f32, 0.5, 0.75]) {
        assert!((w - (t * std::f32::consts::FRAC_PI_2).cos()).abs() < 1e-6);
    }
    i.grinding = false;
    flow.update(&mut s, &i, &rig, 2.0);
    assert_eq!((flow.current.node, flow.current.depth()), (Node::Ground, 0));
}

#[test]
fn grind_graph_is_the_ground_graph_closer_and_lower() {
    let i = inputs();
    let (mut sg, mut sr) = (shared(), shared());
    let g = step(Variant::Ground, &mut GraphState::default(), &mut sg, &i, 2.0);
    let r = step(Variant::Grind, &mut GraphState::default(), &mut sr, &i, 2.0);
    assert_eq!((sg.y_offset, sr.y_offset), (0.8, 0.25));
    assert!((g.interest - r.interest - sg.up * 0.55).length() < 1e-4);
    assert!(r.position.distance(r.interest) < g.position.distance(g.interest) - 1.0);
    assert!((r.rotation * Vec3::X).y < (g.rotation * Vec3::X).y);
    assert_eq!((sg.fov, sr.fov), (g.fov_deg, r.fov_deg));
}

#[test]
fn tags_set_the_offset_the_trigger_and_meta_blending() {
    let (mut s, mut i, rig) = (shared(), inputs(), rig());
    let mut flow = Flow::default();
    s.in_air = false;
    flow.update(&mut s, &i, &rig, 2.0);
    assert_eq!(s.y_offset, 0.8, "the ground graph read the offset before the monitor");
    flow.monitor(&mut s);
    assert!(s.transition_triggered && s.transition_speed_set == 0.1 && s.player_offset_y == 0.3);
    (s.transition_triggered, s.transition_speed_set) = (false, 0.28);
    flow.update(&mut s, &i, &rig, 2.0);
    assert_eq!(s.y_offset, 0.3, "the node's second update reads the tag's value");
    flow.monitor(&mut s);
    assert_eq!(s.player_offset_y, 0.8, "the window left: ResetOnLeave");
    assert_eq!(s.transition_speed_set, 0.300_7, "the released conduit's set exited: ResetOnLeave");
    i.dead = true;
    flow.update(&mut s, &i, &rig, 2.0);
    flow.monitor(&mut s);
    assert!(!s.enable_meta_blending && s.transition_speed_set == 0.8 && s.transition_triggered);
    i.dead = false;
    for _ in 0..3 {
        flow.update(&mut s, &i, &rig, 2.0);
        flow.monitor(&mut s);
    }
    assert_eq!(flow.current.node, Node::Ground);
    assert!(!s.enable_meta_blending, "no ResetOnLeave: off until a rewind restores the camera's state");
}

#[test]
fn stateless_filters_follow_at_their_speed() {
    assert_eq!(stateless_lowpass(0.1, 0.01, 0.7, 0.9, true, 2.0), (0.1, 0.1));
    let (f, d) = stateless_lowpass(1.0, 1.0, 0.2, 0.4, false, 2.0);
    assert!((f - 0.4).abs() < 1e-6 && (d - 1.0).abs() < 1e-6, "the second stage reads the old desired");
    let (p, r) = (Vec3::new(3.0, 2.0, 1.0), Vec3::new(1.0, 0.0, 0.0));
    let (out, o, d) = position_offset_filter(p, r, 1.0, Vec3::ZERO, Vec3::ZERO, false, 2.0);
    assert!(out.distance(p) < 1e-6 && o.distance(p - r) < 1e-6 && d.distance(p - r) < 1e-6);
    let held = position_offset_filter(p, r, 0.0, Vec3::X, Vec3::Y, false, 2.0);
    assert_eq!(held, (r + Vec3::X, Vec3::X, Vec3::Y));
}

#[test]
fn blend_moves_new_towards_old() {
    let new = rig();
    let old =
        Channels { position: Vec3::ZERO, rotation: -Quat::from_rotation_y(1.0), interest: Vec3::X, fov_deg: 90.0 };
    let b = blend(&new, &old, 0.0);
    assert!(b.position == new.position && b.rotation.angle_between(new.rotation) < 1e-5);
    let b = blend(&new, &old, 1.0);
    assert!(b.position == old.position && b.rotation.angle_between(old.rotation) < 1e-5);
    let b = blend(&new, &old, 0.5);
    assert!(b.position.distance(new.position * 0.5) < 1e-5 && (b.fov_deg - 93.5).abs() < 1e-5);
    let half = new.rotation.angle_between(old.rotation) * 0.5;
    assert!((b.rotation.angle_between(new.rotation) - half).abs() < 1e-3);
}
