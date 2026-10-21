use glam::Vec3;

use crate::rider::{World, drag_accel};

pub const BALLISTIC_PROBE_RADIUS: f32 = 0.5;
pub const BALLISTIC_PROBE_MAX_DISTANCE: f32 = 200.0;
pub const BALLISTIC_PROBE_MAX_STEPS: u32 = 128;
const DT_EPS: f32 = 1.192_092_9e-7;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BallisticRay {
    pub valid: bool,
    pub ticks: f32,
    pub normal: Vec3,
    pub end_position: Vec3,
    pub end_velocity: Vec3,
    pub normal_proj_distance: f32,
    pub normal_proj_speed: f32,
    pub moving_towards_impact: bool,
    pub steps: u32,
    pub hit: Option<crate::collision::Hit>,
}

impl Default for BallisticRay {
    fn default() -> Self {
        Self {
            valid: false,
            ticks: 0.0,
            normal: Vec3::Y,
            end_position: Vec3::ZERO,
            end_velocity: Vec3::ZERO,
            normal_proj_distance: 0.0,
            normal_proj_speed: 0.0,
            moving_towards_impact: false,
            steps: 0,
            hit: None,
        }
    }
}

impl BallisticRay {
    pub fn landing(&self) -> Option<crate::air::LandingPrediction> {
        self.valid.then_some(crate::air::LandingPrediction { ticks: self.ticks, normal: self.normal })
    }
}

impl World {
    pub fn ballistic_probe(&self, traj: Vec3, v: Vec3, wind: Vec3, in_air: bool, dt: f32) -> BallisticRay {
        let r = &self.props;
        let (kh, kv, g) = if in_air {
            (r.k_h_air, r.k_v_air, r.g_air.y.abs())
        } else {
            (r.k_h_ground, r.k_v_ground, r.g_ground.y.abs())
        };
        let mut out = BallisticRay::default();
        if dt.abs() <= DT_EPS {
            out.normal_proj_distance = traj.y.abs();
            out.moving_towards_impact = v.y < 0.0;
            return out;
        }
        let step = |v: Vec3| {
            let mut a = drag_accel(kh, kv, v, wind, dt);
            a.y -= g;
            v + a * dt
        };
        let (mut p, mut vel, mut steps) = (traj, v, 0u32);
        while steps < BALLISTIC_PROBE_MAX_STEPS && (p - traj).length_squared() < BALLISTIC_PROBE_MAX_DISTANCE.powi(2) {
            vel = step(vel);
            p += vel * dt;
            steps += 1;
        }
        out.steps = steps;
        let (mut p, mut vel, mut ticks) = (traj, v, 0.0f32);
        for _ in 0..steps {
            let v_next = step(vel);
            let d = v_next * dt;
            let len = d.length();
            if len > 0.0
                && let Some(hit) = self.collision.sphere_cast(p, d / len, len, BALLISTIC_PROBE_RADIUS)
            {
                out.valid = true;
                out.ticks = ticks + dt * (hit.t / len).clamp(0.0, 1.0);
                out.normal = hit.normal;
                out.end_position = hit.point;
                out.end_velocity = vel;
                out.normal_proj_distance = (traj - hit.point).dot(hit.normal).abs();
                out.normal_proj_speed = vel.dot(hit.normal).abs();
                out.moving_towards_impact = v.dot(hit.normal) < 0.0;
                out.hit = Some(hit);
                return out;
            }
            ticks += dt;
            vel = v_next;
            p += d;
        }
        out.ticks = ticks;
        out.end_position = p;
        out.end_velocity = vel;
        out.normal_proj_distance = traj.y.abs();
        out.moving_towards_impact = v.y < 0.0;
        out
    }
}

#[cfg(test)]
mod tests;
