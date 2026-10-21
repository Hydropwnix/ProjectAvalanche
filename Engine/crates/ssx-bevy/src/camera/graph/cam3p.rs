use bevy::math::{Quat, Vec3};

use super::{
    CAMERA_FINISHED_OFFSET, Channels, DISTANCE, DISTANCE_AIR, DISTANCE_FAR, DISTANCE_GRIND, DISTANCE_PRESET,
    GraphState, HINGE_ANGLE, HINGE_DISTANCE, Inputs, LOOKAHEAD, LOOKAHEAD_TICKS, PITCH_CLAMP, Shared, Variant,
    decompose, frame_camera_x, hinge_or_reset, lerp, look_at, range_change, range_change_clamp, right_axis,
    scale_distance,
};

const AVALANCHE_INTENSITY_PRESET: f32 = 1.0;
pub(super) const AVALANCHE_STRENGTH: f32 = 3.0;
const AVALANCHE_STRENGTH_FAR: f32 = 2.0;
const AVALANCHE_MASS: (f32, f32) = (1.0, 5.0);
const AVALANCHE_RETURN_RATE: f32 = 0.01;
const AVALANCHE_MAX_DISTANCE_OFFSET: f32 = 3.0;
const AVALANCHE_ADDED_PITCH: f32 = -0.5;

pub(super) fn avalanche_terms(g: &mut GraphState, s: &mut Shared, mass: f32, strength: f32, dt: f32) -> (f32, f32) {
    let ramp = range_change_clamp(mass, AVALANCHE_MASS.0, AVALANCHE_MASS.1, 0.0, 1.0);
    let filtered = g.av_mass.step(ramp, 0.5, 0.5, false, dt);
    let returning = lerp(s.avalanche_intensity, 0.0, AVALANCHE_RETURN_RATE);
    s.avalanche_intensity = filtered.max(returning);
    let i = s.avalanche_intensity;
    let pitch = g.av_pitch.step(i * AVALANCHE_ADDED_PITCH, 0.3, 0.3, false, dt);
    let strength = AVALANCHE_INTENSITY_PRESET * strength;
    let added = g.av_distance.step(strength * lerp(0.0, AVALANCHE_MAX_DISTANCE_OFFSET, i), 0.1, 0.8, false, dt);
    s.added_distance = added;
    (pitch, added)
}

pub(super) fn avalanche_shake_trigger(s: &mut Shared, i: &Inputs) {
    s.avalanche_shake = f32::from(u8::from(i.alive && i.wipeout && i.bailed_by_avalanche));
}

const CARVING_SPEED: (f32, f32) = (15.0, 70.0);
const LAYER_DISTANCE: (f32, f32) = (3.0, 30.0);
const LAYER_VELOCITY: f32 = 0.5;

pub fn determine_carving_weight(
    speed: f32,
    mass: f32,
    distance_mod: f32,
    boosting: bool,
    uber: bool,
    alive: bool,
) -> f32 {
    if !(mass <= 0.0 && !uber && alive) {
        return 0.0;
    }
    if boosting {
        return 1.0;
    }
    let x = speed * distance_mod;
    let x = if -x >= 0.0 { 0.0 } else { x };
    if 1.0 - x >= 0.0 { x } else { 1.0 }
}

pub fn noise_layer_signals(s: &mut Shared, i: &Inputs, dist: f32) {
    let speed = range_change(i.speed, CARVING_SPEED.0, CARVING_SPEED.1, 0.0, 1.0);
    let near = range_change_clamp(dist, LAYER_DISTANCE.0, LAYER_DISTANCE.1, 1.0, 0.0);
    s.carving_trigger = f32::from(u8::from(i.alive));
    s.carving_weight = determine_carving_weight(speed, i.aggregate_mass, near, i.boosting, false, i.alive);
    s.wipeout_trigger = f32::from(u8::from(i.wipeout && i.alive));
    let moving = range_change_clamp(i.velocity.length(), 0.0, LAYER_VELOCITY, 0.0, 1.0);
    s.wipeout_weight = (s.wipeout_trigger * near).min(moving);
}

pub fn step(v: Variant, g: &mut GraphState, s: &mut Shared, i: &Inputs, dt: f32) -> Channels {
    let ground = !matches!(v, Variant::Air | Variant::FarAir);
    let far = matches!(v, Variant::FarGround | Variant::FarAir);
    let y_offset = match v {
        Variant::Ground => s.player_offset_y,
        Variant::Air => 0.7,
        Variant::Grind => 0.25,
        Variant::FarGround => 1.4,
        Variant::FarAir => 0.9,
    };
    s.y_offset = y_offset;
    let k_up = if ground { 0.05 } else { 0.1 };
    s.up = (s.up + (i.ground_normal - s.up) * k_up).try_normalize().unwrap_or(Vec3::Y);
    if !s.in_air {
        s.target_speed = i.speed;
    }
    s.camera_is_boosting = i.boosting;
    let boost_edge = !s.camera_is_boosting && i.boosting;
    let boost_fov = g.boost.step(s.fov_from_boost, 0.1, 0.05, false, dt);
    let dfov = 1.2 * (DISTANCE - DISTANCE_PRESET);
    let (fmin, fmax) = (90.0 + dfov, 105.0 + dfov);
    let ft = range_change_clamp(s.target_speed, 20.0, 57.0, fmin, fmax);
    s.fov_base = lerp(s.fov_base, ft, (0.015 * dt).clamp(0.0, 1.0)).max(ft);
    let fov = boost_fov + s.fov_base;
    let decayed = s.fov_from_boost - 0.3 * dt;
    let held = if i.boosting { decayed.max(16.0) } else { decayed.max(0.0) };
    s.fov_from_boost = if boost_edge { 60.0 } else { held };
    let strength = if far { AVALANCHE_STRENGTH_FAR } else { AVALANCHE_STRENGTH };
    let (av_pitch, av_distance) = avalanche_terms(g, s, i.aggregate_mass, strength, dt);
    avalanche_shake_trigger(s, i);
    let d0 = av_distance
        + match v {
            Variant::Ground => DISTANCE,
            Variant::Air => DISTANCE_AIR,
            Variant::Grind => DISTANCE_GRIND,
            Variant::FarGround | Variant::FarAir => DISTANCE_FAR,
        };
    let size = (fmin.to_radians() * 0.5).tan() / (fov.to_radians() * 0.5).tan();
    let dist = d0 * size * range_change(s.fov_base / fmin, 0.9, 1.2, 0.98, 1.05);
    noise_layer_signals(s, i, dist);
    let a0 = if ground { i.target_pos + i.target_up * 0.3 } else { i.target_pos };
    let a = a0 + s.up * y_offset;
    let hinge = hinge_or_reset(s);
    let (sin_h, cos_h) = HINGE_ANGLE.sin_cos();
    let d = (a0 - hinge).try_normalize().unwrap_or(Vec3::X);
    let flat = Vec3::new(d.x, 0.0, d.z);
    let clamped = d.y < -cos_h || d.y > cos_h;
    let horizontal = if clamped { scale_distance(Vec3::ZERO, flat, sin_h) } else { flat };
    let length = HINGE_DISTANCE.max(0.5 + d0);
    s.hinge = a0 - scale_distance(Vec3::ZERO, Vec3::new(0.0, d.y.clamp(-cos_h, cos_h), 0.0) + horizontal, length);
    let mut b = a + look_at(s.hinge, a0, 0.0) * Vec3::new(LOOKAHEAD, 0.0, 0.0);
    if ground {
        b += i.velocity * LOOKAHEAD_TICKS;
    }
    let (rider_span, ahead_span) = if v == Variant::FarGround { (0.25, 0.35) } else { (0.3, 0.7) };
    let pa =
        if i.track_finished { CAMERA_FINISHED_OFFSET } else { lerp(0.5 - rider_span, 0.5 + rider_span, s.player_skew) };
    let pb = if ground {
        lerp(0.5 - ahead_span, 0.5 + ahead_span, s.lookahead_skew)
    } else {
        lerp(0.25, 0.75, s.player_skew)
    };
    let lean_roll = if far { 0.01 } else { 0.25 };
    let roll = if ground { g.roll.step(i.lean * lean_roll, 0.01, 0.7, dt) } else { 0.0 };
    let (cam, target, orient) = frame_camera_x(a, b, pa, pb, dist, roll, s.fov_base);
    let po = av_pitch
        + match v {
            Variant::Ground => -0.12_f32,
            Variant::Air => -0.21,
            Variant::Grind => -0.54,
            Variant::FarGround => -0.13,
            Variant::FarAir => -0.15,
        };
    let po = po.clamp(-PITCH_CLAMP, PITCH_CLAMP);
    let tilt = decompose(orient).1;
    let span = if po < 0.0 { tilt + PITCH_CLAMP } else { PITCH_CLAMP - tilt };
    let cam = a + Quat::from_axis_angle(right_axis(orient), span * po) * (cam - a);
    s.in_air = i.airborne && !i.wipeout;
    let h = range_change_clamp(s.target_speed, 0.0, 70.0, 0.0, 1.0);
    let v_xz = lerp(0.8, 0.9, h).clamp(0.8, 0.9);
    let v_y = if s.in_air { 0.3 } else { lerp(0.5, 0.75, h).clamp(0.5, 0.75) };
    let v_y = g.humanize.step(v_y, 0.05, 0.05, false, dt);
    let xz = g.target_xz.step(target, 0.7, v_xz, false, dt);
    let filtered_y = g.target_y.step(target, 0.3, v_y, false, dt).y;
    let y = if s.in_air { target.y } else { filtered_y };
    let picked = if s.handycam { Vec3::new(xz.x, y, xz.z) } else { target };
    let target = g.target.step(picked, 0.5, 0.5, false, dt);
    let cam_f = g.camera.step(cam, 0.5, 0.3, false, dt);
    let gap = g.distance.step(cam_f.distance(target), 0.2, 0.2, false, dt);
    let eye = scale_distance(target, cam_f, gap);
    let at = if ground { eye } else { scale_distance(a, eye, cam.distance(a)) };
    let q = look_at(eye, target, roll);
    let tilt = decompose(q).1;
    let fix = Quat::from_axis_angle(right_axis(q), tilt.clamp(-PITCH_CLAMP, PITCH_CLAMP) - tilt);
    s.fov = fov;
    Channels { position: a + fix * (at - a), rotation: (fix * q).normalize(), interest: a, fov_deg: fov }
}

pub fn step_death(s: &mut Shared, i: &Inputs, rig: &Channels) -> Channels {
    s.up = i.target_pos.try_normalize().unwrap_or(Vec3::Y);
    Channels { fov_deg: s.fov_base, ..*rig }
}

pub const REVERSE_DISTANCE: f32 = 54.0;
const REVERSE_LOOKAHEAD: f32 = 10.0;
const REVERSE_PITCH_OFFSET: f32 = -0.81;
const REVERSE_PITCH_CLAMP: f32 = 0.8;
pub(super) const REVERSE_Y_OFFSET: f32 = -7.0;
const REVERSE_TILT: (f32, f32) = (-1.0, 0.3);
const REVERSE_FOV: f32 = 50.01;

pub fn step_reverse(s: &mut Shared, i: &Inputs) -> Channels {
    s.y_offset = REVERSE_Y_OFFSET;
    let a0 = i.target_pos + i.target_up * 0.3;
    s.up = (s.up + (i.ground_normal - s.up) * 0.05).try_normalize().unwrap_or(Vec3::Y);
    let a = a0 + s.up * REVERSE_Y_OFFSET;
    let hinge = hinge_or_reset(s);
    let (sin_h, cos_h) = HINGE_ANGLE.sin_cos();
    let d = (a0 - hinge).try_normalize().unwrap_or(Vec3::X);
    let flat = Vec3::new(d.x, 0.0, d.z);
    let clamped = d.y < -cos_h || d.y > cos_h;
    let horizontal = if clamped { scale_distance(Vec3::ZERO, flat, sin_h) } else { flat };
    let length = HINGE_DISTANCE.max(0.5 + REVERSE_DISTANCE);
    s.hinge = a0 - scale_distance(Vec3::ZERO, Vec3::new(0.0, d.y.clamp(-cos_h, cos_h), 0.0) + horizontal, length);
    let look = a + look_at(s.hinge, a0, 0.0) * Vec3::new(REVERSE_LOOKAHEAD, 0.0, 0.0);
    let cam = scale_distance(a, look, REVERSE_DISTANCE);
    let (pan, tilt, roll) = decompose(look_at(cam, a, 0.0));
    let tilt = tilt.clamp(REVERSE_TILT.0, REVERSE_TILT.1);
    let q = Quat::from_rotation_y(pan) * Quat::from_rotation_z(tilt) * Quat::from_rotation_x(roll);
    let po = REVERSE_PITCH_OFFSET.clamp(-PITCH_CLAMP, PITCH_CLAMP);
    let span = if po < 0.0 { decompose(q).1 + REVERSE_PITCH_CLAMP } else { REVERSE_PITCH_CLAMP - decompose(q).1 };
    let r = Quat::from_axis_angle(right_axis(q), span * po);
    let position = a + r * (cam - a);
    s.in_air = i.airborne && !i.wipeout;
    avalanche_shake_trigger(s, i);
    Channels { position, rotation: look_at(position, a, 0.0), interest: a, fov_deg: REVERSE_FOV }
}
