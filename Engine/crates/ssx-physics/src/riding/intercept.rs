use glam::{Quat, Vec3};
use ssx_formats::vmx::{self, abs, add, dot3, eq, floor, gt, madd, max, min, mul, negate, recip, refp, rsqrt, sub};

pub const TINY: f32 = 1.525_878_9e-5;
pub const EPS: f32 = 1.192_092_9e-7;
const GOLDEN: f32 = f32::from_bits(0x3EC3_910D);
const GOLDEN_STOP: f32 = f32::from_bits(0x3B80_0000);
pub(crate) const LOG2_E: f32 = f32::from_bits(0x3FB8_AA3B);
const LN_2: f32 = f32::from_bits(0x3F31_7218);
const EXP2_C: [u32; 4] = [0x3F80_0000, 0xBF31_7218, 0x3E75_FDED, 0xBD63_57CA];
const EXP2_D: [u32; 4] = [0x3C1D_8C54, 0xBAAE_1854, 0x391A_A7D7, 0xB736_4261];
const LN_C: [u32; 4] = [0x3FB8_AA0E, 0xBF38_9E52, 0x3EF5_162D, 0xBEB1_D204];
const LN_D: [u32; 4] = [0x3E77_ADBD, 0xBE0C_D4FB, 0x3D55_41C6, 0xBC18_8B0B];
const PI: f32 = f32::from_bits(0x4049_0FDB);
const HALF_PI: f32 = f32::from_bits(0x3FC9_0FDB);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DragModel {
    pub wind: Vec3,
    pub kh: f32,
    pub kv: f32,
    pub g: f32,
}

impl DragModel {
    fn s(&self) -> f32 {
        sqrt(mul(self.kv, self.g))
    }

    pub fn least_vy(&self, dy: f32) -> f32 {
        let e = exp2(mul(mul(mul(2.0, dy), self.kv), LOG2_E));
        mul(recip(self.kv), sqrt(mul(mul(self.kv, self.g), sub(e, 1.0))))
    }
}

pub(crate) fn sqrt(x: f32) -> f32 {
    if eq(0.0, x) { 0.0 } else { mul(x, rsqrt(x)) }
}

fn halves(x: f32, c: [u32; 4], d: [u32; 4]) -> f32 {
    let [c0, c1, c2, c3] = c.map(f32::from_bits);
    let [d0, d1, d2, d3] = d.map(f32::from_bits);
    let x2 = mul(x, x);
    let a = madd(x, c1, c0);
    let b = madd(x, d1, d0);
    let x3 = mul(x, x2);
    let a = madd(x2, c2, a);
    let b = madd(x2, d2, b);
    let x4 = mul(x2, x2);
    let a = madd(x3, c3, a);
    let b = madd(x3, d3, b);
    madd(x4, b, a)
}

pub(crate) fn exp2(x: f32) -> f32 {
    let fl = floor(x);
    let p = halves(sub(x, fl), EXP2_C, EXP2_D);
    mul(vmx::expte(fl), vmx::recip_guarded(p))
}

fn ln_tail(x: f32) -> f32 {
    let m = f32::from_bits((x.to_bits() & 0x007F_FFFF) | 0x3F80_0000);
    let t = sub(m, 1.0);
    mul(madd(t, halves(t, LN_C, LN_D), floor(vmx::loge(x))), LN_2)
}

fn ln_c(x: f32) -> f32 {
    ln_tail(max(x, TINY))
}

fn acosh_c(x: f32) -> f32 {
    let x = max(x, 1.0);
    ln_tail(add(x, sqrt(sub(mul(x, x), 1.0))))
}

fn atanh_c(x: f32) -> f32 {
    let c = min(sub(1.0, TINY), max(add(-1.0, TINY), x));
    mul(0.5, ln_tail(mul(recip(sub(1.0, c)), add(1.0, c))))
}

fn atan2_once(y: f32, x: f32) -> f32 {
    let r0 = refp(x);
    let a = vmx::atan(madd(y, madd(r0, vmx::nmsub(r0, x, 1.0), r0), 0.0));
    let sign = y.to_bits() & 0x8000_0000;
    if eq(0.0, x) {
        f32::from_bits(HALF_PI.to_bits() | sign)
    } else if gt(0.0, x) {
        add(f32::from_bits(PI.to_bits() | sign), a)
    } else {
        a
    }
}

pub fn launch_for(m: &DragModel, v_air: Vec3, vy0: f32, delta: Vec3) -> (Vec3, f32) {
    let (kh, kv, g, s) = (m.kh, m.kv, m.g, m.s());
    let dy = delta.y;
    let vy = if gt(dy, 0.0) { max(vy0, m.least_vy(dy)) } else { vy0 };
    let two_kv = mul(2.0, kv);
    let mut times = [0.0f32; 2];
    let mut rising = false;
    let (v_down, t_off, dy_left) = if gt(vy, 0.0) {
        let t_up = mul(recip(s), atan2_once(mul(vy, s), g));
        let h = mul(recip(two_kv), ln_c(madd(recip(g), mul(mul(kv, vy), vy), 1.0)));
        if gt(dy, 0.0) {
            let c = min(1.0, max(-1.0, exp2(mul(mul(kv, sub(dy, h)), LOG2_E))));
            times[0] = sub(t_up, mul(recip(s), vmx::acos(c)));
            rising = true;
        }
        (0.0, t_up, sub(dy, h))
    } else {
        (vy, 0.0, dy)
    };
    let a = atanh_c(mul(recip(g), mul(v_down, s)));
    let d0 = mul(recip(two_kv), negate(ln_c(sub(1.0, mul(recip(g), mul(mul(kv, v_down), v_down))))));
    let c = acosh_c(exp2(mul(mul(kv, sub(d0, dy_left)), LOG2_E)));
    times[usize::from(rising)] = madd(recip(s), c, madd(recip(s), a, t_off));
    let dh = [delta.x, 0.0, delta.z];
    let l2 = dot3(dh, dh);
    let q = rsqrt(l2);
    let l = if eq(l2, 0.0) { 0.0 } else { mul(l2, q) };
    let (l, dir) = if gt(l, 0.0) { (l, [mul(delta.x, q), mul(delta.z, q)]) } else { (0.0, [0.0, 0.0]) };
    let e = exp2(mul(mul(kh, l), LOG2_E));
    let across = |t: f32| {
        let f = mul(recip(mul(kh, t)), sub(e, 1.0));
        Vec3::new(mul(dir[0], f), vy, mul(dir[1], f))
    };
    let off = |v: Vec3| {
        let d = [sub(v.x, v_air.x), sub(v.y, v_air.y), sub(v.z, v_air.z)];
        dot3(d, d)
    };
    let (mut v, mut t) = (across(times[0]), times[0]);
    if rising {
        let c = across(times[1]);
        if gt(off(v), off(c)) {
            (v, t) = (c, times[1]);
        }
    }
    (v, t)
}

pub fn solve_launch(m: &DragModel, v_air: Vec3, delta: Vec3) -> (Vec3, f32) {
    let lo = if gt(delta.y, 0.0) { m.least_vy(delta.y) } else { min(0.0, v_air.y) };
    let f = |vy: f32| {
        let v = launch_for(m, v_air, vy, delta).0;
        let d = [sub(v.x, v_air.x), sub(v.y, v_air.y), sub(v.z, v_air.z)];
        dot3(d, d)
    };
    let hi = add(v_air.y, sqrt(f(lo)));
    let vy = if vmx::ge(add(lo, TINY), hi) {
        lo
    } else {
        let (mut a, mut b) = (lo, hi);
        let mut x1 = madd(GOLDEN, sub(hi, lo), lo);
        let mut f1 = f(x1);
        for _ in 0..32 {
            let x2 = madd(GOLDEN, sub(b, x1), x1);
            let f2 = f(x2);
            if gt(mul(GOLDEN_STOP, add(abs(x1), abs(x2))), abs(sub(b, a))) {
                break;
            }
            if gt(f1, f2) {
                (a, x1, f1) = (x1, x2, f2);
            } else {
                (b, a) = (a, x2);
            }
        }
        mul(0.5, add(b, a))
    };
    launch_for(m, v_air, vy, delta)
}

pub fn simulate(m: &DragModel, v0: Vec3, t_total: f32, dt: f32) -> Vec3 {
    let (mut disp, mut v, mut t) = (Vec3::ZERO, v0, t_total);
    while t > TINY {
        let mut a = crate::rider::drag_accel(m.kh, m.kv, v, m.wind, dt);
        a.y -= m.g;
        t -= dt;
        v += a * dt;
        disp += v * dt;
    }
    disp
}

pub fn refine(m: &DragModel, mut v: Vec3, delta: Vec3, dt: f32, t: f32) -> Vec3 {
    let close = |p: Vec3| (p - delta).abs().max_element() <= 0.01;
    let mut p = simulate(m, v, t, dt);
    if close(p) {
        return v;
    }
    for _ in 0..16 {
        let col = |k: usize| {
            let mut vk = v;
            vk[k] += TINY;
            (simulate(m, vk, t, dt) - p) * (1.0 / TINY)
        };
        let (a, b, c) = (col(0), col(1), col(2));
        let det = a.dot(b.cross(c));
        let (r0, r1, r2) = (b.cross(c) / det, c.cross(a) / det, a.cross(b) / det);
        if ![r0, r1, r2].iter().all(|r| r.is_finite() && r.abs().max_element() <= f32::MAX) {
            break;
        }
        let e = p - delta;
        v -= Vec3::new(r0.dot(e), r1.dot(e), r2.dot(e));
        p = simulate(m, v, t, dt);
        if close(p) {
            break;
        }
    }
    v
}

pub fn rotation_arc(from: Vec3, to: Vec3) -> Quat {
    let d = from.dot(to);
    if (d - 1.0).abs() <= TINY {
        return Quat::IDENTITY;
    }
    if (d + 1.0).abs() <= TINY {
        let a = from.abs();
        let e = if a.y.min(a.z) > a.x {
            Vec3::X
        } else if a.x.min(a.z) > a.y {
            Vec3::Y
        } else {
            Vec3::Z
        };
        let axis = from.cross(e).normalize();
        return Quat::from_xyzw(axis.x, axis.y, axis.z, 0.0);
    }
    let xyz = from.cross(to) / (2.0 * (1.0 + d)).sqrt();
    Quat::from_xyzw(xyz.x, xyz.y, xyz.z, (0.5 * (1.0 + d)).sqrt()).normalize()
}

pub fn clamp_angle(q: Quat, limit: f32) -> Quat {
    let w = q.w;
    if w.abs() > 1.0 - TINY {
        return q;
    }
    let turns = 2.0 * w.clamp(-1.0, 1.0).acos() / std::f32::consts::TAU;
    let mut frac = turns - turns.floor();
    if frac > 0.5 {
        frac -= 1.0;
    }
    let angle = frac * std::f32::consts::TAU;
    let c = angle.max(-limit).min(limit);
    if (angle - c).abs() <= TINY {
        return q;
    }
    let s2 = 1.0 - w * w;
    let axis = if s2 >= TINY { Vec3::new(q.x, q.y, q.z) / s2.sqrt() } else { Vec3::X };
    let (sin, cos) = (0.5 * c).sin_cos();
    Quat::from_xyzw(axis.x * sin, axis.y * sin, axis.z * sin, cos).normalize()
}

#[allow(clippy::too_many_arguments)]
pub fn takeoff(
    v_air: Vec3,
    launch: Vec3,
    traj: Vec3,
    position: Vec3,
    angle: f32,
    length_scale: f32,
    m: &DragModel,
    dt: f32,
) -> Option<(Vec3, f32)> {
    if launch.abs().max_element() <= EPS {
        return None;
    }
    let delta = position - traj;
    let (v_req, t) = solve_launch(m, v_air, delta);
    if !gt(100.0, dot3(v_req.to_array(), v_req.to_array())) {
        return None;
    }
    let target = refine(m, v_req, delta, dt, t);
    let (la, lt) = (v_air.length(), target.length());
    let dir_air = if la > 0.0 { v_air / la } else { Vec3::ZERO };
    let dir_target = if lt > 0.0 { target / lt } else { Vec3::ZERO };
    let q = clamp_angle(rotation_arc(dir_air, dir_target), angle);
    if la <= EPS {
        return Some((dir_air, t));
    }
    let ratio = (lt / la).max(1.0 / (1.0 + length_scale)).min(1.0 + length_scale);
    Some(((q * dir_air) * (la * ratio), t))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> DragModel {
        DragModel { wind: Vec3::ZERO, kh: 0.0008, kv: 0.0016, g: 9.81 / 3600.0 }
    }

    #[test]
    fn the_refined_launch_lands_on_the_target() {
        let m = model();
        let v_air = Vec3::new(0.0, 0.05, 0.55);
        let delta = Vec3::new(1.5, 8.8, 26.0);
        let (v, t) = solve_launch(&m, v_air, delta);
        assert!(t > 0.0 && v.y >= m.least_vy(delta.y) - 1e-4, "{v} {t}");
        let r = refine(&m, v, delta, 2.0, t);
        assert!((simulate(&m, r, t, 2.0) - delta).abs().max_element() <= 0.01, "{r}");
    }

    #[test]
    fn the_arc_and_its_limit() {
        let (a, b) = (Vec3::new(0.0, 0.0, 1.0), Vec3::new(1.0, 0.0, 0.0));
        let q = rotation_arc(a, b);
        assert!((q * a - b).length() < 1e-5);
        let c = clamp_angle(q, 0.5);
        assert!(((q * a).angle_between(a) - std::f32::consts::FRAC_PI_2).abs() < 1e-4);
        assert!(((c * a).angle_between(a) - 0.5).abs() < 1e-4);
        assert_eq!(clamp_angle(q, crate::session::heli_jump::INTERCEPT_ANGLE), q);
    }
}
