use bevy::prelude::*;

use crate::camera::graph;
use crate::ride::Ride;

pub(super) const TICKS_SINCE_COLLISION: f32 = 200.0;

const MAX_DEGREES_FOR_COLLISION_TEST: f32 = 10.0;
const MAX_DISTANCE_FOR_COLLISION_TEST: f32 = 1.0;

const QUERY_RADIUS: f32 = 0.25;
const QUERY_CLASS_MASK: u32 = 0x1C00;

const RAY_GRID: [Vec2; 9] = [
    Vec2::ZERO,
    Vec2::new(0.0, -1.0),
    Vec2::new(0.0, 1.0),
    Vec2::new(-1.0, 0.0),
    Vec2::new(1.0, 0.0),
    Vec2::new(0.0, -2.0),
    Vec2::new(0.0, 2.0),
    Vec2::new(-2.0, 0.0),
    Vec2::new(2.0, 0.0),
];

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Queries {
    local: Vec2,
    pub(super) collided: bool,
    pub(super) was_colliding: bool,
    pub(super) collided_all: bool,
    pub(super) was_colliding_all: bool,
}

pub(super) fn physics_queries(ride: &Ride, q: &mut Queries, cam: Vec3, p: Vec3, up: Vec3, dt: f32) -> Vec3 {
    let to_player = p - cam;
    let len = to_player.length();
    let f = if len > 0.0 { to_player / len } else { Vec3::ZERO };
    let dist = len.max(2.0);
    let d = if q.collided_all { graph::DISTANCE } else { dist };
    let side = up.cross(f).try_normalize().unwrap_or(Vec3::ZERO);
    let up2 = f.cross(side).try_normalize().unwrap_or(Vec3::ZERO);
    let a = (MAX_DISTANCE_FOR_COLLISION_TEST / dist).atan().min(0.5 * MAX_DEGREES_FOR_COLLISION_TEST.to_radians());
    let ray = -f * d * 1.1;
    let turns = [(side, 0.0), (side, -a), (side, a), (up2, a), (up2, -a), (side, -2.0 * a), (side, 2.0 * a)];
    let turns = turns.into_iter().chain([(up2, 2.0 * a), (up2, -2.0 * a)]);
    let collision = &ride.world().collision;
    let mut hit = [false; 9];
    let (mut free, mut best, mut best_fraction) = (None, 0, -1.0);
    for (i, (axis, angle)) in turns.enumerate() {
        let r = Quat::from_axis_angle(axis, angle) * ray;
        let l = r.length();
        let h = (l > 0.0).then(|| collision.sphere_cast_masked(p, r / l, l, QUERY_RADIUS, QUERY_CLASS_MASK)).flatten();
        hit[i] = h.is_some();
        let fraction = h.map_or(1.0, |h| (h.t / l).clamp(0.0, 1.0));
        if !hit[i] && free.is_none() {
            free = Some(i);
        }
        if fraction > best_fraction {
            (best, best_fraction) = (i, fraction);
        }
    }
    let chosen = free.unwrap_or(best);
    let mut local: Vec2 =
        (0..9).filter(|&i| hit[i] && i != chosen).map(|i| (RAY_GRID[chosen] - RAY_GRID[i]).normalize_or_zero()).sum();
    if q.was_colliding {
        local += (q.local - local) * (-0.2 * dt).exp();
    }
    let factor = 0.02 + 0.78 * (dist.clamp(1.0, 20.0) - 1.0) / 19.0;
    *q = Queries {
        local,
        collided: hit.iter().any(|&h| h),
        was_colliding: q.collided,
        collided_all: free.is_none(),
        was_colliding_all: q.collided_all,
    };
    cam + (side * local.x + up2 * local.y) * factor
}
