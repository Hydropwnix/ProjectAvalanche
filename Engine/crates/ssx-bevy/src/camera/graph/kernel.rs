use core::ops::{Add, Mul, Sub};

use bevy::math::{Quat, Vec3};

fn alpha(k: f32, dt: f32) -> f32 {
    1.0 - (1.0 - k.clamp(0.0, 1.0)).powf(dt)
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Lowpass<T> {
    mid: T,
    out: T,
    started: bool,
}

impl<T: Copy + Add<Output = T> + Sub<Output = T> + Mul<f32, Output = T>> Lowpass<T> {
    pub fn step(&mut self, x: T, rate: f32, value: f32, force: bool, dt: f32) -> T {
        if !self.started || force {
            self.mid = x;
            self.out = x;
        } else {
            self.mid = self.mid + (x - self.mid) * alpha(rate, dt);
            self.out = self.out + (self.mid - self.out) * alpha(value, dt);
        }
        self.started = true;
        self.out
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Spring {
    pos: f32,
    vel: f32,
}

impl Spring {
    pub fn step(&mut self, x: f32, k: f32, c: f32, dt: f32) -> f32 {
        if k > 0.0 {
            let vel = self.vel + ((x - self.pos) * k - self.vel * c) * dt;
            let pos = self.pos + vel * dt;
            self.vel = if vel.is_finite() { vel } else { 0.0 };
            self.pos = if pos.is_finite() { pos } else { 0.0 };
        } else {
            self.pos = x;
            self.vel = 0.0;
        }
        self.pos
    }
}

pub fn lerp(a: f32, b: f32, s: f32) -> f32 {
    a + (b - a) * s
}

pub fn stateless_lowpass(x: f32, k: f32, filtered: f32, desired: f32, reset: bool, dt: f32) -> (f32, f32) {
    if reset {
        return (x, x);
    }
    let a = alpha(k, dt);
    (filtered + (desired - filtered) * a, desired + (x - desired) * a)
}

pub fn position_offset_filter(
    p: Vec3,
    r: Vec3,
    k: f32,
    offset: Vec3,
    desired: Vec3,
    reset: bool,
    dt: f32,
) -> (Vec3, Vec3, Vec3) {
    if reset {
        return (p, p - r, p - r);
    }
    let a = alpha(k, dt);
    let desired = desired + (p - r - desired) * a;
    let offset = offset + (desired - offset) * a;
    (r + offset, offset, desired)
}

pub fn range_change(x: f32, from1: f32, from2: f32, to1: f32, to2: f32) -> f32 {
    if (from2 - from1).abs() > 1.525_878_9e-5 { (x - from1) * (to2 - to1) / (from2 - from1) + to1 } else { from1 }
}

pub fn range_change_clamp(x: f32, from1: f32, from2: f32, to1: f32, to2: f32) -> f32 {
    if (from2 - from1).abs() > 1.525_878_9e-5 {
        range_change(x, from1, from2, to1, to2).clamp(to1.min(to2), to1.max(to2))
    } else {
        from1
    }
}

pub fn scale_distance(centre: Vec3, p: Vec3, d: f32) -> Vec3 {
    let v = p - centre;
    let v = if v.length_squared() < 1.525_878_9e-5 { Vec3::Z } else { v };
    centre + v.normalize() * d
}

pub fn look_at(eye: Vec3, target: Vec3, roll: f32) -> Quat {
    let d = target - eye;
    let len = d.length();
    if len < 1.525_878_9e-5 {
        return Quat::IDENTITY;
    }
    let n = d / len;
    let yaw = (-n.z).atan2(n.x);
    let pitch = n.y.clamp(-1.0, 1.0).asin();
    Quat::from_rotation_y(yaw) * Quat::from_rotation_z(pitch) * Quat::from_rotation_x(roll)
}

pub fn decompose(q: Quat) -> (f32, f32, f32) {
    let f = q * Vec3::X;
    let pan = (-f.z).atan2(f.x);
    let tilt = f.y.clamp(-1.0, 1.0).asin();
    let r = ((Quat::from_rotation_y(pan) * Quat::from_rotation_z(tilt)).inverse() * q).normalize();
    let x = 2.0 * r.w.clamp(-1.0, 1.0).acos();
    let u = x / std::f32::consts::TAU;
    let frac = u - u.trunc();
    let roll = (if frac - 0.5 >= 0.0 { frac - 1.0 } else { frac }) * std::f32::consts::TAU;
    let s2 = 1.0 - r.w * r.w;
    let negative = s2.abs() >= 1.525_878_9e-5 && r.x / s2.max(0.0).sqrt() < 0.0;
    (pan, tilt, if negative { -roll } else { roll })
}

pub fn right_axis(q: Quat) -> Vec3 {
    q * Vec3::Z
}

pub fn frame_camera_x(a: Vec3, b: Vec3, pa: f32, pb: f32, dist: f32, roll: f32, fov_deg: f32) -> (Vec3, Vec3, Quat) {
    let t = (fov_deg.to_radians() * 0.5).atan();
    let th_a = ((1.0 - 2.0 * pa) * t).atan();
    let th_b = ((1.0 - 2.0 * pb) * t).atan();
    let ab = a - b;
    let len = ab.length();
    let (u, len) = if len > 1e-4 { (ab / len, len) } else { (Vec3::NEG_X, 1.0) };
    let s = ((pb - pa).abs() * th_b.cos() / len).clamp(-1.0, 1.0).asin();
    let q1 = Quat::from_rotation_y(th_a - th_b + s);
    let q2 = Quat::from_rotation_y(-th_a);
    let position = a + q1 * (u * dist);
    let target = a + q1 * (u * dist + q2 * (-u * dist));
    (position, target, look_at(position, target, roll))
}

#[derive(Clone, Copy, Debug)]
pub struct SafeInputs {
    pub up: Vec3,
    pub target_pos: Vec3,
    pub safe: Vec3,
    pub ideal: Vec3,
    pub previous: Vec3,
    pub distance: f32,
    pub y_offset: f32,
    pub ticks: f32,
    pub speed: f32,
    pub collided: bool,
    pub was_colliding: bool,
}

const BLEND_IN: f32 = 15.0;

pub fn create_position_from_safe_values(c: &SafeInputs, dt: f32) -> (Vec3, f32) {
    let base = c.target_pos + c.up * c.y_offset;
    let dir = |v: Vec3| if v.length_squared() < 1.525_878_9e-5 { Vec3::Z } else { v.normalize() };
    let to_prev = c.previous - base;
    let current = to_prev.length().min(c.distance);
    let back = if current < 1.525_878_9e-5 { Vec3::Z } else { to_prev };
    let k = range_change_clamp(back.length_squared(), 4.0, 20.0, 0.9, 0.6);
    let stepped = current + 0.08 * (c.distance - current).signum() * f32::from(u8::from(c.distance != current));
    let kept = if (stepped - c.distance).abs() < 0.08 { c.distance } else { stepped };
    let from = base + back.normalize() * kept;
    let (to, w) = if c.ticks > BLEND_IN {
        (c.ideal, range_change_clamp(c.ticks, BLEND_IN + dt, 300.0, 0.0, 1.0))
    } else {
        (base + dir(c.safe - base) * c.distance, range_change_clamp(c.ticks, 0.0, BLEND_IN, 0.0, k))
    };
    let mut ticks = c.ticks + dt;
    if c.collided {
        ticks = if c.was_colliding { ticks.min(BLEND_IN) } else { (BLEND_IN - 1.3 * c.ticks).max(0.0) };
    }
    let w = range_change_clamp(c.speed, 0.0, 10.0, 0.0, w);
    (from + (to - from) * w, ticks)
}
