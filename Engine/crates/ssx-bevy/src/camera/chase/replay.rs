use bevy::prelude::*;
use ssx_physics::collision::{CollisionWorld, Hit};

use super::collision::sweep_hits;
use super::{CAMERA_TYPE_NORMAL, Chase, Snapshot, StepContext};
use crate::camera::graph::Inputs;
use crate::camera::{GAME_NEAR, GAME_REST_HFOV_DEG};

#[derive(Clone, Copy, Debug)]
pub struct RecordedRider {
    pub position: Vec3,
    pub up: Vec3,
    pub velocity: Vec3,
    pub speed: f32,
    pub ground_normal: Vec3,
    pub lean: f32,
    pub airborne: bool,
    pub wipeout: bool,
    pub boosting: bool,
    pub grinding: bool,
    pub dead: bool,
}

impl RecordedRider {
    fn inputs(&self) -> Inputs {
        Inputs {
            target_pos: self.position,
            target_up: self.up,
            velocity: self.velocity,
            speed: self.speed,
            ground_normal: self.ground_normal,
            lean: self.lean,
            airborne: self.airborne,
            wipeout: self.wipeout,
            boosting: self.boosting,
            grinding: self.grinding,
            dead: self.dead,
            track_finished: false,
            aggregate_mass: 0.0,
            bailed_by_avalanche: false,
            alive: true,
            heli_position: Vec3::ZERO,
            heli_interact: false,
            heli_time: 0.0,
            back_press: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraView {
    pub position: Vec3,
    pub rotation: Quat,
    pub fov_deg: f32,
    pub near: f32,
}

pub struct ReplayCamera {
    chase: Chase,
}

impl ReplayCamera {
    pub fn new(collision: &CollisionWorld, level_start: Vec3, first: &RecordedRider, view: CameraView) -> Self {
        let ctx = StepContext { collision, steps: 0, exec: None, playback: None };
        let view =
            Snapshot { position: view.position, rotation: view.rotation, fov_deg: view.fov_deg, near: view.near };
        let chase = Chase::without_nis(ctx, level_start, CAMERA_TYPE_NORMAL, &first.inputs(), view, 0.0, None);
        Self { chase }
    }

    pub fn step(&mut self, collision: &CollisionWorld, rider: &RecordedRider, dt: f32) -> CameraView {
        let steps = self.chase.steps + 1;
        let ctx = StepContext { collision, steps, exec: None, playback: None };
        let s = self.chase.update(ctx, &rider.inputs(), dt, None);
        self.chase.steps = steps;
        CameraView { position: s.position, rotation: s.rotation, fov_deg: s.fov_deg, near: s.near }
    }

    pub fn sweeps(&self, collision: &CollisionWorld, camera: Option<Vec3>) -> Option<([Option<Hit>; 9], f32)> {
        let c = &self.chase;
        let ideal = c.queries.collided_all.then_some(c.meta.ideal_distance);
        let target = c.target?;
        camera.or(c.camera_pos).map(|cam| sweep_hits(collision, cam, target, c.shared.up, ideal))
    }

    pub fn target_up(&self) -> Vec3 {
        self.chase.shared.up
    }

    pub fn hinge(&self) -> Vec3 {
        self.chase.shared.hinge
    }

    pub fn transition(&self) -> (f32, String) {
        (self.chase.meta.transition_speed_current, format!("{:?}", self.chase.flow.current.node))
    }

    pub fn collision(&self) -> (bool, bool, f32) {
        let q = &self.chase.queries;
        (q.collided, q.collided_all, self.chase.ticks_since_collision)
    }
}

impl Default for CameraView {
    fn default() -> Self {
        Self { position: Vec3::ZERO, rotation: Quat::IDENTITY, fov_deg: GAME_REST_HFOV_DEG, near: GAME_NEAR }
    }
}
