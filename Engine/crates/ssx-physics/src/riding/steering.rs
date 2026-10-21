use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI};

use glam::{Quat, Vec2, Vec3};

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
        while rem > 1.525_88e-5 {
            let h = rem.min(1.0);
            rem -= h;
            let u = self.compute(h, s);
            let x_old = self.x;
            self.x = (self.x + h * (self.rate + h * self.gain * u)).clamp(-1.0, 1.0);
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
        let x = self.x;
        let d = (self.x_prev - x) / h;
        self.x_prev = x;
        self.integral += (s - x) * h;
        let g = self.gains;
        let u = g.p * (s - x) + g.i * self.integral + g.d * d;
        let out = if u.abs() <= 1.19e-7 { 0.0 } else { u.signum() * u.abs().min(1.0) };
        if g.i.abs() > 1.19e-7 {
            self.integral -= ((u - out) / g.i).clamp(self.integral.min(0.0), self.integral.max(0.0));
        }
        out
    }
}

pub fn wrap_pi(a: f32) -> f32 {
    let w = (a + PI).rem_euclid(2.0 * PI) - PI;
    if w <= -PI { w + 2.0 * PI } else { w }
}

pub fn stick_angle_and_magnitude(stick: Vec2) -> (f32, f32) {
    let mag = stick.length().min(1.0);
    if mag <= 1e-6 {
        return (0.0, 0.0);
    }
    (-wrap_pi(stick.y.atan2(stick.x) - FRAC_PI_2), mag)
}

pub fn lean_input(c: &CarvingTuning, angle: f32, mag: f32, max_in: f32, ice: f32) -> f32 {
    let n = c.lean_input_angle_transform.eval(angle);
    let t = (n + (c.ice_axe.lean_input_transform.eval(angle) - n) * ice).clamp(-FRAC_PI_2, FRAC_PI_2);
    let lim = max_in.sin();
    mag * t.sin().clamp(-lim, lim)
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
    pub cached_slip: f32,
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

pub fn twist(q: Quat, axis: Vec3) -> f32 {
    wrap_pi(2.0 * Vec3::new(q.x, q.y, q.z).dot(axis).atan2(q.w))
}

pub fn fsel_sign(x: f32) -> f32 {
    if x >= 0.0 { 1.0 } else { -1.0 }
}

pub fn kinematics(
    st: &mut SteeringState,
    c: &CarvingTuning,
    q: Quat,
    alignment: Quat,
    v: Vec3,
    dt: f32,
    ice: f32,
) -> Kinematics {
    let inv = q.conjugate();
    let local = inv * alignment;
    let up = inv * Vec3::Y;
    let m = st.concavity.max((c.uphill_coefficient * v.y).clamp(0.0, 1.0));
    let w = m + c.bias_weight * (c.bias - m);
    let lean = wrap_pi(twist(local, Vec3::X) - (1.0 - w) * up.z.atan2(up.y));
    let pitch = twist(local, Vec3::Z);
    let vl = inv * v;
    let slip = slip_angle(c, vl, st.cached_slip);
    st.cached_slip = slip;
    if vl.x.abs() > c.speed_threshold {
        st.forward_sign = fsel_sign(vl.x);
    }
    let fwd = st.forward_sign;
    let eq = c.equilibrium_lean_angle + (c.ice_axe.equilibrium_lean_angle - c.equilibrium_lean_angle) * ice;
    let max_step = c.max_lean_vel * dt / eq;
    let target = lean * fwd / eq;
    st.lean_norm = (st.lean_norm + (target - st.lean_norm).clamp(-max_step, max_step)).clamp(-1.0, 1.0);
    Kinematics { lean, slip, pitch, fwd, lean_norm: st.lean_norm }
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
        target + (st.concavity - target) * (-c.concavity_gain * dt).exp()
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
        let lean = twist(q.conjugate() * *a, Vec3::X);
        st.stance_override_sign = -st.stance_override_sign;
        st.slip_angle = wrap_pi(st.slip_angle + PI);
        st.slip_dig_angle = wrap_pi(st.slip_dig_angle + PI);
        *a = (*q * Quat::from_rotation_x(-2.0 * lean) * q.conjugate() * *a).normalize();
        st.lean_pid.mirror();
        *q = ground_frame(*a, n);
    }
    let target = brake * st.slip_edge * FRAC_PI_2;
    let (ds, dd) = (c.max_slip_angle_delta * dt, c.max_slip_dig_angle_delta * dt);
    st.slip_angle += wrap_pi(target - st.slip_angle).clamp(-ds, ds);
    st.slip_dig_angle += wrap_pi(target - st.slip_dig_angle).clamp(-dd, dd);
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
    let lean_in = lean_input(c, angle, mag, max_in, ice);
    let (blend, board_press) = press;
    let early_angle = if board_press { slip_angle } else { (1.0 - blend) * slip_angle };
    let (lean_weight, slip_amount, brake) = early_slip(c, early_angle, slip_mag, max_in);
    SteeringInputs { angle, mag, slip_angle, slip_mag, lean_in, lean_weight, slip_amount, brake }
}

pub fn apply_rotation(alignment: Quat, q: Quat, dq: Quat) -> Quat {
    (q * dq * q.conjugate() * alignment).normalize()
}

pub fn rates_to_rotation(c: &CarvingTuning, dt: f32, x_lean: f32, x_spin: f32, x_pitch: f32) -> Quat {
    let ql = Quat::from_rotation_x(c.max_lean_vel * dt * x_lean);
    let qs = Quat::from_rotation_y(c.max_spin_vel * dt * x_spin);
    let qp = Quat::from_rotation_z(c.max_pitch_vel * dt * x_pitch);
    qs * qp * ql
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
    let target = c.max_lean_input * c.lean_input_curve.eval(lean_in.abs()) * lean_in * k.fwd;
    let max_lean = c.max_lean_angle + (c.ice_axe.max_lean_angle - c.max_lean_angle) * ice;
    let s_lean = (target - k.lean / max_lean).clamp(-1.0, 1.0);
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
