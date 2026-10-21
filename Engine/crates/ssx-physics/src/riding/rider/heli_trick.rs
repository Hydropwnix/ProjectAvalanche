use glam::{Quat, Vec3};

use super::{AERIAL, AERIAL_NODES, Input, RideState, RiderState, World, air};

pub const END_WINDOW_TICKS: f32 = 45.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HeliTrick {
    pub ticks: f32,
}

pub fn rate(s: &RiderState) -> f32 {
    s.heli.anim_time_scale.clamp(0.25, 3.0)
}

pub(super) fn enter(s: &mut RiderState, graph: bool) -> bool {
    if !s.heli.helicopter_interaction || s.janked.janked {
        return false;
    }
    // TODO(guess): without the rider's graph (unit tests) the node is not entered (its root motion is the graph's clip); settle: none needed in the game (a port limit of the graph-less step).
    if !graph {
        crate::guess_hit!();
        return false;
    }
    s.heli_trick = Some(HeliTrick::default());
    s.state = RideState::Airborne;
    true
}

pub(super) fn motion(
    s: &mut RiderState,
    w: &World,
    input: &Input,
    before: RideState,
    dt: f32,
    running: Option<&[String]>,
) -> (Vec3, Option<Quat>, &'static [&'static str]) {
    let Some(h) = s.heli_trick else { return (Vec3::ZERO, None, &[]) };
    let r = rate(s);
    let due = ((END_WINDOW_TICKS - h.ticks) / r).max(0.0);
    if due < dt {
        s.heli_trick = None;
        (s.state, s.first_air_tick) = (RideState::Airborne, true);
        s.tricks.restart_flow();
        return (air(s, w, input, before, dt - due), None, AERIAL);
    }
    s.heli_trick = Some(HeliTrick { ticks: h.ticks + dt * r });
    if running.is_some_and(|r| r.iter().any(|n| AERIAL_NODES.contains(&n.as_str()))) {
        return (air(s, w, input, before, dt), None, AERIAL);
    }
    (Vec3::ZERO, Some(s.traj_orientation), &[])
}
