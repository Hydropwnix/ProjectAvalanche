use glam::{Quat, Vec3};

use crate::collision::{Hit, SegmentQuery};
use crate::rider::World;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ImpactState {
    pub colliding: bool,
    pub in_soft_collision: bool,
    pub trigger_impact: bool,
    pub impact_this_frame: bool,
    pub impact_velocity: Vec3,
    pub pre_impact_velocity: Vec3,
    pub pre_impact_orientation: Quat,
    pub impact_speed: f32,
    pub impact_intensity: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct CorrectionInputs {
    pub from: Vec3,
    pub to: Vec3,
    pub velocity: Vec3,
    pub orientation: Quat,
    pub ground_valid: bool,
    pub distance_from_ground: f32,
    pub prev_colliding: bool,
    pub grinding: bool,
    pub prev_grinding: bool,
    pub dt: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct CorrectionOutput {
    pub position: Vec3,
    pub bounce: Option<Vec3>,
    pub soft: Option<Vec3>,
    pub impact: ImpactState,
}

const FAN_WEIGHTS: [f32; 8] = [0.9, 0.9, 1.0, 0.85, 0.75, 0.85, 1.0, 0.9];
const EPS_DOT: f32 = 1.525_878_9e-5;

fn normalize_or_zero(v: Vec3) -> Vec3 {
    v.try_normalize().unwrap_or(Vec3::ZERO)
}

#[derive(Clone, Copy, Debug)]
struct Res {
    valid: bool,
    p: Vec3,
    n: Vec3,
    frac: f32,
}

impl Res {
    fn of(hit: Option<Hit>, len: f32) -> Self {
        match hit {
            Some(h) => Res {
                valid: true,
                p: h.point,
                n: h.normal,
                frac: if len > 0.0 { (h.t / len).clamp(0.0, 1.0) } else { 0.0 },
            },
            None => Res { valid: false, p: Vec3::ZERO, n: Vec3::Y, frac: 1.0 },
        }
    }
}

impl World {
    fn query(&self, start: Vec3, end: Vec3, radius: f32, two_sided: bool) -> Res {
        let q = SegmentQuery { start, end, radius, two_sided, class_mask: 0 };
        Res::of(self.collision.segment_query(&q), (end - start).length())
    }

    pub fn collision_correction(&self, i: &CorrectionInputs) -> CorrectionOutput {
        let q = &self.tuning.queries;
        let j = &self.tuning.janked;
        let mut out = CorrectionOutput { position: i.to, bounce: None, soft: None, impact: ImpactState::default() };
        let v = i.velocity;
        let speed = v.length();
        let (from, to) = (i.from, i.to);
        let d = to - from;
        let dist = d.length();
        let lookahead = dist + q.collision_probe_lookahead;
        let sep = q.probe_ray_separation.eval(speed).min(dist);
        let rh = j.hard_ray_radius;
        let rs = if i.ground_valid { j.soft_ray_radius.min(i.distance_from_ground) } else { j.soft_ray_radius };
        let k = q.correction_offset;
        let hard = self.query(from, to, rh, true);
        let soft = self.query(from, to, rs, true);
        if hard.valid {
            let (p, mut n) = (hard.p, hard.n);
            let mut corrected = None;
            if !i.prev_colliding && dist > rh && speed > 1.192_092_9e-7 {
                let vh = v / speed;
                let refv = if vh.y.abs() > 1.0 - EPS_DOT { Vec3::X } else { Vec3::Y };
                let s = normalize_or_zero(vh.cross(refv));
                let u = normalize_or_zero(s.cross(vh));
                let end = from + vh * lookahead;
                let mut seg = [(Vec3::ZERO, Vec3::ZERO); 24];
                for idx in 0..4 {
                    let th = std::f32::consts::TAU * idx as f32 / 8.0;
                    let o = (u * th.cos() + s * th.sin()) * sep;
                    seg[idx] = (from, from + o);
                    seg[idx + 4] = (from, from - o);
                    seg[idx + 8] = (from + o * 0.5, end + o * 0.5);
                    seg[idx + 12] = (from - o * 0.5, end - o * 0.5);
                    seg[idx + 16] = (from + o, end + o);
                    seg[idx + 20] = (from - o, end - o);
                }
                let r: Vec<Res> = seg.iter().map(|&(a, b)| self.query(a, b, rh, false)).collect();
                let (mut best, mut score) = (None, 0.0);
                for kq in 8..24 {
                    let jj = kq & 7;
                    if r[jj].frac < if kq < 16 { 0.5 } else { 1.0 } {
                        continue;
                    }
                    let access = seg[jj].1 - seg[jj].0;
                    let ok = !r[kq].valid || access.dot(r[kq].n) > -EPS_DOT;
                    if ok && FAN_WEIGHTS[jj] * r[kq].frac > score {
                        (best, score) = (Some(kq), FAN_WEIGHTS[jj] * r[kq].frac);
                    }
                }
                if let Some(kq) = best {
                    let (a, b) = seg[kq];
                    if r[kq].valid {
                        n = r[kq].n;
                    } else if (b - p).dot(n) < -EPS_DOT {
                        let e = b - p;
                        n = normalize_or_zero(normalize_or_zero(e.cross(n)).cross(e));
                    }
                    let reach = (dist * dist - (a - from).length_squared()).max(0.0).sqrt() / lookahead;
                    let f = (reach.min(r[kq].frac) - k / lookahead).max(0.0);
                    corrected = Some(a + (b - a) * f);
                }
            }
            let c = corrected.unwrap_or_else(|| {
                let f = if dist <= 1.192_092_9e-7 { 0.0 } else { (hard.frac - k / dist).max(0.0) };
                from + d * f
            });
            let push = self.query(c, c + n * (2.0 * k), rh, false);
            out.position = c + n * (2.0 * k * 0.5 * push.frac);
            if v.dot(n) < 0.0 {
                let vn = n * v.dot(n);
                let vb = (v - vn) - vn * q.coefficient_of_restitution;
                let dir = normalize_or_zero(vb);
                let e = q.energy_preservation.eval(dir.y.clamp(-1.0, 1.0).acos());
                let len = vb.length();
                let sp = ((len + (speed - len) * e) * (1.0 - q.coefficient_of_friction)).max(q.min_bounce_speed);
                out.bounce = Some(dir * sp);
            }
            let up = i.orientation * Vec3::Y;
            if up.dot(n) <= q.cos_angle_from_upright_for_no_impact_on_landing && !i.grinding && !i.prev_grinding {
                out.impact = self.impact_surface_weights(i.orientation, v, n, i.prev_colliding);
            }
            out.impact.in_soft_collision = true;
        } else if soft.valid {
            let x = ((rs - (to - soft.p).dot(soft.n)) / (rs - rh)).clamp(0.0, 1.0);
            let a = q.soft_ray_acceleration.eval(x);
            out.soft = Some(normalize_or_zero(v + soft.n * (a * i.dt)) * speed);
            out.impact.in_soft_collision = true;
        }
        out
    }

    pub fn impact_surface_weights(&self, orientation: Quat, v: Vec3, n: Vec3, prev_colliding: bool) -> ImpactState {
        let q = &self.tuning.queries;
        let mut o = ImpactState { colliding: true, ..ImpactState::default() };
        let s = -v.dot(n);
        if !prev_colliding && s > q.min_impact_speed_threshold {
            o.trigger_impact = true;
            o.impact_this_frame = true;
            o.pre_impact_orientation = orientation;
            o.pre_impact_velocity = v;
            o.impact_velocity = orientation.conjugate() * (n * s);
            o.impact_speed = s;
            let span = q.max_impact_speed_threshold - q.min_impact_speed_threshold;
            o.impact_intensity = ((s - q.min_impact_speed_threshold) / span).clamp(0.0, 1.0);
        }
        o
    }
}

impl World {
    pub fn ghost_bounce(&self, v: Vec3, g: Vec3) -> Vec3 {
        let q = &self.tuning.queries;
        let push = (g - Vec3::splat(v.dot(g))) * q.coefficient_of_restitution;
        let len = push.length();
        if len.abs() <= 1.192_092_9e-7 { v + push } else { v + push / len * len.min(q.max_ghost_bounce_speed) }
    }

    pub fn ghost_impact_state(
        &self,
        base: ImpactState,
        orientation: Quat,
        v: Vec3,
        prev_colliding: bool,
    ) -> ImpactState {
        let mut o = ImpactState { colliding: true, ..base };
        if !prev_colliding {
            o.impact_this_frame = true;
            o.pre_impact_orientation = orientation;
            o.pre_impact_velocity = v;
            o.impact_velocity = Vec3::ZERO;
            o.impact_speed = 0.0;
            o.impact_intensity = 0.5;
        }
        o
    }
}

#[cfg(test)]
mod tests;
