use bevy::math::{Quat, Vec3};

use super::cam3p::avalanche_shake_trigger;
use super::{
    Channels, DISTANCE, DISTANCE_PRESET, GraphState, HINGE_ANGLE, Inputs, Shared, decompose, hinge_or_reset, lerp,
    look_at, noise_layer_signals, range_change, range_change_clamp, right_axis, scale_distance,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant15 {
    Ground,
    Air,
    Rails,
    Wipeout,
    Heli,
}

struct Params15 {
    y_offset: f32,
    hinge: f32,
    pitch_clamp: f32,
    pitch_offset: f32,
    distance: f32,
}

fn params15(v: Variant15) -> Params15 {
    let (y_offset, hinge, pitch_offset, distance) = match v {
        Variant15::Ground => (0.53, 4.0, -0.26, 4.75),
        Variant15::Air => (0.6, 8.0, -0.39, 5.75),
        Variant15::Rails => (0.4, 4.0, -0.32, 4.75),
        Variant15::Wipeout => (0.5, 4.0, -0.33, 5.15),
        Variant15::Heli => (0.6, 8.0, -0.39, CAMERA_15P_DISTANCE_HELI),
    };
    Params15 { y_offset, hinge, pitch_clamp: 1.29, pitch_offset, distance }
}

pub const CAMERA_15P_DISTANCE: f32 = 4.75;
const CAMERA_15P_DISTANCE_HELI: f32 = 7.0;
const LOOKAHEAD_15P: f32 = 50.0;
const HELI_YAW: f32 = 0.99;

pub fn step_15p(v: Variant15, g: &mut GraphState, s: &mut Shared, i: &Inputs, camera_pos: Vec3, dt: f32) -> Channels {
    let p = params15(v);
    s.y_offset = p.y_offset;
    s.up = (s.up + (i.ground_normal - s.up) * 0.1).try_normalize().unwrap_or(Vec3::Y);
    if !s.in_air {
        s.target_speed = i.speed;
    }
    s.camera_is_boosting = i.boosting;
    let boost_fov = g.boost.step(s.fov_from_boost, 0.1, 0.05, false, dt);
    let dfov = 1.2 * (DISTANCE - DISTANCE_PRESET);
    let (fmin, fmax) = (90.0 + dfov, 105.0 + dfov);
    let ft = range_change_clamp(s.target_speed, 20.0, 57.0, fmin, fmax);
    s.fov_base = lerp(s.fov_base, ft, (0.015 * dt).clamp(0.0, 1.0)).max(ft);
    let fov = boost_fov + s.fov_base;
    let decayed = s.fov_from_boost - 0.3 * dt;
    s.fov_from_boost = if i.boosting { decayed.max(16.0) } else { decayed.max(0.0) };
    let size = (fmin.to_radians() * 0.5).tan() / (fov.to_radians() * 0.5).tan();
    let dist = p.distance * size * range_change(s.fov_base / fmin, 0.9, 1.2, 0.98, 1.05);
    noise_layer_signals(s, i, dist);
    avalanche_shake_trigger(s, i);
    let pivot = i.target_pos + g.velocity.step(i.velocity, 1.0, 0.8, false, dt);
    let a = pivot + s.up * p.y_offset;
    let hinge = hinge_or_reset(s);
    let (sin_h, cos_h) = HINGE_ANGLE.sin_cos();
    let d = (pivot - hinge).try_normalize().unwrap_or(Vec3::X);
    let flat = Vec3::new(d.x, 0.0, d.z);
    let clamped = d.y < -cos_h || d.y > cos_h;
    let horizontal = if clamped { scale_distance(Vec3::ZERO, flat, sin_h) } else { flat };
    s.hinge = pivot - scale_distance(Vec3::ZERO, Vec3::new(0.0, d.y.clamp(-cos_h, cos_h), 0.0) + horizontal, p.hinge);
    let b = a + look_at(s.hinge, pivot, 0.0) * Vec3::new(LOOKAHEAD_15P, 0.0, 0.0) + i.velocity * -0.1;
    let cam = scale_distance(b, a, b.distance(a) + dist);
    let q = look_at(cam, b, 0.0);
    let tilt = decompose(q).1;
    let span = if p.pitch_offset < 0.0 { tilt + p.pitch_clamp } else { p.pitch_clamp - tilt };
    let pitch = Quat::from_axis_angle(right_axis(q), span * p.pitch_offset);
    let yaw = if v == Variant15::Heli {
        let side = (i.target_pos - i.heli_position).cross(camera_pos - i.heli_position).y;
        Quat::from_axis_angle(Vec3::Y, if side > 0.0 { -HELI_YAW } else { HELI_YAW })
    } else {
        Quat::IDENTITY
    };
    let turn = pitch * yaw;
    s.in_air = i.airborne && !i.wipeout;
    s.fov = fov;
    Channels { position: a + turn * (cam - a), rotation: (turn * q).normalize(), interest: b, fov_deg: fov }
}
