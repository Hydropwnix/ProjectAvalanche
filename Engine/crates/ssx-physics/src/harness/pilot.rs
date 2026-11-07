use glam::Vec3;

use crate::rider::{RiderState, World};

const DEPLOY_HOLD_S: f32 = 0.8;
const FLARE_S: f32 = 1.6;
const FLARE_USE: f32 = 0.85;
const GROUND_RAY_M: f32 = 300.0;

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Pilot {
    deployed_at: Option<f32>,
}

impl Pilot {
    pub(super) fn hold(&mut self, s: &RiderState, time: f32) -> bool {
        if !s.state.is_wingsuit() {
            self.deployed_at = None;
            return false;
        }
        let at = *self.deployed_at.get_or_insert(time);
        time - at < DEPLOY_HOLD_S
    }

    pub(super) fn flare(s: &RiderState, w: &World) -> bool {
        if s.state == crate::rider::RideState::WingsuitFly && s.wingsuit.use_percentage >= FLARE_USE {
            return true;
        }
        let vy = s.velocity.y * crate::TICKS_PER_SECOND;
        if vy >= -1.0 {
            return false;
        }
        let h = w.collision.ray_cast(s.traj + Vec3::Y * 0.5, -Vec3::Y, GROUND_RAY_M).map(|h| h.t - 0.5);
        h.is_some_and(|h| h / -vy < FLARE_S)
    }
}
