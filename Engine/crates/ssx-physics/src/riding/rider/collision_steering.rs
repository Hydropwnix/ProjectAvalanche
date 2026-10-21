use glam::{Quat, Vec3};

use crate::character::CarvingTuning;
use crate::collision::{CollisionWorld, SegmentQuery};

pub const SWEEP_RADIUS: f32 = 0.4;
pub const FAN_SEGMENTS: usize = 14;
pub const OBSTACLE_MASK: u32 = !0x802;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Steered {
    pub dp: Vec3,
    pub v: Vec3,
    pub side: f32,
}

fn normalize0(v: Vec3) -> Vec3 {
    v.try_normalize().unwrap_or(Vec3::ZERO)
}

fn turn(v: Vec3, to: Vec3, angle: f32) -> Vec3 {
    match v.cross(to).try_normalize() {
        Some(axis) => Quat::from_axis_angle(axis, angle) * v,
        None => v,
    }
}

fn angle_between(a: Vec3, b: Vec3) -> f32 {
    normalize0(a).dot(normalize0(b)).clamp(-1.0, 1.0).acos()
}

pub fn collision_steering(
    c: &CollisionWorld,
    t: &CarvingTuning,
    p: Vec3,
    v: Vec3,
    n: Vec3,
    stick_dir: Vec3,
    dt: f32,
) -> Option<Steered> {
    let speed = v.length();
    let look = t.collision_look_ahead;
    let d = if stick_dir.abs().max_element() > 1.192_092_9e-7 { stick_dir } else { v };
    let clamp = 0.5 * (t.collision_scan_side_angle - t.collision_scan_forward_angle);
    let half = 0.5 * (t.collision_scan_side_angle + t.collision_scan_forward_angle);
    let wanted = if angle_between(v, d) > clamp { turn(v, d, clamp) } else { d };
    let target = normalize0(wanted) * speed * look;
    let straight = v * look;
    let step = half * 0.153_846_2;
    let ends = std::iter::once(straight)
        .chain((0..FAN_SEGMENTS).map(|k| Quat::from_axis_angle(n, -half + k as f32 * step) * target));
    let (mut best, mut best_progress) = (straight, 0.0f32);
    for e in ends {
        let q =
            SegmentQuery { start: p, end: p + e, radius: SWEEP_RADIUS, two_sided: false, class_mask: OBSTACLE_MASK };
        let end = c.segment_query(&q).map_or(p + e, |h| h.point);
        let progress = (end - p).dot(v);
        if progress > best_progress {
            (best, best_progress) = (end, progress);
        }
    }
    let new_v = normalize0(best - p) * speed;
    if (new_v * look - straight).abs().max_element() <= 0.1 {
        return None;
    }
    let side = if v.cross(new_v).dot(n) > 0.0 { -1.0 } else { 1.0 };
    let lim = dt * t.collision_steering_angular_vel;
    let v_out = if angle_between(v, new_v) > lim { turn(v, new_v, lim) } else { new_v };
    Some(Steered { dp: v_out * dt, v: v_out, side })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{add_quad, tuning};

    fn wall(class: u8) -> CollisionWorld {
        let mut c = CollisionWorld::default();
        let v = |y: f32, z: f32| Vec3::new(4.0, y, z);
        add_quad(&mut c, [v(-2.0, -1.0), v(-2.0, 1.0), v(2.0, 1.0), v(2.0, -1.0)], class);
        c.finish();
        c
    }

    #[test]
    fn a_rock_ahead_turns_the_velocity_at_the_steering_rate() {
        let t = tuning().carving;
        let v = Vec3::X * 0.3;
        let s = collision_steering(&wall(1), &t, Vec3::ZERO, v, Vec3::Y, Vec3::ZERO, 2.0).expect("steered");
        let turned = v.angle_between(s.v);
        assert!((turned - 2.0 * t.collision_steering_angular_vel).abs() < 1e-5, "{turned}");
        assert!((s.v.length() - 0.3).abs() < 1e-6 && s.dp == s.v * 2.0 && s.side.abs() == 1.0);
        assert!(collision_steering(&wall(0), &t, Vec3::ZERO, v, Vec3::Y, Vec3::ZERO, 2.0).is_none());
        assert!(collision_steering(&wall(1), &t, Vec3::new(-20.0, 0.0, 0.0), v, Vec3::Y, Vec3::ZERO, 2.0).is_none());
    }
}
