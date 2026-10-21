#![allow(clippy::neg_cmp_op_on_partial_ord)]

use anyhow::Result;
use glam::Vec3;

use crate::collision::CollisionWorld;
use crate::tuning::Bundle;

use super::path::{Solver, nearest_ridge};
use super::surface_analysis::SurfaceAnalysis;

const TINY: f32 = 1.525_879e-5;
const EPS: f32 = 1.192_092_9e-7;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SteeringTuning {
    pub scan_angle: f32,
    pub look_ahead_in_air: f32,
    pub ridgiprocity_threshold: f32,
    pub traj_offset: f32,
    pub min_ticks_before_steering: f32,
    pub min_ticks_for_wind_correction: f32,
    pub max_adjustment_radius: f32,
    pub max_path_angle: f32,
}

impl Default for SteeringTuning {
    fn default() -> Self {
        Self {
            scan_angle: 0.698_131_7,
            look_ahead_in_air: 2.0,
            ridgiprocity_threshold: 0.9,
            traj_offset: 0.3,
            min_ticks_before_steering: 5.0,
            min_ticks_for_wind_correction: 4.0,
            max_adjustment_radius: 0.1,
            max_path_angle: std::f32::consts::FRAC_PI_4,
        }
    }
}

impl SteeringTuning {
    pub fn load(bundle: &Bundle) -> Result<Self> {
        let p = bundle.view("PlayerUpdateSceneOp", "PlayerUpdateSceneOpAsset")?;
        Ok(Self {
            scan_angle: p.f32("GrindSteeringScanAngle")?,
            look_ahead_in_air: p.f32("GrindSteeringLookAheadInAir")?,
            ridgiprocity_threshold: p.f32("GrindSteeringRidgiprocityThreshold")?,
            traj_offset: p.f32("GrindSteeringTrajOffset")?,
            min_ticks_before_steering: p.f32("GrindSteeringMinTicksBeforeSteering")?,
            min_ticks_for_wind_correction: p.f32("GrindSteeringMinTicksForWindCorrection")?,
            max_adjustment_radius: p.f32("GrindSteeringMaxAdjustmentRadius")?,
            max_path_angle: p.asset("GrindTuningAsset")?.f32("MaxPathAngle")?,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GrindSteering {
    pub active: bool,
    pub target_pos: Vec3,
    pub target_vel: Vec3,
    pub wind: Vec3,
    pub time_to_land: f32,
    pub land_pos: Vec3,
    pub land_vel: Vec3,
}

pub struct SteeringInputs<'a> {
    pub sa: Option<&'a SurfaceAnalysis>,
    pub c: &'a CollisionWorld,
    pub t: &'a SteeringTuning,
    pub in_air: bool,
    pub ticks_airborne: f32,
    pub disabled: bool,
    pub traj: Vec3,
    pub velocity: Vec3,
    pub look_ahead_vel: Vec3,
    pub k_h_air: f32,
    pub k_v_air: f32,
    pub g_air: f32,
    pub dt: f32,
}

impl GrindSteering {
    fn clear(&mut self) {
        (self.active, self.target_pos, self.target_vel, self.wind, self.time_to_land) =
            (false, Vec3::ZERO, Vec3::ZERO, Vec3::ZERO, 0.0);
    }

    pub fn update(&mut self, i: &SteeringInputs) {
        let Some(sa) = i.sa else { return };
        if i.disabled {
            self.clear();
            return;
        }
        if !self.active {
            self.acquire(sa, i);
        } else if !i.in_air {
            self.clear();
        }
        self.predict(i);
        self.wind = Vec3::ZERO;
        if self.active && i.in_air {
            self.wind_correction(i);
        }
        if i.in_air && (!self.active || self.time_to_land.abs() <= EPS) {
            self.clear();
        }
    }
}

impl GrindSteering {
    fn acquire(&mut self, sa: &SurfaceAnalysis, i: &SteeringInputs) {
        let t = i.t;
        if !i.in_air || !(i.ticks_airborne > t.min_ticks_before_steering) {
            return;
        }
        let start = nearest_ridge(sa, 128, 128, 128, 128, t.ridgiprocity_threshold);
        let s = Solver::solve(sa, start, i.look_ahead_vel, i.dt, t.look_ahead_in_air, t.scan_angle);
        let Some(p) = s.extract() else { return };
        let n = p.dir_b.cross(Vec3::Y).cross(p.dir_b).normalize_or_zero();
        let b = i.look_ahead_vel - n * i.look_ahead_vel.dot(n);
        let ang = super::state::angle_between(b, p.dir_b);
        if p.peak_a > t.ridgiprocity_threshold
            && p.peak_b > t.ridgiprocity_threshold
            && super::state::rail_at(i.c, p.pos_a)
            && ang < t.max_path_angle
        {
            self.active = true;
            self.target_pos = p.pos_a + Vec3::Y * t.traj_offset;
            self.target_vel = p.dir_a;
        }
    }

    fn predict(&mut self, i: &SteeringInputs) {
        let target = self.target_pos;
        let (mut bt, mut bp, mut bv) = (0.0, i.traj, i.velocity);
        let mut bd = (i.traj - target).length_squared();
        if self.active {
            let (mut p, mut v, mut t) = (i.traj, i.velocity, 0.0);
            loop {
                let mut a = crate::rider::drag_accel(i.k_h_air, i.k_v_air, v, Vec3::ZERO, i.dt);
                a.y -= i.g_air;
                t += i.dt;
                v += a * i.dt;
                p += v * i.dt;
                let d = (target - p).length_squared();
                if !(d > bd) {
                    (bt, bp, bv, bd) = (t, p, v, d);
                }
                if !(v.y > 0.0 || p.y > target.y) {
                    break;
                }
            }
        }
        (self.time_to_land, self.land_pos, self.land_vel) = (bt, bp, bv);
    }

    fn wind_correction(&mut self, i: &SteeringInputs) {
        let t = i.t;
        let ttl = self.time_to_land;
        if ttl < t.min_ticks_for_wind_correction {
            return;
        }
        let lv = self.land_vel;
        let h = (lv.x * lv.x + lv.z * lv.z).sqrt();
        let dh = if h > 0.0 { (lv.x / h, lv.z / h) } else { (0.0, 0.0) };
        let e = (self.target_pos.x - self.land_pos.x, self.target_pos.z - self.land_pos.z);
        let along = e.0 * dh.0 + e.1 * dh.1;
        let lat = (e.0 - dh.0 * along, e.1 - dh.1 * along);
        let r = t.max_adjustment_radius * ttl;
        if !(lv.y < 0.0) || !(r * r > lat.0 * lat.0 + lat.1 * lat.1) {
            self.clear();
            return;
        }
        let w = solve_wind(i, i.velocity, self.target_pos - i.traj, ttl);
        self.wind = if w.length_squared() < 16.0 { w } else { Vec3::ZERO };
    }
}

fn simulate(i: &SteeringInputs, w: Vec3, v0: Vec3, t_total: f32) -> Vec3 {
    let (mut disp, mut v, mut t) = (Vec3::ZERO, v0, t_total);
    while t > TINY {
        let mut a = crate::rider::drag_accel(i.k_h_air, i.k_v_air, v, w, i.dt);
        a.y -= i.g_air;
        t -= i.dt;
        v += a * i.dt;
        disp += v * i.dt;
    }
    disp
}

fn solve_wind(i: &SteeringInputs, v: Vec3, delta: Vec3, ttl: f32) -> Vec3 {
    let close = |p: Vec3| (p - delta).abs().max_element() <= 0.01;
    let mut w = Vec3::ZERO;
    let mut p = simulate(i, w, v, ttl);
    if close(p) {
        return w;
    }
    let det = |a: Vec3, b: Vec3, c: Vec3| {
        a.x * (b.y * c.z - b.z * c.y) - b.x * (a.y * c.z - a.z * c.y) + c.x * (a.y * b.z - a.z * b.y)
    };
    for _ in 0..16 {
        let col = |k: usize| {
            let mut wk = w;
            wk[k] += TINY;
            (simulate(i, wk, v, ttl) - p) * (1.0 / TINY)
        };
        let (c0, c1, c2) = (col(0), col(1), col(2));
        let d = det(c0, c1, c2);
        if d == 0.0 || !(1.0 / d).is_finite() {
            break;
        }
        let err = p - delta;
        let x = Vec3::new(det(err, c1, c2) / d, det(c0, err, c2) / d, det(c0, c1, err) / d);
        if !x.is_finite() {
            break;
        }
        w -= x;
        p = simulate(i, w, v, ttl);
        if close(p) {
            break;
        }
    }
    w
}
