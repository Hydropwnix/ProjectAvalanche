use bevy::math::{Quat, Vec3, Vec4};

mod cam15p;
mod cam3p;
mod flow;
mod kernel;

pub use cam3p::{REVERSE_DISTANCE, noise_layer_signals, step, step_death, step_reverse};
pub use cam15p::{CAMERA_15P_DISTANCE, Variant15, step_15p};
pub use flow::Flow;
pub use kernel::{
    Lowpass, SafeInputs, Spring, create_position_from_safe_values, decompose, frame_camera_x, lerp, look_at,
    position_offset_filter, range_change, range_change_clamp, right_axis, scale_distance, stateless_lowpass,
};

#[derive(Clone, Copy, Debug)]
pub struct Shared {
    pub up: Vec3,
    pub hinge: Vec3,
    pub force_hinge_reset: bool,
    pub hinge_reset: Vec3,
    pub handycam: bool,
    pub camera_pos: Vec3,
    pub fov_base: f32,
    pub fov_from_boost: f32,
    pub camera_is_boosting: bool,
    pub target_speed: f32,
    pub in_air: bool,
    pub fov: f32,
    pub y_offset: f32,
    pub player_skew: f32,
    pub lookahead_skew: f32,
    pub player_offset_y: f32,
    pub enable_meta_blending: bool,
    pub transition_triggered: bool,
    pub transition_speed_set: f32,
    pub big_air: bool,
    pub avalanche_intensity: f32,
    pub added_distance: f32,
    pub avalanche_shake: f32,
    pub carving_trigger: f32,
    pub carving_weight: f32,
    pub wipeout_trigger: f32,
    pub wipeout_weight: f32,
}

impl Shared {
    pub fn new(hinge: Vec3) -> Self {
        Self {
            up: Vec3::Y,
            hinge,
            force_hinge_reset: false,
            hinge_reset: hinge,
            handycam: true,
            camera_pos: Vec3::new(0.0, 0.0, 99_999.0),
            fov_base: 90.0,
            fov_from_boost: 0.0,
            camera_is_boosting: false,
            target_speed: 0.0,
            in_air: true,
            fov: 90.0,
            y_offset: 4.0,
            player_skew: 0.5,
            lookahead_skew: 0.5,
            player_offset_y: 0.8,
            enable_meta_blending: true,
            transition_triggered: false,
            transition_speed_set: 0.300_7,
            big_air: false,
            avalanche_intensity: 0.0,
            added_distance: 0.0,
            avalanche_shake: 0.0,
            carving_trigger: 1.0,
            carving_weight: 1.0,
            wipeout_trigger: 0.0,
            wipeout_weight: 1.0,
        }
    }

    pub fn update_skews(&mut self, lean: f32, dt: f32) {
        let target = (1.0 - lean) * 0.5;
        let keep = (-0.02 * dt).exp();
        self.player_skew = (self.player_skew - target) * keep + target;
        self.lookahead_skew = (self.lookahead_skew - target) * keep + target;
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Inputs {
    pub target_pos: Vec3,
    pub target_up: Vec3,
    pub velocity: Vec3,
    pub speed: f32,
    pub ground_normal: Vec3,
    pub lean: f32,
    pub airborne: bool,
    pub wipeout: bool,
    pub boosting: bool,
    pub grinding: bool,
    pub dead: bool,
    pub track_finished: bool,
    pub aggregate_mass: f32,
    pub bailed_by_avalanche: bool,
    pub alive: bool,
    pub heli_position: Vec3,
    pub heli_interact: bool,
    pub heli_time: f32,
    pub back_press: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Channels {
    pub position: Vec3,
    pub rotation: Quat,
    pub interest: Vec3,
    pub fov_deg: f32,
}

pub fn blend(new: &Channels, old: &Channels, w_old: f32) -> Channels {
    let q = if new.rotation.dot(old.rotation) > 0.0 { old.rotation } else { -old.rotation };
    Channels {
        position: new.position.lerp(old.position, w_old),
        rotation: Quat::from_vec4(Vec4::from(new.rotation).lerp(Vec4::from(q), w_old)).normalize(),
        interest: new.interest.lerp(old.interest, w_old),
        fov_deg: lerp(new.fov_deg, old.fov_deg, w_old),
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct GraphState {
    velocity: Lowpass<Vec3>,
    boost: Lowpass<f32>,
    roll: Spring,
    humanize: Lowpass<f32>,
    target_xz: Lowpass<Vec3>,
    target_y: Lowpass<Vec3>,
    target: Lowpass<Vec3>,
    camera: Lowpass<Vec3>,
    distance: Lowpass<f32>,
    av_mass: Lowpass<f32>,
    av_pitch: Lowpass<f32>,
    av_distance: Lowpass<f32>,
}

pub const DISTANCE: f32 = 6.0;
const DISTANCE_AIR: f32 = 6.9;
const DISTANCE_GRIND: f32 = 4.6;
const DISTANCE_PRESET: f32 = 6.4;
pub const DISTANCE_FAR: f32 = 9.9;
pub const HINGE_DISTANCE: f32 = 13.0;
const HINGE_ANGLE: f32 = 0.7;
const LOOKAHEAD: f32 = 75.0;
const LOOKAHEAD_TICKS: f32 = 30.0;
const PITCH_CLAMP: f32 = 1.197;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    Ground,
    Air,
    Grind,
    FarGround,
    FarAir,
}

fn hinge_or_reset(s: &mut Shared) -> Vec3 {
    if std::mem::take(&mut s.force_hinge_reset) { s.hinge_reset } else { s.hinge }
}

pub const CAMERA_FINISHED_OFFSET: f32 = 0.8;

#[cfg(test)]
mod tests;
