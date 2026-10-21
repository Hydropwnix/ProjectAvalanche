use glam::{Quat, Vec3};

pub const TINY: f32 = 1.525_878_9e-5;
pub const EPS: f32 = 1.192_092_9e-7;
const GOLDEN: f32 = 0.381_966;
const GOLDEN_STOP: f32 = 0.003_906_25;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DragModel {
    pub wind: Vec3,
    pub kh: f32,
    pub kv: f32,
    pub g: f32,
}

impl DragModel {
    fn s(&self) -> f32 {
        (self.kv * self.g).sqrt()
    }

    pub fn least_vy(&self, dy: f32) -> f32 {
        (self.kv * self.g * ((2.0 * dy * self.kv).exp() - 1.0)).sqrt() / self.kv
    }
}

fn ln_c(x: f32) -> f32 {
    x.max(TINY).ln()
}

fn acosh_c(x: f32) -> f32 {
    let x = x.max(1.0);
    (x + (x * x - 1.0).sqrt()).ln()
}

fn atanh_c(x: f32) -> f32 {
    let c = x.clamp(-1.0 + TINY, 1.0 - TINY);
    0.5 * ((1.0 + c) / (1.0 - c)).ln()
}

pub fn launch_for(m: &DragModel, v_air: Vec3, vy0: f32, delta: Vec3) -> (Vec3, f32) {
    // TODO(guess): libm's exp / ln / atan / acos for the game's approximations (0x82345890, 0x82D05438, `XMVectorATan` 0x828F5388, `XMVectorACos` 0x8249EF98; 6·10⁻⁴ off, 5·10⁻³ after the golden section, `intercept_check`); settle: port the four approximations.
    crate::guess_hit!();
    let (kv, g, s) = (m.kv, m.g, m.s());
    let dy = delta.y;
    let vy = if dy > 0.0 { vy0.max(m.least_vy(dy)) } else { vy0 };
    let mut times = [0.0f32; 2];
    let mut n = 0;
    let (v_down, t_off, dy_left) = if vy > 0.0 {
        let t_up = (vy * s / g).atan() / s;
        let h = ln_c(1.0 + kv * vy * vy / g) / (2.0 * kv);
        if dy > 0.0 {
            let c = (kv * (dy - h)).exp().clamp(-1.0, 1.0);
            times[0] = t_up - c.acos() / s;
            n = 1;
        }
        (0.0, t_up, dy - h)
    } else {
        (vy, 0.0, dy)
    };
    let d0 = -ln_c(1.0 - kv * v_down * v_down / g) / (2.0 * kv);
    times[n] = t_off + atanh_c(v_down * s / g) / s + acosh_c((kv * (d0 - dy_left)).exp()) / s;
    let dh = Vec3::new(delta.x, 0.0, delta.z);
    let l = dh.length();
    let dir = if l > 0.0 { dh / l } else { Vec3::ZERO };
    let across = |t: f32| {
        let mut v = dir * (((m.kh * l).exp() - 1.0) / (m.kh * t));
        v.y = vy;
        v
    };
    let (mut v, mut t) = (across(times[0]), times[0]);
    if n == 1 {
        let c = across(times[1]);
        if (v - v_air).length_squared() > (c - v_air).length_squared() {
            (v, t) = (c, times[1]);
        }
    }
    (v, t)
}

pub fn solve_launch(m: &DragModel, v_air: Vec3, delta: Vec3) -> (Vec3, f32) {
    let lo = if delta.y > 0.0 { m.least_vy(delta.y) } else { v_air.y.min(0.0) };
    let hi = v_air.y + (launch_for(m, v_air, lo, delta).0 - v_air).length();
    let f = |vy: f32| (launch_for(m, v_air, vy, delta).0 - v_air).length_squared();
    let vy = if lo + TINY >= hi {
        lo
    } else {
        let (mut a, mut b) = (lo, hi);
        let mut x1 = a + GOLDEN * (b - a);
        let mut f1 = f(x1);
        for _ in 0..32 {
            let x2 = x1 + GOLDEN * (b - x1);
            let f2 = f(x2);
            if GOLDEN_STOP * (x1.abs() + x2.abs()) > (b - a).abs() {
                break;
            }
            if f1 > f2 {
                (a, x1, f1) = (x1, x2, f2);
            } else {
                (b, a) = (a, x2);
            }
        }
        0.5 * (b + a)
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
    if v_req.length_squared() >= 100.0 {
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
