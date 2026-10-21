use std::f32::consts::{FRAC_PI_2, PI, TAU};

use anyhow::{Result, bail};
use glam::{Quat, Vec2, Vec3};

use crate::actuator::{Actuator, NO_LANDING, crit_damped, wrap_angle};
use crate::character::ResolvedCharacterProperties;
use crate::curve::Curve;
use crate::tuning::{AssetView, Bundle};

const EPS: f32 = 1.192_092_9e-7;

#[derive(Clone, Debug, Default)]
pub struct AerialVariant {
    pub takeoff_flip_strength: Option<Curve>,
    pub takeoff_spin_strength: Option<Curve>,
    pub enable_air_jumps: bool,
    pub enable_prewind_by_default: bool,
}

#[derive(Clone, Debug, Default)]
pub struct AerialTuning {
    pub input_x_dead_zone_time: f32,
    pub input_y_dead_zone_time: f32,
    pub auto_upright_time: f32,
    pub flip_wiggle_responsiveness: f32,
    pub spin_wiggle_responsiveness: f32,
    pub roll_max_velocity: f32,
    pub roll_max_prewind_velocity: f32,
    pub roll_threshold_angle: f32,
    pub roll_threshold_velocity_scale: f32,
    pub wobble_max_velocity: f32,
    pub wobble_adjustment_ratio: f32,
    pub flip_max_assist_velocity: f32,
    pub flip_assist_curve: Curve,
    pub flip_forward_balance_compensation_time: f32,
    pub flip_backward_balance_compensation_time: f32,
    pub flip_bias_velocity: f32,
    pub flip_idle_speed_percentage: f32,
    pub flip_idle_time_threshold: f32,
    pub swivel_brake_tolerance: f32,
    pub swivel_convergence: f32,
    pub spin_max_assist_velocity: f32,
    pub spin_assist_curve: Curve,
    pub spin_balance_compensation_time: f32,
    pub spin_idle_speed_percentage: f32,
    pub spin_idle_time_threshold: f32,
    pub airborne: AerialVariant,
    pub jump: AerialVariant,
}

impl AerialVariant {
    fn load(a: &AssetView<'_>) -> Result<Self> {
        let curve = |f: &str| a.asset_opt(f)?.map(|c| c.as_curve()).transpose();
        Ok(Self {
            takeoff_flip_strength: curve("TakeoffFlipStrengthCurve")?,
            takeoff_spin_strength: curve("TakeoffSpinStrengthCurve")?,
            enable_air_jumps: a.int("EnableAirJumps")? != 0,
            enable_prewind_by_default: a.int("EnablePrewindByDefault")? != 0,
        })
    }
}

impl AerialTuning {
    pub fn load(bundle: &Bundle) -> Result<Self> {
        let a = bundle.view("aerial_airborne", "AerialControllerAsset")?;
        let j = bundle.view("aerial_jump", "AerialControllerAsset")?;
        for ((name, x), (_, y)) in a.floats().into_iter().zip(j.floats()) {
            if x != y {
                bail!("aerial_airborne.{name} = {x} but aerial_jump has {y} (expected identical floats)");
            }
        }
        Ok(Self {
            input_x_dead_zone_time: a.f32("InputXAxisDeadZoneTime")?,
            input_y_dead_zone_time: a.f32("InputYAxisDeadZoneTime")?,
            auto_upright_time: a.f32("AutoUprightTime")?,
            flip_wiggle_responsiveness: a.f32("FlipWiggleAngleResponsiveness")?,
            spin_wiggle_responsiveness: a.f32("SpinWiggleAngleResponsiveness")?,
            roll_max_velocity: a.f32("RollMaxVelocity")?,
            roll_max_prewind_velocity: a.f32("RollMaxPrewindVelocity")?,
            roll_threshold_angle: a.f32("RollThresholdAngle")?,
            roll_threshold_velocity_scale: a.f32("RollThresholdVelocityScale")?,
            wobble_max_velocity: a.f32("WobbleMaxVelocity")?,
            wobble_adjustment_ratio: a.f32("WobbleAdjustmentRatio")?,
            flip_max_assist_velocity: a.f32("FlipMaxAssistVelocity")?,
            flip_assist_curve: a.curve("FlipAssistFactorCurve")?,
            flip_forward_balance_compensation_time: a.f32("FlipForwardBalanceCompensationTime")?,
            flip_backward_balance_compensation_time: a.f32("FlipBackwardBalanceCompensationTime")?,
            flip_bias_velocity: a.f32("FlipBiasVelocity")?,
            flip_idle_speed_percentage: a.f32("FlipIdleSpeedPercentage")?,
            flip_idle_time_threshold: a.f32("FlipIdleTimeThreshold")?,
            swivel_brake_tolerance: a.f32("SwivelDirectionBrakeTolerance")?,
            swivel_convergence: a.f32("SwivelDirectionConvergence")?,
            spin_max_assist_velocity: a.f32("SpinMaxAssistVelocity")?,
            spin_assist_curve: a.curve("SpinAssistFactorCurve")?,
            spin_balance_compensation_time: a.f32("SpinBalanceCompensationTime")?,
            spin_idle_speed_percentage: a.f32("SpinIdleSpeedPercentage")?,
            spin_idle_time_threshold: a.f32("SpinIdleTimeThreshold")?,
            airborne: AerialVariant::load(&a)?,
            jump: AerialVariant::load(&j)?,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Takeoff {
    #[default]
    RollOff,
    Jump,
    AirJump,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LandingPrediction {
    pub ticks: f32,
    pub normal: Vec3,
}

pub fn q_flip(a: f32) -> Quat {
    Quat::from_rotation_z(-a)
}

pub fn q_spin(a: f32) -> Quat {
    Quat::from_rotation_y(-a)
}

fn frame(x: Vec3, y: Vec3) -> Quat {
    Quat::from_mat3(&glam::Mat3::from_cols(x, y, x.cross(y))).normalize()
}

const FRAME_PARALLEL: f32 = 0.05;

pub fn frame_from_velocity(v: Vec3, fallback: Quat) -> Quat {
    if v.abs().max_element() <= EPS {
        return fallback;
    }
    let x = v.normalize();
    let side = x.cross(Vec3::Y);
    if side.abs().max_element() <= FRAME_PARALLEL {
        return fallback;
    }
    let z = side.normalize();
    frame(x, z.cross(x))
}

pub fn frame_heading_normal(v: Vec3, n: Vec3, fallback: Quat) -> Quat {
    let side = v.cross(Vec3::Y);
    if v.abs().max_element() <= EPS || side.abs().max_element() <= FRAME_PARALLEL {
        return fallback;
    }
    let z = side.normalize();
    let y = n - z * n.dot(z);
    if y.abs().max_element() <= FRAME_PARALLEL {
        return fallback;
    }
    let y = y.normalize();
    frame(y.cross(z), y)
}

pub fn swivel_angle(d: Vec3) -> f32 {
    (d.x.atan2(d.z) - FRAC_PI_2).rem_euclid(TAU)
}

pub fn compose(psi: f32, flip: f32, res: Quat, spin: f32) -> Quat {
    Quat::from_rotation_y(psi) * q_flip(flip) * res * q_spin(spin)
}

pub fn decompose(rel: Quat, psi: f32) -> (f32, f32, Quat) {
    let m = Quat::from_rotation_y(-psi) * rel;
    let u = m * Vec3::Y;
    let flip = wrap_angle(u.x.atan2(u.y));
    let m2 = q_flip(flip).conjugate() * m;
    let x = m2 * Vec3::X;
    let spin = wrap_angle(x.z.atan2(x.x));
    let res = (m2 * q_spin(spin).conjugate()).normalize();
    (flip, spin, res)
}

fn towards_identity(q: Quat, max: f32) -> Quat {
    let angle = q.angle_between(Quat::IDENTITY);
    if angle <= max || angle <= EPS { Quat::IDENTITY } else { q.slerp(Quat::IDENTITY, max / angle).normalize() }
}

fn up_tilt_about_x(u: Vec3) -> f32 {
    u.z.atan2(u.y)
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AirOutputs {
    pub flip_speed: f32,
    pub spin_speed: f32,
    pub flip_magnitude: f32,
    pub spin_magnitude: f32,
    pub target_flip_magnitude: f32,
    pub target_spin_magnitude: f32,
    pub flip_angle_degrees: f32,
    pub air_idle: bool,
    pub air_idle_time: f32,
    pub swivel_direction: Vec3,
    pub flip_wiggle: f32,
    pub spin_wiggle: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Assist {
    pub flip: f32,
    pub flip_ticks: f32,
    pub spin: f32,
    pub spin_ticks: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AirRotation {
    pub takeoff: Takeoff,
    pub first_tick: bool,
    pub q_ref: Quat,
    pub q_land: Quat,
    pub q_res: Quat,
    pub flip_dir: Vec3,
    pub flip_axis: Vec3,
    pub swivel: bool,
    pub flip: Actuator,
    pub spin: Actuator,
    pub flip_input: f32,
    pub spin_input: f32,
    pub flip_idle: f32,
    pub spin_idle: f32,
    pub air_idle: f32,
    pub air_time: f32,
    pub land_flip_period: f32,
    pub land_flip_offset: f32,
    pub land_spin_period: f32,
    pub land_spin_offset: f32,
    pub land_weight: f32,
    pub ticks_to_land: f32,
    pub target_valid: bool,
    pub prediction: Option<LandingPrediction>,
    pub flip_wiggle: [f32; 3],
    pub spin_wiggle: [f32; 3],
    pub outputs: AirOutputs,
}

#[derive(Clone, Copy, Debug)]
pub struct AirInputs {
    pub stick: Vec2,
    pub velocity: Vec3,
    pub allow_stance_switch: bool,
    pub grind_steering: Option<f32>,
    pub tutorial_mode: bool,
    pub heli_interact: Option<bool>,
}

impl AirRotation {
    #[allow(clippy::too_many_arguments)]
    pub fn takeoff(
        t: &AerialTuning,
        r: &ResolvedCharacterProperties,
        takeoff: Takeoff,
        board: Quat,
        wf: Quat,
        velocity: Vec3,
        ground_normal: Vec3,
        stick: Vec2,
        prewind: f32,
        prediction: Option<LandingPrediction>,
    ) -> Self {
        let variant = if takeoff == Takeoff::Jump { &t.jump } else { &t.airborne };
        let q_ref = frame_from_velocity(velocity, board);
        let q_land = frame_heading_normal(velocity, Vec3::Y, board);
        let (theta0, phi0, q_res) = decompose(q_ref.conjugate() * board, 0.0);
        let (d_theta, d_phi, _) = decompose((q_ref.conjugate() * wf * q_ref).normalize(), 0.0);
        let prewind = if takeoff == Takeoff::AirJump || variant.enable_prewind_by_default { prewind } else { 0.0 };
        let (mut x, mut y) = (stick.x, stick.y);
        let tilt0 = up_tilt_about_x(q_land.conjugate() * ground_normal);
        let tilt_pred = up_tilt_about_x(q_land.conjugate() * (board * Vec3::Y))
            + t.roll_threshold_velocity_scale * up_tilt_about_x(q_land.conjugate() * (wf * Vec3::Y));
        let roll = tilt0.abs() > t.roll_threshold_angle
            && tilt_pred.abs() > t.roll_threshold_angle
            && tilt_pred * x * prewind >= 0.0;
        let (swivel, flip_dir, flip_v) = if roll {
            (y, x) = (x, 0.0);
            let v = d_theta + t.roll_max_prewind_velocity * y * prewind;
            let (lo, hi) = if tilt_pred > 0.0 { (0.0, t.roll_max_velocity) } else { (-t.roll_max_velocity, 0.0) };
            (true, Vec3::Z, v.clamp(lo, hi))
        } else {
            (false, Vec3::X, d_theta + r.flip_max_prewind_velocity * y * prewind + t.flip_bias_velocity)
        };
        let spin_v = d_phi + if swivel { 0.0 } else { r.spin_max_prewind_velocity * x * prewind };
        let flip_v = flip_v.clamp(-r.flip_max_launch_velocity, r.flip_max_launch_velocity);
        let spin_v = spin_v.clamp(-r.spin_max_launch_velocity, r.spin_max_launch_velocity);
        let strength = |c: &Option<Curve>| match (c, prediction) {
            (Some(c), Some(p)) => c.eval(p.ticks).clamp(0.0, 1.0),
            _ => 1.0,
        };
        let (sf, ss) = (strength(&variant.takeoff_flip_strength), strength(&variant.takeoff_spin_strength));
        let flip = Actuator::new(
            r.flip_actuator_strength,
            theta0,
            flip_v,
            y,
            sf * r.flip_max_velocity,
            r.flip_max_velocity,
            t.flip_max_assist_velocity,
        );
        let spin = Actuator::new(
            r.spin_actuator_strength,
            phi0,
            spin_v,
            x,
            ss * r.spin_max_velocity,
            r.spin_max_velocity,
            t.spin_max_assist_velocity,
        );
        Self {
            takeoff,
            first_tick: true,
            q_ref,
            q_land,
            q_res,
            flip_dir,
            flip_axis: Vec3::X,
            swivel,
            flip,
            spin,
            flip_input: y,
            spin_input: x,
            land_flip_period: TAU,
            land_spin_period: PI,
            ticks_to_land: NO_LANDING,
            prediction,
            ..Self::default()
        }
    }

    pub fn orientation(&self) -> Quat {
        (self.q_ref * compose(swivel_angle(self.flip_dir), self.flip.angle, self.q_res, self.spin.angle)).normalize()
    }
}

impl AirRotation {
    pub fn step(
        &mut self,
        t: &AerialTuning,
        r: &ResolvedCharacterProperties,
        inp: &AirInputs,
        board: Quat,
        goofy: &mut bool,
        dt: f32,
    ) -> Quat {
        self.air_time += dt;
        if self.first_tick {
            self.first_tick = false;
        } else {
            self.select_landing_target(t, inp, board);
        }
        self.drift_reference_frame(t, dt);
        self.read_stick_inputs(t, inp.stick);
        self.step_actuators(t, r, inp.stick, board, dt);
        if self.air_idle.abs() > EPS && inp.allow_stance_switch {
            let z = (self.q_ref.conjugate() * board) * Vec3::Z;
            if z.z.abs() > EPS {
                *goofy = z.z < 0.0;
            }
        }
        self.idle_timers(t, dt);
        self.write_outputs(board);
        self.orientation()
    }

    fn select_landing_target(&mut self, t: &AerialTuning, inp: &AirInputs, board: Quat) {
        let (v, grind_steering, tutorial_mode) = (inp.velocity, inp.grind_steering, inp.tutorial_mode);
        (self.land_flip_period, self.land_flip_offset, self.land_spin_period, self.land_spin_offset) =
            (TAU, 0.0, PI, 0.0);
        self.land_weight = 0.0;
        if let Some(left) = inp.heli_interact {
            self.q_land = frame_heading_normal(v, Vec3::Y, board);
            (self.land_spin_period, self.land_spin_offset) = (TAU, if left { 0.0 } else { PI });
            self.ticks_to_land = 0.0;
            self.land_weight = 1.0;
            self.target_valid = true;
        } else if let Some(ttl) = grind_steering {
            self.q_land = frame_heading_normal(v, Vec3::Y, board);
            self.ticks_to_land = (ttl - 6.0).max(0.0);
            self.land_weight = 1.0;
            self.target_valid = true;
        } else {
            self.select_ballistic_target(v, board);
        }
        if self.flip_input.abs() <= EPS && self.spin_input.abs() <= EPS && self.ticks_to_land > t.auto_upright_time {
            self.q_land = frame_heading_normal(v, Vec3::Y, board);
            self.ticks_to_land = t.auto_upright_time;
            self.land_weight = 1.0;
            self.target_valid = false;
            if tutorial_mode {
                (self.land_spin_period, self.land_spin_offset) = (TAU, 0.0);
            }
        }
    }

    fn select_ballistic_target(&mut self, v: Vec3, board: Quat) {
        match self.prediction {
            Some(p) => {
                self.q_land = frame_heading_normal(v, p.normal, board);
                self.ticks_to_land = p.ticks;
                self.target_valid = true;
            }
            None => {
                self.q_land = frame_heading_normal(v, Vec3::Y, board);
                self.ticks_to_land = NO_LANDING;
                self.target_valid = false;
            }
        }
    }

    fn drift_reference_frame(&mut self, t: &AerialTuning, dt: f32) {
        let d = (self.q_ref.conjugate() * self.q_land).normalize();
        let (axis, angle) = d.to_axis_angle();
        let angle = wrap_angle(angle);
        let step = (t.wobble_adjustment_ratio * self.flip.vel * dt).abs().min(t.wobble_max_velocity);
        if axis.is_finite() && angle.abs() > EPS {
            self.q_ref = (self.q_ref * Quat::from_axis_angle(axis, angle.clamp(-step, step))).normalize();
        }
    }

    fn read_stick_inputs(&mut self, t: &AerialTuning, stick: Vec2) {
        let ttl = self.prediction.map_or(NO_LANDING, |p| p.ticks);
        let x = if ttl < t.input_x_dead_zone_time && self.spin_idle != 0.0 { 0.0 } else { stick.x };
        let y = if ttl < t.input_y_dead_zone_time && self.flip_idle != 0.0 { 0.0 } else { stick.y };
        if self.flip_idle >= t.flip_idle_time_threshold && self.spin_idle >= t.spin_idle_time_threshold {
            self.swivel = false;
        }
        if self.swivel {
            let d = Vec3::new(y, 0.0, x);
            self.flip_axis = d.try_normalize().unwrap_or(self.flip_dir);
            (self.flip_input, self.spin_input) = (d.length(), 0.0);
        } else {
            (self.flip_axis, self.flip_input, self.spin_input) = (Vec3::X, y, x);
        }
        let brake = (swivel_angle(self.flip_dir) - swivel_angle(self.flip_axis) - PI).rem_euclid(TAU);
        let brake = brake.min(TAU - brake);
        if self.flip.vel.abs() > EPS && brake <= t.swivel_brake_tolerance {
            (self.flip_axis, self.flip_input) = (-self.flip_axis, -self.flip_input);
        }
    }
}

impl AirRotation {
    fn step_actuators(&mut self, t: &AerialTuning, r: &ResolvedCharacterProperties, raw: Vec2, board: Quat, dt: f32) {
        let rel = self.q_ref.conjugate() * board;
        if self.flip_input.abs() > EPS {
            let target = self.flip_axis * (self.flip.stick_scale * self.flip_input).abs();
            let d = target + (self.flip_dir * self.flip.vel.abs() - target) * (-t.swivel_convergence * dt).exp();
            self.flip_dir = d.try_normalize().unwrap_or(self.flip_dir);
        }
        let psi = swivel_angle(self.flip_dir);
        (self.flip.angle, self.spin.angle, self.q_res) = decompose(rel, psi);
        self.flip.set_input(self.flip_input);
        self.spin.set_input(self.spin_input);
        (self.flip_wiggle[2], self.spin_wiggle[2]) = (raw.y, raw.x);
        let Assist { flip: fa, flip_ticks: ft, spin: sa, spin_ticks: st } = self.assist(t, r);
        self.flip.update(dt, ft, self.land_flip_offset, fa, self.land_flip_period);
        self.spin.update(dt, st, self.land_spin_offset, sa, self.land_spin_period);
        for (w, resp) in [
            (&mut self.flip_wiggle, t.flip_wiggle_responsiveness),
            (&mut self.spin_wiggle, t.spin_wiggle_responsiveness),
        ] {
            let (e, rate) = crit_damped(w[0] - w[2], w[1], resp.max(0.0).sqrt(), dt);
            (w[0], w[1]) = (e + w[2], rate);
        }
        let wobble = (t.wobble_adjustment_ratio * self.flip.vel * dt).abs().min(t.wobble_max_velocity);
        self.q_res = towards_identity(self.q_res, wobble);
    }

    pub fn assist(&self, t: &AerialTuning, r: &ResolvedCharacterProperties) -> Assist {
        let w = self.land_weight;
        if !(self.target_valid || w > 0.0) {
            return Assist { flip: w, flip_ticks: NO_LANDING, spin: w, spin_ticks: NO_LANDING };
        }
        let lerp = |a: f32, b: f32, s: f32| a + (b - a) * s;
        let flip = w
            .max(lerp(r.flip_max_assist_factor, r.flip_min_assist_factor, self.flip_input.abs()))
            .max(t.flip_assist_curve.eval(self.air_time));
        let s = self.flip_dir.x * sign(self.flip.vel);
        let flip_ticks = (self.ticks_to_land
            + t.flip_forward_balance_compensation_time * s.max(0.0)
            + t.flip_backward_balance_compensation_time * (-s).max(0.0))
        .max(1.0);
        let spin = w
            .max(lerp(r.spin_max_assist_factor, r.spin_min_assist_factor, self.spin_input.abs()))
            .max(t.spin_assist_curve.eval(self.air_time));
        let spin_ticks = (self.ticks_to_land + t.spin_balance_compensation_time).max(1.0);
        Assist { flip, flip_ticks, spin, spin_ticks }
    }

    fn idle_timers(&mut self, t: &AerialTuning, dt: f32) {
        let tick = |idle: bool, c: f32| if idle { c + dt } else { 0.0 };
        self.flip_idle = tick(self.flip.is_idle(t.flip_idle_speed_percentage), self.flip_idle);
        self.spin_idle = tick(self.spin.is_idle(t.spin_idle_speed_percentage), self.spin_idle);
        let both = self.flip_idle >= t.flip_idle_time_threshold && self.spin_idle >= t.spin_idle_time_threshold;
        self.air_idle = tick(both, self.air_idle);
    }

    fn write_outputs(&mut self, board: Quat) {
        let f = &self.flip;
        let back = f.normalized() < 0.0;
        let s = if back { -1.0 } else { 1.0 };
        let angle = swivel_angle(self.flip_dir).to_degrees() + if back { 180.0 } else { 0.0 };
        self.outputs = AirOutputs {
            flip_speed: s * f.vel,
            spin_speed: self.spin.vel,
            flip_magnitude: s * f.magnitude(),
            spin_magnitude: self.spin.magnitude(),
            target_flip_magnitude: s * f.normalized_unassisted(),
            target_spin_magnitude: self.spin.normalized_unassisted(),
            flip_angle_degrees: angle.rem_euclid(360.0),
            air_idle: self.air_idle > 0.0,
            air_idle_time: self.air_idle,
            swivel_direction: self.flip_dir,
            flip_wiggle: self.flip_wiggle_output(board),
            spin_wiggle: self.spin_wiggle_output(),
        };
    }

    fn flip_wiggle_output(&self, board: Quat) -> f32 {
        let z = ((self.q_ref.conjugate() * board) * Vec3::Z).z;
        let x = z.abs().min(1.0);
        let s = if z >= 0.0 { 1.0 } else { -1.0 };
        self.flip_wiggle[0] * x * x * x * (x * (6.0 * x - 15.0) + 10.0) * s
    }

    fn spin_wiggle_output(&self) -> f32 {
        let r = if self.spin.stick_scale.abs() > EPS { self.spin.vel / self.spin.stick_scale } else { 0.0 };
        let w = self.spin_wiggle[0];
        if w > 1.525_879e-5 {
            (w - r.max(0.0)).max(0.0)
        } else if w < -1.525_879e-5 {
            (w - r.min(0.0)).min(0.0)
        } else {
            0.0
        }
    }
}

fn sign(x: f32) -> f32 {
    if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests;
