use bevy::prelude::*;

use super::{
    CAMERA_TYPE_15P, CAMERA_TYPE_FAR, CAMERA_TYPE_NORMAL, CAMERA_TYPE_REVERSE, CAMERA_TYPE_TUTORIAL, Chase,
    MAX_TICKS_IN_INTRO_CAM,
};
use crate::camera::graph::{self as graph, Channels, Inputs, SafeInputs};

pub(super) const REVERSE_ANCHOR_Y: f32 = 3.0;

const TRANSITION_SPEED_SET_DEFAULT: f32 = 0.280_72;
pub(super) const TYPE_TRANSITION_SPEED_SET_DEFAULT: f32 = 0.015;
const SPEED_FILTER_RATE: f32 = 0.01;

impl Chase {
    pub(super) fn meta3p(&mut self, i: &Inputs, c: &Channels, dt: f32, distance: f32, anchor_y: f32) -> Channels {
        let (s, m) = (&mut self.shared, &mut self.meta);
        let triggered = s.transition_triggered;
        m.ideal_distance = distance;
        let anchor = i.target_pos + i.target_up * anchor_y;
        let roll = graph::decompose(c.rotation).2;
        let fwd = c.rotation * Vec3::X;
        s.transition_triggered = false;
        if !triggered {
            s.transition_speed_set = TRANSITION_SPEED_SET_DEFAULT;
        }
        let target_speed = if triggered { s.transition_speed_set } else { 1.0 };
        (m.filtered_speed, m.filtered_desired_speed) = graph::stateless_lowpass(
            target_speed,
            SPEED_FILTER_RATE,
            m.filtered_speed,
            m.filtered_desired_speed,
            triggered,
            dt,
        );
        let k = m.filtered_speed;
        m.transition_speed_current = k;
        let look = c.position + fwd * (c.interest - c.position).dot(fwd).max(0.5);
        let blend_reset = m.trigger_blend_reset;
        let roll = self.meta3p.roll.step(roll, k, k, blend_reset, dt);
        let fov = self.meta3p.fov.step(s.fov, k, k, blend_reset, dt);
        let (target, to, td) =
            graph::position_offset_filter(look, anchor, k, m.target_offset, m.target_desired, blend_reset, dt);
        let (position, po, pd) = graph::position_offset_filter(
            c.position,
            anchor,
            k,
            m.position_offset,
            m.position_desired,
            blend_reset,
            dt,
        );
        (m.target_offset, m.target_desired, m.position_offset, m.position_desired) = (to, td, po, pd);
        s.fov = fov;
        Channels { position, rotation: graph::look_at(position, target, roll), interest: c.interest, fov_deg: fov }
    }

    pub(super) fn meta15p(&mut self, i: &Inputs, c: &Channels, dt: f32) -> Channels {
        let (s, m, f) = (&mut self.shared, &mut self.meta, &mut self.meta15);
        let triggered = s.transition_triggered;
        m.ideal_distance = graph::CAMERA_15P_DISTANCE;
        let velocity = f.velocity.step(i.velocity, 1.0, 0.8, false, dt);
        let anchor = i.target_pos + i.target_up * 0.3;
        let roll = graph::decompose(c.rotation).2;
        let fwd = c.rotation * Vec3::X;
        s.transition_triggered = false;
        if !triggered {
            s.transition_speed_set = TRANSITION_SPEED_SET_DEFAULT;
        }
        let rel = f.pivot.step(i.target_pos + velocity - anchor, 0.05, 0.05, false, dt);
        let reference = anchor + rel;
        let target_speed = if triggered { s.transition_speed_set } else { 1.0 };
        (m.filtered_speed, m.filtered_desired_speed) = graph::stateless_lowpass(
            target_speed,
            SPEED_FILTER_RATE,
            m.filtered_speed,
            m.filtered_desired_speed,
            triggered,
            dt,
        );
        let k = m.filtered_speed;
        m.transition_speed_current = k;
        let look = c.position + fwd * (c.interest - c.position).dot(fwd).max(0.5);
        let blend_reset = m.trigger_blend_reset;
        let roll = f.roll.step(roll, k, k, blend_reset, dt);
        let fov = f.fov.step(s.fov, k, k, blend_reset, dt);
        let (target, to, td) =
            graph::position_offset_filter(look, reference, k, m.target_offset, m.target_desired, blend_reset, dt);
        let (position, po, pd) = graph::position_offset_filter(
            c.position,
            reference,
            k,
            m.position_offset,
            m.position_desired,
            blend_reset,
            dt,
        );
        (m.target_offset, m.target_desired, m.position_offset, m.position_desired) = (to, td, po, pd);
        s.camera_pos = position;
        s.fov = fov;
        Channels { position, rotation: graph::look_at(position, target, roll), interest: c.interest, fov_deg: fov }
    }

    pub(super) fn meta(&mut self, i: &Inputs, c: &Channels, safe: Option<Vec3>, dt: f32) -> Channels {
        let (s, m) = (&mut self.shared, &mut self.meta);
        let not_snapped = !m.snap;
        let reset = m.trigger_blend_reset;
        let (ty, fallback, ticks) = game_entry_camera_type(m.ticks_in_intro_cam, m.camera_type, m.fallback_type, dt);
        m.ticks_in_intro_cam = ticks;
        let type_changed = ty != m.camera_type_last;
        if type_changed {
            m.camera_type_last = ty;
        }
        let restart = type_changed || reset;
        let keep_set = not_snapped && s.enable_meta_blending;
        m.snap = i.back_press && ty != CAMERA_TYPE_TUTORIAL && ty != CAMERA_TYPE_REVERSE;
        let cycled = if m.snap { cycle_camera_type(ty, &mut m.cycle_index) } else { ty };
        m.camera_type = cycled;
        let switched = m.camera_type != ty;
        let blend_type = switched || restart;
        m.fallback_type = if switched { m.camera_type } else { fallback };
        if !blend_type {
            m.type_speed_set = TYPE_TRANSITION_SPEED_SET_DEFAULT;
        }
        let k_set = if keep_set { m.type_speed_set } else { 1.0 };
        let target_speed = if blend_type { k_set } else { 1.0 };
        (m.meta_speed, m.meta_desired_speed) = graph::stateless_lowpass(
            target_speed,
            SPEED_FILTER_RATE,
            m.meta_speed,
            m.meta_desired_speed,
            blend_type,
            dt,
        );
        let k = m.meta_speed;
        m.type_transition_speed = k;
        let anchor = i.target_pos + i.target_up * 0.3;
        let roll = graph::decompose(c.rotation).2;
        let fwd = c.rotation * Vec3::X;
        let look = c.position + fwd * (c.interest - c.position).dot(fwd).max(0.5);
        let blend_reset = m.trigger_blend_reset;
        let roll = self.metac.roll.step(roll, k, k, blend_reset, dt);
        let fov = self.metac.fov.step(s.fov, k, k, blend_reset, dt);
        s.fov = fov;
        let (look, to, td) = graph::position_offset_filter(
            look,
            anchor,
            k,
            m.meta_target_offset,
            m.meta_target_desired,
            blend_reset,
            dt,
        );
        let (ideal, po, pd) = graph::position_offset_filter(
            c.position,
            anchor,
            k,
            m.meta_position_offset,
            m.meta_position_desired,
            blend_reset,
            dt,
        );
        (m.meta_target_offset, m.meta_target_desired, m.meta_position_offset, m.meta_position_desired) =
            (to, td, po, pd);
        m.ticks_to_force_avoidance = (m.ticks_to_force_avoidance - dt).max(0.0);
        let speed = if m.ticks_to_force_avoidance > 0.0 { 20.0 } else { i.speed };
        if m.snap {
            m.ticks_to_force_avoidance = 120.0;
        }
        let q = self.queries;
        let ticks = if q.collided_all && !q.was_colliding_all { 0.0 } else { self.ticks_since_collision };
        let (position, ticks) = graph::create_position_from_safe_values(
            &SafeInputs {
                up: s.up,
                target_pos: i.target_pos,
                safe: safe.unwrap_or(ideal),
                ideal,
                previous: self.previous.unwrap_or(ideal),
                distance: m.ideal_distance + s.added_distance,
                y_offset: s.y_offset,
                ticks,
                speed,
                collided: q.collided,
                was_colliding: q.was_colliding,
            },
            dt,
        );
        self.ticks_since_collision = ticks;
        self.camera_pos = Some(position);
        s.camera_pos = position;
        let rotation = graph::look_at(position, look, roll);
        self.camera_orientation = rotation;
        let on = s.enable_meta_blending;
        Channels {
            position: if on { position } else { c.position },
            rotation: if on { rotation } else { c.rotation },
            interest: c.interest,
            fov_deg: fov,
        }
    }
}

fn game_entry_camera_type(ticks: f32, ty: i32, fallback: i32, dt: f32) -> (i32, i32, f32) {
    if ty == CAMERA_TYPE_REVERSE || ty == CAMERA_TYPE_TUTORIAL {
        return (ty, ty, ticks);
    }
    let next = (ticks + dt).min(1000.0);
    if ticks <= 0.0 {
        (CAMERA_TYPE_15P, ty, next)
    } else if ticks > MAX_TICKS_IN_INTRO_CAM {
        (fallback, fallback, next)
    } else {
        (ty, fallback, next)
    }
}

const CAMERA_TYPE_CYCLE: [i32; 3] = [CAMERA_TYPE_15P, CAMERA_TYPE_NORMAL, CAMERA_TYPE_FAR];

fn cycle_camera_type(ty: i32, index: &mut i32) -> i32 {
    let n = CAMERA_TYPE_CYCLE.len() as i32;
    *index = match CAMERA_TYPE_CYCLE.iter().rposition(|&t| t == ty) {
        Some(0) => n - 1,
        Some(k) => k as i32 - 1,
        None => 0,
    };
    CAMERA_TYPE_CYCLE[*index as usize]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_game_entry_camera_type_runs_its_intro() {
        assert_eq!(game_entry_camera_type(0.0, 0, 0, 2.0), (CAMERA_TYPE_15P, 0, 2.0));
        assert_eq!(game_entry_camera_type(4.0, CAMERA_TYPE_15P, 0, 2.0), (CAMERA_TYPE_15P, 0, 6.0));
        assert_eq!(game_entry_camera_type(6.0, CAMERA_TYPE_15P, 0, 2.0), (0, 0, 8.0));
        assert_eq!(game_entry_camera_type(999.0, 0, 0, 2.0), (0, 0, 1000.0));
        assert_eq!(game_entry_camera_type(0.0, CAMERA_TYPE_REVERSE, 0, 2.0), (2, 2, 0.0));
    }

    #[test]
    fn back_cycles_normal_15p_far() {
        let mut index = -1;
        assert_eq!(cycle_camera_type(CAMERA_TYPE_NORMAL, &mut index), CAMERA_TYPE_15P);
        assert_eq!(index, 0);
        assert_eq!(cycle_camera_type(CAMERA_TYPE_15P, &mut index), CAMERA_TYPE_FAR);
        assert_eq!(index, 2);
        assert_eq!(cycle_camera_type(CAMERA_TYPE_FAR, &mut index), CAMERA_TYPE_NORMAL);
        assert_eq!(cycle_camera_type(CAMERA_TYPE_TUTORIAL, &mut index), CAMERA_TYPE_15P);
    }
}
