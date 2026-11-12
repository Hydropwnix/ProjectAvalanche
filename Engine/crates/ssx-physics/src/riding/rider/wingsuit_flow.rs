//! `SF_Ride`'s wingsuit nodes in the step (`Docs/research/wingsuit.md` §2, §5): the transitions into, between and
//! out of `state_wingsuit_enter` / `_fly` / `_exit` and their controllers with the transition blends (old nodes run
//! as blend slaves, the node three links down the chain frozen; a window splits an update, 0x82DE1F88; parts
//! composed by 0x82DC4498).

use glam::{Quat, Vec3};

use super::{Input, RideState, RiderState, World, air};
use crate::graph::stick_axes;
use crate::rootmotion::nlerp;
use crate::wingsuit::{
    BLEND_DEFAULT, BLEND_ENTER_TO_FLY, BLEND_EXIT_TO_AIRBORNE, BLEND_FLY_TO_EXIT, BLEND_JUMP_TO_ENTER,
    ControllerGameState, ControllerInputs, DEPLOY_END_TICKS, NodeBlend, OldNode, OldSide, RETRACT_END_TICKS,
    WingsuitController, ballistic_step, joint_of, traj_of,
};

/// The physics controller classes a wingsuit update stands for (the graph's physics delta sets).
pub const WINGSUIT_CLASSES: &[&str] =
    &["WingsuitControllerAsset", "BallisticControllerAsset", "AerialControllerAsset", "CarvingControllerAsset"];

/// `SF_Ride`'s `MaxActiveNodes` (3, `anim_all.cba`): `0x82B671B0` walks `MaxActiveNodes` links down the blend chain
/// from the new node and freezes the node it reaches (the fourth node, counting the current one), dropping that
/// node's own blend (`+0x288 = 0`). A frozen node keeps its pose and writes a zero delta (`FrozenController_Create`
/// 0x82B64778, `_Update` 0x82B64848). `SF_Camera`'s 1 freezes the blend's old side, as its doc records.
const MAX_ACTIVE_NODES: usize = 3;

/// `state_wingsuit_enter`'s entry conditions (module docs).
fn enter_allowed(s: &RiderState) -> bool {
    s.wingsuit.enabled && s.wingsuit.press()
}

/// One part of an update: the world translation, the rotation in the trajectory joint's frame (`None`: no channel).
#[derive(Clone, Copy, Debug)]
struct Part {
    dp: Vec3,
    q: Option<Quat>,
}

const ZERO: Part = Part { dp: Vec3::ZERO, q: Some(Quat::IDENTITY) };

/// `Pose_BlendDeltaTrajectory` 0x83184298 at new-side weight `w`.
fn mix(old: Part, new: Part, w: f32) -> Part {
    let q = match (old.q, new.q) {
        (Some(a), Some(b)) => Some(nlerp(a, b, w)),
        (a, None) => a,
        (None, b) => b,
    };
    Part { dp: old.dp.lerp(new.dp, w), q }
}

/// `DeltaAccumKernel_Compose` 0x82DC4498 in world terms (`j0` = the update's trajectory joint).
fn accumulate(first: Part, last: Part, j0: Quat) -> Part {
    let q1 = first.q.unwrap_or(Quat::IDENTITY);
    let dp = first.dp + j0 * (q1 * (j0.conjugate() * last.dp));
    let q = match (first.q, last.q) {
        (Some(a), Some(b)) => Some((b * a).normalize()),
        (a, None) => a,
        (None, b) => b,
    };
    Part { dp, q }
}

/// A `WingsuitController` update on the update's velocity, angular velocity, wind and stick (writes the shared state).
fn wingsuit_part(s: &mut RiderState, w: &World, input: &Input, c: &mut WingsuitController, j0: Quat, dt: f32) -> Part {
    let i = ControllerInputs {
        j: j0,
        velocity: s.traj_velocity,
        angular_velocity: s.filtered_angular_velocity,
        wind: s.grind.wind,
        stick: stick_axes(input.stick()),
        effectiveness: s.wingsuit.effectiveness,
        time_scalar: s.wingsuit.time_scalar,
    };
    let mut g =
        ControllerGameState { time: s.wingsuit.time, has_touched_ground: s.score.has_touched_ground, goofy: s.goofy };
    let out = c.update(&w.tuning.wingsuit, &w.props, &i, &mut g, dt);
    (s.wingsuit.time, s.score.has_touched_ground, s.goofy) = (g.time, g.has_touched_ground, g.goofy);
    let Some(o) = out else { return ZERO };
    (s.wingsuit.lift_velocity, s.wingsuit.use_percentage, s.wingsuit.last) = (o.lift, o.use_percentage, Some(o));
    Part { dp: j0 * o.t, q: Some(o.dq) }
}

/// `ballistic_airborne`'s update: the translation; the stance when it is the current node.
fn ballistic_part(s: &mut RiderState, w: &World, j0: Quat, current: bool, dt: f32) -> Part {
    let (v, goofy) = ballistic_step(&w.props, s.traj_velocity, s.grind.wind, j0, s.goofy, dt);
    if current {
        s.goofy = goofy;
    }
    Part { dp: if dt.abs() <= super::TINY { Vec3::ZERO } else { v * dt }, q: None }
}

/// The exited aerial controller (`AerialController_Exit` 0x82CD8BF0; `WriteOutputs` 0x82CDC6CC stops).
fn aerial_slave_part(s: &mut RiderState, w: &World, input: &Input, side: &mut OldSide, j0: Quat, dt: f32) -> Part {
    let OldSide::Aerial { alignment, jump, first } = side else { return ZERO };
    let saved = (s.state, s.alignment, s.goofy, s.air_jumped, s.first_air_tick, s.velocity);
    s.state = if *jump { RideState::Jump } else { RideState::Airborne };
    (s.alignment, s.first_air_tick) = (*alignment, *first);
    let dp = air(s, w, input, s.state, dt);
    (*alignment, *first) = (s.alignment, false);
    let q = j0.conjugate() * joint_of(*alignment);
    (s.state, s.alignment, s.goofy, s.air_jumped, s.first_air_tick, s.velocity) = saved;
    Part { dp, q: Some(q.normalize()) }
}

/// An old node of the chain at `depth` (1 = the node left by the current one).
fn old_part(s: &mut RiderState, w: &World, input: &Input, node: &mut OldNode, depth: usize, j0: Quat, dt: f32) -> Part {
    if depth >= MAX_ACTIVE_NODES {
        node.blend = None; // the game drops the frozen node's blend; it writes a zero delta (`FrozenController_Update`)
        return ZERO;
    }
    let new = match &mut node.side {
        OldSide::Wingsuit(c) => wingsuit_part(s, w, input, c, j0, dt),
        OldSide::Ballistic => ballistic_part(s, w, j0, false, dt),
        side @ OldSide::Aerial { .. } => aerial_slave_part(s, w, input, side, j0, dt),
    };
    blend_with(s, w, input, &mut node.blend, new, depth, j0, dt)
}

/// `new` blended with the old side of `blend` (advanced by `dt`; released when its weight is spent).
#[allow(clippy::too_many_arguments)]
fn blend_with(
    s: &mut RiderState,
    w: &World,
    input: &Input,
    blend: &mut Option<NodeBlend>,
    new: Part,
    depth: usize,
    j0: Quat,
    dt: f32,
) -> Part {
    let Some(b) = blend.as_mut() else { return new };
    match b.advance(dt) {
        None => {
            *blend = None;
            new
        }
        Some(w_old) => {
            let old = old_part(s, w, input, &mut b.old, depth + 1, j0, dt);
            mix(old, new, 1.0 - w_old)
        }
    }
}

/// The current node's controller for `dt` ticks with its blend chain.
fn node_part(s: &mut RiderState, w: &World, input: &Input, before: RideState, j0: Quat, dt: f32) -> Part {
    let new = match s.state {
        RideState::WingsuitEnter | RideState::WingsuitFly => {
            let mut c = s.wingsuit.controller.take().unwrap_or_else(|| WingsuitController::new(&w.tuning.wingsuit));
            let p = wingsuit_part(s, w, input, &mut c, j0, dt);
            s.wingsuit.controller = Some(c);
            p
        }
        RideState::WingsuitExit => ballistic_part(s, w, j0, true, dt),
        _ => {
            let dp = air(s, w, input, before, dt);
            Part { dp, q: Some((j0.conjugate() * joint_of(s.alignment)).normalize()) }
        }
    };
    let mut blend = s.wingsuit.blend.take();
    let p = blend_with(s, w, input, &mut blend, new, 0, j0, dt);
    s.wingsuit.blend = blend;
    p
}

/// The node left as a blend's old side: its controller and its own blend.
fn old_node(s: &mut RiderState, w: &World) -> OldNode {
    let side = match s.state {
        RideState::WingsuitEnter | RideState::WingsuitFly => OldSide::Wingsuit(
            s.wingsuit.controller.take().unwrap_or_else(|| WingsuitController::new(&w.tuning.wingsuit)),
        ),
        RideState::WingsuitExit => OldSide::Ballistic,
        state => OldSide::Aerial { alignment: s.alignment, jump: state == RideState::Jump, first: s.first_air_tick },
    };
    OldNode { side, blend: s.wingsuit.blend.take() }
}

/// A transition into the wingsuit node `to` with a `BlendCurve` of `blend_time` over the node left; the air states'
/// `SF_AirTricks` and `SF_Jump` are left.
fn enter(s: &mut RiderState, w: &World, to: RideState, blend_time: f32) {
    let from = s.state;
    let old = old_node(s, w);
    if matches!(from, RideState::Airborne | RideState::Jump) {
        // TODO(guess): the left `SF_AirTricks` as `TrickState::land` (the trick values reset at once; in the game the Lua tags `OnGrabExit` / `OnGrabReset` reset them when the monitor exits the left node's sets, 0x82B58D38, 0x82CEC2F0, 0x82CEAD68); settle: the tag kinds behind those scripts and their leave on the monitor's exit (0x8319BF80).
        crate::guess_hit!();
        s.tricks.land();
        s.jump_airborne = false;
    }
    s.state = to;
    s.wingsuit.signal = true;
    s.wingsuit.ticks = 0.0;
    s.wingsuit.controller = (to != RideState::WingsuitExit).then(|| WingsuitController::new(&w.tuning.wingsuit));
    s.wingsuit.blend = Some(NodeBlend::new(old, blend_time));
}

/// `state_wingsuit_exit → state_airborne` at the retract's `End`: a new aerial controller with the window's blend.
fn leave_to_airborne(s: &mut RiderState, w: &World) {
    let old = old_node(s, w);
    (s.state, s.first_air_tick, s.jump_airborne) = (RideState::Airborne, true, false);
    s.wingsuit.signal = false;
    s.wingsuit.ticks = 0.0;
    s.wingsuit.blend = Some(NodeBlend::new(old, BLEND_EXIT_TO_AIRBORNE));
}

/// `conduit_land → state_riding` from a wingsuit node: the chain is dropped (`TimeScaleBlend`).
pub(super) fn land(s: &mut RiderState) {
    s.state = RideState::Riding;
    s.wingsuit.signal = false;
    (s.wingsuit.controller, s.wingsuit.blend, s.wingsuit.ticks) = (None, None, 0.0);
}

/// The wingsuit transitions of `SF_Ride` at the update's start in list order (`correcting`: the recovery hop's
/// correction); a node entered takes its own due transitions too (0x82DE1F88). Returns whether the state changed.
pub(super) fn transition(s: &mut RiderState, w: &World, correcting: bool) -> bool {
    let mut changed = false;
    for _ in 0..25 {
        if !transition_once(s, w, correcting) {
            break;
        }
        changed = true;
        if !s.state.is_wingsuit() {
            break;
        }
    }
    changed
}

fn transition_once(s: &mut RiderState, w: &World, correcting: bool) -> bool {
    let very_close = s.close.is_very_close;
    match s.state {
        RideState::Airborne if !correcting && !s.recovering_from_uber_bail() => {
            if s.wingsuit.can_use() && enter_allowed(s) {
                enter(s, w, RideState::WingsuitEnter, BLEND_DEFAULT);
            } else if s.wingsuit.signal {
                enter(s, w, RideState::WingsuitFly, BLEND_DEFAULT);
            } else {
                return false;
            }
        }
        // `BLN_InAir` (set after the update that ran the air node): true once a jump state ran.
        RideState::Jump if s.wingsuit.can_use() && enter_allowed(s) => {
            enter(s, w, RideState::WingsuitEnter, BLEND_JUMP_TO_ENTER)
        }
        RideState::WingsuitEnter | RideState::WingsuitFly if very_close => land(s),
        RideState::WingsuitExit if very_close && !s.uber_trigger => land(s),
        // `fly → exit`: `BlendCurve` 15, the default 8 with the board particle.
        RideState::WingsuitFly if !s.wingsuit.hold => {
            let blend = if s.board_particle.enabled { BLEND_DEFAULT } else { BLEND_FLY_TO_EXIT };
            enter(s, w, RideState::WingsuitExit, blend)
        }
        _ => return false,
    }
    true
}

/// The wingsuit nodes' physics for the update: the displacement and the orientation it leaves; a window splits the
/// update. `flags` = `ride`'s.
pub(super) fn motion(
    s: &mut RiderState,
    w: &World,
    input: &Input,
    before: RideState,
    flags: (bool, bool),
    dt: f32,
) -> (Vec3, Option<Quat>) {
    let t0 = s.traj_orientation;
    let j0 = joint_of(t0);
    let window = match s.state {
        RideState::WingsuitEnter => Some(DEPLOY_END_TICKS),
        RideState::WingsuitExit => Some(RETRACT_END_TICKS),
        _ => None,
    };
    let due = window.map(|e| (e - s.wingsuit.ticks).max(0.0)).filter(|&d| d < dt);
    let part = match due {
        Some(due) => {
            let first = node_part(s, w, input, before, j0, due);
            if s.state == RideState::WingsuitEnter {
                let old = old_node(s, w);
                s.state = RideState::WingsuitFly;
                (s.wingsuit.ticks, s.wingsuit.controller) = (0.0, Some(WingsuitController::new(&w.tuning.wingsuit)));
                s.wingsuit.blend = Some(NodeBlend::new(old, BLEND_ENTER_TO_FLY));
                transition(s, w, false);
            } else {
                leave_to_airborne(s, w);
                if s.close.is_very_close {
                    land(s); // `state_airborne`'s `conduit_land`, due at the window
                }
            }
            let rest = dt - due;
            let last = if s.state == RideState::Riding {
                s.on_ground = true;
                super::touchdown(s, w, rest);
                let dp = super::ride(s, w, input, flags, rest);
                Part { dp, q: Some((j0.conjugate() * joint_of(super::ground_trajectory(s))).normalize()) }
            } else {
                let p = node_part(s, w, input, RideState::WingsuitExit, j0, rest);
                s.wingsuit.ticks = rest;
                p
            };
            accumulate(first, last, j0)
        }
        None => {
            let p = node_part(s, w, input, before, j0, dt);
            s.wingsuit.ticks += dt;
            p
        }
    };
    let root = traj_of(j0 * part.q.unwrap_or(Quat::IDENTITY));
    if s.state.is_wingsuit() {
        s.alignment = root;
    }
    if dt.abs() > super::TINY && s.state != RideState::Riding {
        s.velocity = part.dp / dt;
    }
    (part.dp, Some(root))
}
