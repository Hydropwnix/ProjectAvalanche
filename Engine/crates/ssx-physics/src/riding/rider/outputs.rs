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
    use ssx_formats::vmx;
    let unit = |p: Vec3| {
        let d = vmx::dot3(p.to_array(), p.to_array());
        let y = vmx::rsqrte(d);
        let y = vmx::madd(vmx::mul(y, 0.5), vmx::nmsub(d, vmx::mul(y, y), 1.0), y);
        (p * y).to_array()
    };
    let c = vmx::min(vmx::max(vmx::dot3(unit(bp), unit(vp)), -1.0), 1.0);
    let a = vmx::acos(c);
    if a > std::f32::consts::FRAC_PI_2 { std::f32::consts::PI - a } else { a }
}

pub fn carving_reach_value(pitch: f32, bail_on_landing: bool, uber: bool) -> f32 {
    let a = crate::steering::wrap_pi_f64(pitch).abs();
    let reach = if a <= std::f32::consts::FRAC_PI_2 { 1.0 } else { ssx_formats::vmx::sin(a) };
    if bail_on_landing || uber { 0.0 } else { reach }
}

pub fn landing_spin_value(x: f32) -> f32 {
    const TWO_52: f64 = 4_503_599_627_370_496.0;
    let y = x * f32::from_bits(0x3DA2_F983);
    let yd = f64::from(y);
    let m = if yd >= 0.0 { -TWO_52 } else { TWO_52 };
    let r = (yd - m) + m;
    let floor = r - if yd - r >= 0.0 { 0.0 } else { 1.0 };
    let f = y - floor as f32;
    let four_pi = f32::from_bits(0x4149_0FDB);
    if f > 0.5 { (f - 1.0) * four_pi } else { f * four_pi }
}

pub fn landing_spin_value_normalized(v: f32) -> f32 {
    let (inv_two_pi, two_pi) = (f64::from_bits(0x3FC4_5F30_6446_F9B4), f64::from_bits(0x4019_21FB_6000_0000));
    let y = f64::from(v) * inv_two_pi;
    let k = y.trunc();
    let f = if (y - 0.5) - k >= 0.0 { (y - 1.0) - k } else { y - k };
    let wrap = if k - (y + 0.5) >= 0.0 { two_pi } else { 0.0 };
    f.mul_add(two_pi, wrap) as f32
}

pub fn nose_tail_factor(n: Vec3, q_traj: Quat, goofy: bool) -> f32 {
    let u = rotate_fused(q_traj, Vec3::Y);
    let x = (n - u * n.dot(u)).x;
    if goofy { x } else { -x }
}

fn rotate_fused(q: Quat, v: Vec3) -> Vec3 {
    let a = Vec3::new(q.x, q.y, q.z);
    let cross = |a: Vec3, b: Vec3| {
        let t =
            Vec3::new((-a.y).mul_add(b.x, a.x * b.y), (-a.z).mul_add(b.y, a.y * b.z), (-a.x).mul_add(b.z, a.z * b.x));
        Vec3::new(t.y, t.z, t.x)
    };
    let c = cross(a, v);
    let t = Vec3::new(q.w.mul_add(v.x, c.x), q.w.mul_add(v.y, c.y), q.w.mul_add(v.z, c.z));
    let c2 = cross(a, t);
    Vec3::new(2.0f32.mul_add(c2.x, v.x), 2.0f32.mul_add(c2.y, v.y), 2.0f32.mul_add(c2.z, v.z))
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
    fn the_fused_rotation_is_a_rotation() {
        let q = Quat::from_euler(glam::EulerRot::YXZ, 0.7, -0.3, 0.2);
        for v in [Vec3::Y, Vec3::X, Vec3::new(0.3, -0.5, 0.8)] {
            assert!(rotate_fused(q, v).distance(q * v) < 1e-6);
        }
    }

    #[test]
    fn board_angle_is_folded() {
        let n = Vec3::Y;
        assert!(angle_between_board_and_velocity(Vec3::Z, Vec3::new(0.0, -0.3, 2.0), n).abs() < 1e-3);
        let a = angle_between_board_and_velocity(Vec3::Z, Vec3::new(1.0, 0.0, -1.0), n);
        assert!((a - std::f32::consts::FRAC_PI_4).abs() < 3e-5);
        assert!(angle_between_board_and_velocity(Vec3::Z, Vec3::ZERO, n).is_nan(), "the game's NaN");
        assert!(angle_between_board_and_velocity(Vec3::Y, Vec3::Z, n).is_nan());
    }

    #[test]
    fn nose_tail_is_the_normals_tilt_off_the_up_axis() {
        let n = Vec3::new(0.6, 0.8, 0.0);
        assert!((nose_tail_factor(n, Quat::IDENTITY, true) - 0.6).abs() < 1e-6);
        assert!((nose_tail_factor(n, Quat::IDENTITY, false) + 0.6).abs() < 1e-6);
    }

    #[test]
    fn landing_spin_values_match_the_executable() {
        let cases: [[u32; 3]; 20] = [
            [0x0000_0000, 0x0000_0000, 0x0000_0000],
            [0x8000_0000, 0x8000_0000, 0x0000_0000],
            [0x3f80_0000, 0x3f80_0000, 0x3f80_0000],
            [0xbf80_0000, 0xbf7f_fffb, 0xbf7f_fffb],
            [0x4049_0fdb, 0x4049_0fdb, 0x4049_0fdb],
            [0xc049_0fdb, 0xc049_0fdb, 0xc049_0fdb],
            [0x40c9_0fdb, 0x40c9_0fdb, 0xa649_0fdb],
            [0xc0c9_0fdb, 0x40c9_0fdb, 0xa649_0fdb],
            [0x4149_0fdb, 0x0000_0000, 0x0000_0000],
            [0x4116_cbe4, 0xc049_0fdb, 0xc049_0fdb],
            [0xc116_cbe4, 0x4049_0fdb, 0x4049_0fdb],
            [0x7fc0_0000, 0x7fc0_0000, 0x7fc0_0000],
            [0x42c8_0000, 0xbf07_ed76, 0xbf07_ed76],
            [0xc47a_0000, 0x40a9_e8ec, 0xbf79_3778],
            [0x4e6e_6b28, 0x0000_0000, 0x0000_0000],
            [0x3456_bf95, 0x3456_bf96, 0x3456_bf96],
            [0xc19a_3c0c, 0x40bb_4f3c, 0xbedc_09f0],
            [0x416d_2157, 0x4010_45f0, 0x4010_45f0],
            [0x4024_eff9, 0x4024_eff9, 0x4024_eff9],
            [0x41df_933d, 0x4034_1b0c, 0x4034_1b0c],
        ];
        for c in cases {
            let [x, value, normalized] = c.map(f32::from_bits);
            let same = |g: f32, w: f32| g.to_bits() == w.to_bits() || (g.is_nan() && w.is_nan());
            assert!(same(landing_spin_value(x), value), "{c:x?}");
            assert!(same(landing_spin_value_normalized(value), normalized), "{c:x?}");
        }
    }
}
