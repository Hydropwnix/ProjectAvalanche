use bevy::prelude::*;

use crate::camera::graph::Inputs;

#[derive(Resource, Default, Clone, Debug)]
pub struct SpectateTarget {
    pub rider: Option<u16>,
    pub inputs: Option<Inputs>,
}

#[cfg(feature = "scripts")]
pub fn spectate_inputs_at(position: Vec3, velocity: Vec3, in_air: bool, back_press: bool) -> Inputs {
    Inputs {
        target_pos: position,
        target_up: Vec3::Y,
        velocity,
        // TODO(guess): a bot's or ghost's target up, ground normal and lean are the camera's defaults (up, up, 0) and
        // its speed the velocity's (m per tick × 60): a bot's step keeps only its position, velocity and air flag
        // (`Bot` in ssx_physics::session::bots), a ghost's replay the same from its pose block; settle: the pose
        // block's trajectory rotation and the 3P graph's target fields (`QueryTask_Cam_GetTarget` 0x82EB4E58).
        speed: velocity.length() * 60.0,
        ground_normal: Vec3::Y,
        lean: 0.0,
        airborne: in_air,
        wipeout: false,
        boosting: false,
        grinding: false,
        dead: false,
        track_finished: false,
        aggregate_mass: 0.0,
        bailed_by_avalanche: false,
        alive: true,
        heli_position: Vec3::ZERO,
        heli_interact: false,
        heli_time: 0.0,
        back_press,
    }
}
