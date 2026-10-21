use glam::{Quat, Vec3};

pub fn goofy_update(a: f32, goofy: bool, h: f32) -> bool {
    let near_axis = a <= h && a >= -h;
    let far = std::f32::consts::PI - h;
    let inside = a <= far && a >= -far;
    if near_axis || !inside { goofy } else { a > 0.0 }
}

pub fn angle_between_board_and_velocity(board_z: Vec3, v: Vec3, n: Vec3) -> f32 {
    let vp = v - n * v.dot(n);
    let bp = board_z - n * board_z.dot(n);
    // TODO(guess): no other reader of the NaN (anim_all.cba names the value once more, a `DATAb` asset at +0x1D9614); settle: that asset's reader.
    if vp.length_squared() <= 0.0 || bp.length_squared() <= 0.0 {
        crate::guess_hit!();
        return 0.0;
    }
    let a = vp.normalize().dot(bp.normalize()).clamp(-1.0, 1.0).acos();
    if a > std::f32::consts::FRAC_PI_2 { std::f32::consts::PI - a } else { a }
}

pub fn nose_tail_factor(n: Vec3, q_traj: Quat, goofy: bool) -> f32 {
    let u = q_traj * Vec3::Y;
    let x = (n - u * n.dot(u)).x;
    if goofy { x } else { -x }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::{FRAC_PI_2, PI};

    #[test]
    fn goofy_follows_the_side_of_the_velocity() {
        assert!(goofy_update(FRAC_PI_2, false, 0.0));
        assert!(!goofy_update(-FRAC_PI_2, true, 0.0));
        assert!(goofy_update(0.0, true, 0.0) && goofy_update(PI, false, 0.0));
        assert!(goofy_update(0.1, true, 0.2) && !goofy_update(PI - 0.1, false, 0.2));
    }

    #[test]
    fn board_angle_is_folded() {
        let n = Vec3::Y;
        assert!(angle_between_board_and_velocity(Vec3::Z, Vec3::new(0.0, -0.3, 2.0), n).abs() < 1e-6);
        let a = angle_between_board_and_velocity(Vec3::Z, Vec3::new(1.0, 0.0, -1.0), n);
        assert!((a - std::f32::consts::FRAC_PI_4).abs() < 1e-5);
        assert_eq!(angle_between_board_and_velocity(Vec3::Z, Vec3::ZERO, n), 0.0);
    }

    #[test]
    fn nose_tail_is_the_normals_tilt_off_the_up_axis() {
        let n = Vec3::new(0.6, 0.8, 0.0);
        assert!((nose_tail_factor(n, Quat::IDENTITY, true) - 0.6).abs() < 1e-6);
        assert!((nose_tail_factor(n, Quat::IDENTITY, false) + 0.6).abs() < 1e-6);
    }
}
