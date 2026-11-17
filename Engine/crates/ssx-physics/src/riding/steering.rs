use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI};

use glam::{Quat, Vec2, Vec3};

use ssx_formats::vmx;

use crate::character::{CarvingTuning, PidGains};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pid {
    pub gains: PidGains,
    pub gain: f32,
    pub x: f32,
    pub rate: f32,
    x_prev: f32,
    integral: f32,
}

impl Pid {
    pub fn new(gains: PidGains, gain: f32) -> Self {
        Self { gains, gain, ..Self::default() }
    }

    pub fn update(&mut self, dt: f32, s: f32) {
        let mut rem = dt;
        while rem > 1.525_878_9e-5 {
            let h = if rem - 1.0 >= 0.0 { 1.0 } else { rem };
            rem -= h;
            let u = self.compute(h, s);
            let x_old = self.x;
            self.x = (self.gain * u).mul_add(h, self.rate).mul_add(h, self.x).clamp(-1.0, 1.0);
            self.rate = (self.x - x_old) / h;
        }
    }

    pub fn set_driven(&mut self, x: f32) {
        self.x = x.clamp(-1.0, 1.0);
        self.rate = 0.0;
        self.integral = 0.0;
        self.x_prev = self.x;
    }

    pub fn mirror(&mut self) {
        self.x = (-self.x).clamp(-1.0, 1.0);
        self.rate = -self.rate;
        self.integral = -self.integral;
        self.x_prev = -self.x_prev;
    }

    fn compute(&mut self, h: f32, s: f32) -> f32 {
        const EPS: f32 = 1.192_092_9e-7;
        let x = self.x;
        let e = s - x;
        let d = (self.x_prev - x) / h;
        self.x_prev = x;
        self.integral = e.mul_add(h, self.integral);
        let g = self.gains;
        let u = g.i.mul_add(self.integral, g.d.mul_add(d, g.p * e));
        let sign = if u >= 0.0 { 1.0 } else { -1.0 };
        let out = if u.abs() <= EPS { 0.0 } else { sign * u.abs().min(1.0) };
        if g.i.abs() > EPS {
            let (lo, hi) = (self.integral.min(0.0), self.integral.max(0.0));
            self.integral -= ((u - out) / g.i).max(lo).min(hi);
        }
        out
    }
}

pub fn wrap_pi(a: f32) -> f32 {
    let w = (a + PI).rem_euclid(2.0 * PI) - PI;
    if w <= -PI { w + 2.0 * PI } else { w }
}

pub fn wrap_pi_f64(a: f32) -> f32 {
    const INV_TWO_PI: f64 = 0.159_154_938_663_005_67;
    const TWO_PI: f64 = 6.283_185_482_025_146_5;
    let t = f64::from(a) * INV_TWO_PI;
    let i = if t.is_nan() { -2_147_483_648.0 } else { f64::from(t as i32) };
    let frac = if (t - 0.5) - i >= 0.0 { (t - 1.0) - i } else { t - i };
    let add = if i - (t + 0.5) >= 0.0 { TWO_PI } else { 0.0 };
    frac.mul_add(TWO_PI, add) as f32
}

pub fn vmx_wrap_pi(a: f32) -> f32 {
    let v = a * 0.159_154_94;
    let f = v - v.floor();
    let f = if f > 0.5 { f - 1.0 } else { f };
    f * 6.283_185_5
}

pub fn stick_angle_and_magnitude(stick: Vec2) -> (f32, f32) {
    (-wrap_pi_f64(stick.y.atan2(stick.x) - FRAC_PI_2), stick.length().min(1.0))
}

pub fn lean_input(c: &CarvingTuning, angle: f32, mag: f32, max_in: f32, ice: f32, upright: bool) -> f32 {
    let n = c.lean_input_angle_transform.eval(angle);
    let mut t = (c.ice_axe.lean_input_transform.eval(angle) - n).mul_add(ice, n);
    if t.abs() > FRAC_PI_2 && upright {
        t = fsel_sign(t) * FRAC_PI_2;
    }
    let sin = |x: f32| f64::from(x).sin() as f32;
    let (s, hi, lo) = (sin(t), sin(max_in), sin(-max_in));
    mag * s.max(lo).min(hi)
}

pub fn early_slip(c: &CarvingTuning, angle: f32, mag: f32, max_in: f32) -> (f32, f32, f32) {
    let a = angle.abs();
    let limit = c.early_slip_input_limit;
    let max_angle = c.early_slip_max_angle;
    if a < limit {
        let t = ((a - max_in) / (limit - max_in)).clamp(0.0, 1.0);
        (1.0, t * mag, (-(t * max_angle).sin() * mag).clamp(-1.0, 0.0))
    } else {
        let t2 = ((a - limit) / (PI - limit)).clamp(0.0, 1.0);
        (0.0, 0.0, (-(max_angle + t2 * (FRAC_PI_2 - max_angle)).sin() * mag).clamp(-1.0, 0.0))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Kinematics {
    pub lean: f32,
    pub slip: f32,
    pub pitch: f32,
    pub fwd: f32,
    pub lean_norm: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SteeringState {
    pub lean_pid: Pid,
    pub spin_pid: Pid,
    pub pitch_pid: Pid,
    pub forward_sign: f32,
    pub lean_norm: f32,
    pub slip_edge: f32,
    pub slip_angle: f32,
    pub slip_dig_angle: f32,
    pub stance_override_sign: f32,
    pub concavity: f32,
    pub normal_force: f32,
}

impl SteeringState {
    pub fn new(c: &CarvingTuning) -> Self {
        Self {
            lean_pid: Pid::new(c.lean_pid, c.max_lean_accel),
            spin_pid: Pid::new(c.spin_pid, c.max_spin_accel),
            pitch_pid: Pid::new(c.pitch_pid, c.max_pitch_accel),
            forward_sign: 1.0,
            stance_override_sign: 1.0,
            ..Self::default()
        }
    }

    pub fn store_kinematics(&mut self, k: &Kinematics) {
        (self.forward_sign, self.lean_norm) = (k.fwd, k.lean_norm);
    }
}

const GROUND_FRAME_ROLLED: f32 = 0.7;

pub fn ground_frame(alignment: Quat, n: Vec3) -> Quat {
    let project = |d: Vec3| (d - n * d.dot(n)).try_normalize();
    let side = alignment * Vec3::Z;
    let x = if side.dot(n).clamp(-1.0, 1.0).abs() > GROUND_FRAME_ROLLED {
        project(alignment * Vec3::X)
    } else {
        project(side).map(|z| n.cross(z))
    };
    let x = x.unwrap_or_else(|| n.any_orthonormal_vector());
    Quat::from_mat3(&glam::Mat3::from_cols(x, n, x.cross(n))).normalize()
}

pub fn vmx_cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    let lane = |i: usize| vmx::nmsub(a[(i + 1) % 3], b[i], vmx::mul(a[i], b[(i + 1) % 3]));
    [lane(1), lane(2), lane(0)]
}

pub fn vmx_quat_mul(a: Quat, b: Quat) -> Quat {
    let (av, bv) = ([a.x, a.y, a.z], [b.x, b.y, b.z]);
    let c = vmx_cross(av, bv);
    let xyz: [f32; 3] = std::array::from_fn(|i| vmx::madd(av[i], b.w, vmx::madd(bv[i], a.w, c[i])));
    Quat::from_xyzw(xyz[0], xyz[1], xyz[2], vmx::sub(vmx::mul(a.w, b.w), vmx::dot3(av, bv)))
}

pub fn vmx_conjugate(q: Quat) -> Quat {
    Quat::from_xyzw(-q.x, -q.y, -q.z, q.w)
}

pub fn vmx_normalize(q: Quat) -> Quat {
    let a = q.to_array();
    let r = vmx::rsqrt(vmx::dot4(a, a));
    Quat::from_array(a.map(|x| vmx::mul(x, r)))
}

pub fn vmx_axis_angle(axis: [f32; 3], angle: f32) -> Quat {
    let (s, c) = vmx::sin_cos(vmx::mul(angle, 0.5));
    Quat::from_xyzw(vmx::mul(axis[0], s), vmx::mul(axis[1], s), vmx::mul(axis[2], s), c)
}

pub fn vmx_frame_to_world(a: Quat, b: Quat) -> Quat {
    vmx_normalize(vmx_quat_mul(vmx_quat_mul(b, a), vmx_conjugate(b)))
}

pub fn vmx_rotate(q: Quat, v: Vec3) -> Vec3 {
    let (qv, vv) = ([q.x, q.y, q.z], v.to_array());
    let c = vmx_cross(qv, vv);
    let u: [f32; 3] = std::array::from_fn(|i| vmx::madd(q.w, vv[i], c[i]));
    let c2 = vmx_cross(qv, u);
    Vec3::from_array(std::array::from_fn(|i| vmx::madd(2.0, c2[i], vv[i])))
}

fn vmx_atan2(y: f32, x: f32) -> f32 {
    let r0 = vmx::refp(x);
    let r1 = vmx::madd(r0, vmx::nmsub(r0, x, 1.0), r0);
    let a = vmx::atan(vmx::madd(y, r1, 0.0));
    let sign = y.to_bits() & 0x8000_0000;
    let a = if vmx::gt(0.0, x) { vmx::add(f32::from_bits(PI.to_bits() | sign), a) } else { a };
    if vmx::eq(0.0, x) { f32::from_bits(FRAC_PI_2.to_bits() | sign) } else { a }
}

pub fn velocity_angle_in_trajectory_frame(q_traj: Quat, v: Vec3, speed_threshold: f32, fallback: f32) -> f32 {
    let l = vmx_rotate(q_traj.conjugate(), v);
    let m = vmx::abs(vmx::add(vmx::mul(l.x, l.x), vmx::mul(l.z, l.z)));
    if vmx::ge(vmx::mul(speed_threshold, speed_threshold), m) { fallback } else { vmx_atan2(l.z, l.x) }
}

pub fn twist(q: Quat, axis: Vec3) -> f32 {
    let a = vmx_atan2(vmx::dot3([q.x, q.y, q.z], axis.to_array()), q.w);
    let v = vmx::mul(vmx::mul(2.0, a), f32::from_bits(0x3E22_F983));
    let f = vmx::sub(v, vmx::floor(v));
    let f = vmx::sub(f, if vmx::gt(f, 0.5) { 1.0 } else { 0.0 });
    vmx::mul(f, f32::from_bits(0x40C9_0FDB))
}

pub fn lean_bias_angle(q: Quat, w: f32) -> f32 {
    let up = vmx_rotate(q, Vec3::Y);
    let eps = f32::from_bits(0x3400_0000);
    if !up.to_array().iter().any(|c| vmx::gt(vmx::abs(*c), eps)) {
        return 0.0;
    }
    (1.0 - w) * vmx_atan2(up.z, up.y)
}

pub fn lean_angle(q: Quat, bias: f32) -> f32 {
    wrap_pi_f64(twist(q, Vec3::X) - bias)
}

pub fn fsel_sign(x: f32) -> f32 {
    if x >= 0.0 { 1.0 } else { -1.0 }
}

pub const CACHED_SPIN_ANGLE: f32 = 0.0;

pub fn kinematics(
    st: &SteeringState,
    c: &CarvingTuning,
    q: Quat,
    alignment: Quat,
    v: Vec3,
    dt: f32,
    ice: f32,
) -> Kinematics {
    let inv = q.conjugate();
    let local = vmx_quat_mul(inv, alignment);
    let m = st.concavity.max((c.uphill_coefficient * v.y).clamp(0.0, 1.0));
    let w = (c.bias - m).mul_add(c.bias_weight, m);
    let lean = lean_angle(local, lean_bias_angle(inv, w));
    let pitch = twist(local, Vec3::Z);
    let vl = vmx_rotate(inv, v);
    let slip = slip_angle(c, vl, CACHED_SPIN_ANGLE);
    let fwd = if vl.x.abs() <= c.speed_threshold { st.forward_sign } else { fsel_sign(vl.x) };
    let eq = (c.ice_axe.equilibrium_lean_angle - c.equilibrium_lean_angle).mul_add(ice, c.equilibrium_lean_angle);
    let r = 1.0 / eq;
    let step = (c.max_lean_vel * r) * dt;
    let target = (r * lean) * fwd;
    let x = (st.lean_norm - step).max(target).min(st.lean_norm + step);
    let lean_norm = x.clamp(-1.0, 1.0);
    Kinematics { lean, slip, pitch, fwd, lean_norm }
}

pub fn update_concavity(st: &mut SteeringState, c: &CarvingTuning, contact: bool, v_filtered: Vec3, n: Vec3, dt: f32) {
    if !contact {
        st.concavity = 0.0;
        return;
    }
    let target = (c.concavity_coefficient * v_filtered.dot(-n)).clamp(0.0, 1.0);
    st.concavity = if st.concavity.abs() <= 1.192_092_9e-7 {
        target
    } else {
        let e = f64::from(-(c.concavity_gain * dt)).exp() as f32;
        e.mul_add(st.concavity - target, target)
    };
}

#[allow(clippy::too_many_arguments)]
pub fn update_slip_state(
    st: &mut SteeringState,
    c: &CarvingTuning,
    a: &mut Quat,
    q: &mut Quat,
    n: Vec3,
    v_world: Vec3,
    lean_in: f32,
    brake: f32,
    stopped: bool,
    dt: f32,
) {
    if stopped {
        return;
    }
    let vl = q.conjugate() * v_world;
    let fwd = if vl.x.abs() <= c.speed_threshold { st.forward_sign } else { fsel_sign(vl.x) };
    let edge = fsel_sign(lean_in) * fwd;
    let flip = edge != st.slip_edge && st.slip_angle.abs() > FRAC_PI_4;
    if flip || brake > c.slip_edge_hold_threshold || st.slip_edge.abs() <= 1.525_879e-5 {
        st.slip_edge = edge;
    }
    if flip {
        let lean = lean_angle(vmx_quat_mul(q.conjugate(), *a), 0.0);
        st.stance_override_sign = -st.stance_override_sign;
        st.slip_angle = wrap_pi_f64(st.slip_angle + PI);
        st.slip_dig_angle = wrap_pi_f64(st.slip_dig_angle + PI);
        let dq = vmx_axis_angle([1.0, 0.0, 0.0], vmx::mul(-lean, 2.0));
        *a = vmx_normalize(vmx_quat_mul(vmx_frame_to_world(dq, *q), *a));
        st.lean_pid.mirror();
        *q = ground_frame(*a, n);
    }
    let target = brake * st.slip_edge * FRAC_PI_2;
    let (ds, dd) = (c.max_slip_angle_delta * dt, c.max_slip_dig_angle_delta * dt);
    st.slip_angle += wrap_pi_f64(target - st.slip_angle).max(-ds).min(ds);
    st.slip_dig_angle += wrap_pi_f64(target - st.slip_dig_angle).max(-dd).min(dd);
}

pub fn slip_angle(c: &CarvingTuning, vl: Vec3, cached: f32) -> f32 {
    if vl.x.abs() <= c.stopped_threshold && vl.z.abs() <= c.stopped_threshold {
        return cached;
    }
    if vl.x == 0.0 { FRAC_PI_2.copysign(vl.z) } else { (vl.z / vl.x).atan() }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SteeringInputs {
    pub angle: f32,
    pub mag: f32,
    pub slip_angle: f32,
    pub slip_mag: f32,
    pub lean_in: f32,
    pub lean_weight: f32,
    pub slip_amount: f32,
    pub brake: f32,
}

pub fn steering_inputs(c: &CarvingTuning, stick: Vec2, speed: f32) -> SteeringInputs {
    read_inputs(c, stick, c.max_lean_vs_speed_curve.eval(speed), 1.0, true, (false, false), None, (0.0, false), 0.0)
}

pub fn max_lean_input_angle(
    c: &CarvingTuning,
    curves: (&crate::curve::Curve, &crate::curve::Curve),
    speed: f32,
    blend: f32,
    board_press: bool,
    ice: f32,
) -> f32 {
    let n = c.max_lean_vs_speed_curve.eval(speed);
    let a = n + (c.ice_axe.max_lean_input_curve.eval(speed) - n) * ice;
    let other = if board_press { curves.1 } else { curves.0 };
    a + (other.eval(speed) - a) * blend
}

#[allow(clippy::too_many_arguments)]
pub fn read_inputs(
    c: &CarvingTuning,
    stick: Vec2,
    max_in: f32,
    damp: f32,
    can_brake: bool,
    zero: (bool, bool),
    force_stop: Option<f32>,
    press: (f32, bool),
    ice: f32,
) -> SteeringInputs {
    let (mut angle, mut mag) = stick_angle_and_magnitude(stick);
    (angle, mag) = (angle * damp, mag * damp);
    let (mut slip_angle, mut slip_mag) = if can_brake { (angle, mag) } else { (0.0, 0.0) };
    if zero.0 {
        (angle, mag, slip_angle, slip_mag) = (0.0, 0.0, 0.0, 0.0);
    }
    if let Some(m) = force_stop {
        (angle, mag, slip_angle, slip_mag) = if can_brake { (-PI, m, -PI, m) } else { (0.0, 0.0, 0.0, 0.0) };
    }
    if zero.1 {
        (angle, mag, slip_angle, slip_mag) = (0.0, 0.0, 0.0, 0.0);
    }
    let lean_in = lean_input(c, angle, mag, max_in, ice, can_brake);
    let (blend, board_press) = press;
    let early_angle = if board_press { slip_angle } else { (1.0 - blend) * slip_angle };
    let (lean_weight, slip_amount, brake) = early_slip(c, early_angle, slip_mag, max_in);
    SteeringInputs { angle, mag, slip_angle, slip_mag, lean_in, lean_weight, slip_amount, brake }
}

pub fn apply_rotation(alignment: Quat, q: Quat, dq: Quat) -> Quat {
    vmx_normalize(vmx_quat_mul(vmx_frame_to_world(dq, q), alignment))
}

pub fn rates_to_rotation(c: &CarvingTuning, dt: f32, x_lean: f32, x_spin: f32, x_pitch: f32) -> Quat {
    let ql = vmx_axis_angle([1.0, 0.0, 0.0], c.max_lean_vel * dt * x_lean);
    let qs = vmx_axis_angle([0.0, 1.0, 0.0], c.max_spin_vel * dt * x_spin);
    let qp = vmx_axis_angle([0.0, 0.0, 1.0], c.max_pitch_vel * dt * x_pitch);
    vmx_quat_mul(vmx_quat_mul(qs, qp), ql)
}

#[allow(clippy::too_many_arguments)]
pub fn pid_step(
    st: &mut SteeringState,
    c: &CarvingTuning,
    k: &Kinematics,
    i: &SteeringInputs,
    stopped: bool,
    grind: Option<(&crate::rider::grind::GrindTuning, f32)>,
    dt: f32,
    ice: f32,
) -> Quat {
    let lean_in = if stopped { 0.0 } else { i.lean_in };
    let max_lean = (c.ice_axe.max_lean_angle - c.max_lean_angle).mul_add(ice, c.max_lean_angle);
    let target = c.lean_input_curve.eval(lean_in.abs()) * c.max_lean_input * lean_in;
    let s_lean = target.mul_add(k.fwd, (-1.0 / max_lean) * k.lean).clamp(-1.0, 1.0);
    let s_pitch = (-k.pitch / c.max_pitch_angle).clamp(-1.0, 1.0) * c.pitch_vs_normal_curve.eval(st.normal_force);
    st.lean_pid.update(dt, s_lean);
    st.pitch_pid.update(dt, s_pitch);
    let dir = fsel_sign(st.slip_angle);
    let mut s_spin = c.max_spin_input * (k.fwd * dir * i.slip_amount - i.lean_weight * k.lean_norm)
        + (-k.slip / c.max_spin_angle).clamp(-1.0, 1.0);
    if stopped {
        s_spin = 0.0;
    }
    if let Some((g, lateral)) = grind {
        let mut sc = crate::rider::grind::SpinController {
            max_vel: c.max_spin_vel,
            vel_gain: g.spin_velocity_gain,
            angle_gain: g.spin_angle_gain,
            angle: k.slip,
            rate: st.spin_pid.x * c.max_spin_vel,
        };
        sc.update(dt, c.max_spin_vel * lateral);
        st.spin_pid.set_driven(sc.rate / c.max_spin_vel);
    } else {
        st.spin_pid.update(dt, s_spin);
    }
    rates_to_rotation(c, dt, st.lean_pid.x, st.spin_pid.x, st.pitch_pid.x)
}
