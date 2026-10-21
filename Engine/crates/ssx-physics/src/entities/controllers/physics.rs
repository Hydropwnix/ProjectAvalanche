use glam::Vec3;

pub const PUSH_MIN_SPEED: f32 = 1.0;
pub const PUSH_VELOCITY_WEIGHT: f32 = 4.0;
pub const PUSH_LIFT: f32 = 0.25;
pub const PUSH_ANGULAR: f32 = 25.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Push {
    pub time: f32,
    pub linear: Vec3,
    pub angular: Vec3,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PhysicsComponent {
    pub frozen: bool,
    pub pushes: Vec<Push>,
}

impl Default for PhysicsComponent {
    fn default() -> Self {
        Self { frozen: true, pushes: Vec::new() }
    }
}

fn direction_or_zero(x: Vec3) -> Vec3 {
    let d2 = x.length_squared();
    if d2 == 0.0 { Vec3::ZERO } else { x / d2.sqrt() }
}

pub fn push_away(owner: Vec3, player: Vec3, velocity: Vec3, speed: f32, strength: f32, time: f32) -> Option<Push> {
    if speed <= PUSH_MIN_SPEED {
        return None;
    }
    let d = direction_or_zero((owner - player).with_y(0.0));
    let v = direction_or_zero(velocity);
    let mut dir = direction_or_zero(v * PUSH_VELOCITY_WEIGHT + d);
    dir.y += PUSH_LIFT;
    let linear = dir * (strength * speed);
    let angular = linear / linear.length() * PUSH_ANGULAR;
    Some(Push { time, linear, angular })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_weights_the_velocity_four_times_and_lifts() {
        let p = push_away(Vec3::new(0.0, 0.0, 1.0), Vec3::ZERO, Vec3::X * 0.3, 20.0, 1.0, 5.0).unwrap();
        let dir = (Vec3::X * 4.0 + Vec3::Z).normalize() + Vec3::Y * 0.25;
        assert!((p.linear - dir * 20.0).length() < 1e-4);
        assert!((p.angular.length() - 25.0).abs() < 1e-4);
        assert_eq!(p.time, 5.0);
    }

    #[test]
    fn no_push_at_walking_speed() {
        assert!(push_away(Vec3::Z, Vec3::ZERO, Vec3::X, 1.0, 1.0, 0.0).is_none());
    }

    #[test]
    fn a_standing_player_on_top_pushes_straight_up() {
        let p = push_away(Vec3::Y, Vec3::ZERO, Vec3::ZERO, 2.0, 1.0, 0.0).unwrap();
        assert!((p.linear - Vec3::Y * 0.5).length() < 1e-6);
        assert!((p.angular - Vec3::Y * 25.0).length() < 1e-4);
    }
}
