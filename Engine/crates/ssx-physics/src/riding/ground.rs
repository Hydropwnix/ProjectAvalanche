use glam::{Quat, Vec3};

use crate::character::{CarvingTuning, ResolvedCharacterProperties, Tuning};
use crate::curve::Curve;
use crate::rider::drag_accel;
use crate::steering::{Kinematics, fsel_sign, wrap_pi};
use crate::surface::SurfaceProperties;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CarvingState {
    pub have_made_first_contact: bool,
    pub use_leg_spring: bool,
    pub uls_ticks: f32,
    pub on_ground_ticks: f32,
    pub ride_ticks: f32,
    pub riding_height: f32,
    pub distance_carved: f32,
    pub chatter_height: f32,
    pub initialized: bool,
    pub collision_anticipation_side: f32,
    pub is_dead: bool,
    pub spray_magnitude: f32,
    pub spray_derivative: f32,
    pub ticks_on_rock: f32,
    pub air_time: f32,
}

pub fn snow_compression(c: &CarvingTuning, sp: &SurfaceProperties, distance_carved: f32) -> f32 {
    let restore = c.snow_compression_restore_curve.eval(distance_carved).max(0.0);
    let dig = c.snow_compression_from_dig_curve.eval(0.0);
    ((sp.snow_depth - sp.riding_depth).min(restore) - dig).clamp(0.0, sp.snow_depth.max(0.0))
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HeightBlends {
    pub tuck: f32,
    pub pre_jump: f32,
    pub boost: f32,
}

#[allow(clippy::too_many_arguments)]
pub fn riding_height_target(
    c: &CarvingTuning,
    b: HeightBlends,
    lean: f32,
    stable_ticks: f32,
    initialized: bool,
    traj_height: f32,
    forced: bool,
) -> f32 {
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t.clamp(0.0, 1.0);
    let mut h = lerp(
        lerp(lerp(c.riding_height_neutral, c.riding_height_tucked, b.tuck), c.riding_height_boost, b.boost),
        c.riding_height_pre_jump,
        b.pre_jump,
    );
    if forced {
        return -traj_height;
    }
    if initialized {
        h = lerp(-traj_height, h, c.riding_height_anim_blend_curve.eval(stable_ticks));
    }
    let cos = lean.abs().cos();
    let rad = c.collision_radius_base + c.collision_radius_upright_offset * cos;
    (cos * h).min(-rad)
}

pub fn update_riding_height(c: &CarvingTuning, rh: f32, kin30: f32, floor: Option<f32>, first: bool, dt: f32) -> f32 {
    let step = c.max_riding_height_delta * dt;
    let rh = if first { kin30 } else { rh + (kin30 - rh).clamp(-step, step) };
    rh.max(floor.unwrap_or(-1.0)).clamp(-c.riding_height_max, -c.riding_height_min)
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BrakeInputs {
    pub slip_angle: f32,
    pub slip_mag: f32,
    pub pitch: f32,
    pub dead_on_ground: bool,
    pub board_press: bool,
    pub uber: bool,
    pub grind_or_rock: bool,
    pub vertical: bool,
    pub ride_ticks: f32,
    pub can_brake: bool,
}

pub fn brake_factor(t: &Tuning, tumble: &Curve, i: &BrakeInputs) -> f32 {
    let c = &t.carving;
    if !i.vertical && (i.dead_on_ground || (i.uber && i.ride_ticks >= c.uber_crash_ticks_to_full_friction)) {
        1.0
    } else if i.board_press {
        0.1
    } else if i.grind_or_rock {
        c.board_press_friction_modifier
    } else if !i.can_brake {
        tumble.eval(i.pitch)
    } else {
        c.slip_friction_input_curve.eval(i.slip_angle.abs()) * i.slip_mag
    }
}

#[allow(clippy::too_many_arguments)]
pub fn friction_accel(
    t: &Tuning,
    sp: &SurfaceProperties,
    v: Vec3,
    lean: f32,
    slip_dig: f32,
    normal_y: f32,
    brake: f32,
    uber: bool,
) -> Vec3 {
    friction_accel_grind(t, sp, v, lean, slip_dig, normal_y, brake, uber, if uber { 0.0 } else { 1.0 }, None)
}

#[allow(clippy::too_many_arguments)]
pub fn friction_accel_grind(
    t: &Tuning,
    sp: &SurfaceProperties,
    v: Vec3,
    lean: f32,
    slip_dig: f32,
    normal_y: f32,
    brake: f32,
    uber: bool,
    grind_scale: f32,
    ice_slip: Option<f32>,
) -> Vec3 {
    if v.abs().max_element() <= 1.525_88e-5 {
        return Vec3::ZERO;
    }
    let mut slip = ice_slip.unwrap_or_else(|| slip_dig.abs().sin() + lean.abs().sin());
    if v.z.signum() == lean.signum() && lean != 0.0 && v.z != 0.0 {
        slip = 0.0;
    }
    let lateral = slip * sp.dig_lean * grind_scale;
    let brake_mu = if uber { sp.friction_uber_bail } else { sp.friction_brake };
    let mu = sp.friction + (brake_mu - sp.friction) * brake;
    let steep = (normal_y / t.carving.vertical_surface_angle_threshold.cos()).clamp(0.0, 1.0);
    let c = Vec3::new(mu * steep, 0.0, if uber { mu } else { lateral });
    -v.normalize() * c * t.carving.friction_force_multiplier
}

pub fn clamp_not_reversing(a: Vec3, v: Vec3, dt: f32) -> Vec3 {
    let lim = -v / dt;
    a.clamp(lim.min(Vec3::ZERO), lim.max(Vec3::ZERO))
}

#[allow(clippy::too_many_arguments)]
pub fn adhesion_accel(
    c: &CarvingTuning,
    sp: &SurfaceProperties,
    normal_y: f32,
    adhesion_mod: f32,
    kin34: f32,
    stable: bool,
    pitch: f32,
    bail: bool,
    have_made_first_contact: bool,
) -> f32 {
    if bail || !have_made_first_contact {
        return 0.0;
    }
    let t = (-kin34 / sp.adhesion_dist_above).clamp(0.0, 1.0);
    let mut m = normal_y.max(0.0) * sp.adhesion_accel * adhesion_mod * (1.0 - t);
    if !stable {
        m *= c.balance_surface_adhesion_modifier_curve.eval(wrap_pi(pitch).abs());
    }
    m
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BalanceView {
    pub pitch_balance: f32,
    pub pitch_balance_vel: f32,
    pub bail_on_landing: bool,
    pub pitch: f32,
    pub stable: bool,
}

pub struct LandingInputs<'a> {
    pub c: &'a CarvingTuning,
    pub sp: &'a SurfaceProperties,
    pub st: &'a CarvingState,
    pub balance: BalanceView,
    pub v_n: f32,
    pub g_n: f32,
    pub normal_y: f32,
    pub kin30: f32,
    pub kin34: f32,
    pub f210: f32,
    pub uls: bool,
    pub pitch_balance_vel: f32,
    pub adhesion_mod: f32,
    pub dt: f32,
}

pub fn landing_and_adhesion(i: &LandingInputs) -> (f32, f32, f32) {
    let (c, b, st) = (i.c, &i.balance, i.st);
    let mut above = i.kin34 < 0.0;
    let mut above0 = above;
    let mut e = -i.f210;
    let mut rem = i.kin34;
    let mut t = i.dt;
    let mut dn = 0.0;
    let mut v = i.v_n;
    let mut v_start = v;
    for iter in 0..2 {
        let a_adh = if above {
            adhesion_accel(
                c,
                i.sp,
                i.normal_y,
                i.adhesion_mod,
                i.kin34,
                b.stable,
                b.pitch,
                b.bail_on_landing,
                st.have_made_first_contact,
            )
        } else {
            0.0
        };
        if v < 0.0 && st.on_ground_ticks <= c.min_landing_impact_velocity_ticks {
            v = v.min(-c.min_landing_impact_velocity);
        }
        v_start = v;
        v += i.g_n * t;
        let v_free = v;
        let absorb = !i.uls
            || (b.pitch_balance.abs() > c.pitch_balance_stable_threshold
                && fsel_sign(b.pitch_balance) == fsel_sign(i.pitch_balance_vel)
                && !b.bail_on_landing);
        let mut d;
        if v < c.landing_absorption_threshold && absorb {
            v *= c.landing_velocity_decay_curve.eval(st.on_ground_ticks);
            d = v * t;
        } else {
            let k2 =
                if b.bail_on_landing { 1.0 } else { c.landing_riding_height_spring_stiffness_curve.eval(st.uls_ticks) };
            if above || k2 <= 0.0 {
                d = (v + a_adh) * t;
            } else {
                let k = k2.sqrt();
                let ex = (-k * t).exp();
                let e0 = e + i.kin30;
                let cc = v + k * e0;
                let vs = (v - k * cc * t) * ex;
                e = (e0 + cc * t) * ex - i.kin30;
                d = e + i.f210;
                if vs >= v_free {
                    v = vs;
                } else {
                    d = v_free * t;
                    v = v_free;
                }
            }
        }
        let now_above = rem - d < 0.0;
        if now_above != above0 && iter == 0 {
            let f = if d.abs() > 0.0 { (rem / d).abs() } else { 0.0 };
            d = rem;
            e += rem;
            rem = 0.0;
            v = v_start + i.g_n * f * t;
            t *= 1.0 - f;
            above = now_above;
            above0 = now_above;
            dn += d;
            continue;
        }
        if now_above != above0 {
            d = rem;
        }
        dn += d;
        break;
    }
    let nf = ((v - v_start) * c.normal_force_multiplier / i.dt).max(0.0);
    (dn, v, nf)
}

pub struct GroundInputs<'a> {
    pub t: &'a Tuning,
    pub r: &'a ResolvedCharacterProperties,
    pub sp: &'a SurfaceProperties,
    pub st: &'a CarvingState,
    pub balance: BalanceView,
    pub q: Quat,
    pub n: Vec3,
    pub contact: bool,
    pub v_world: Vec3,
    pub kin: &'a Kinematics,
    pub kin30: f32,
    pub kin34: f32,
    pub f210: f32,
    pub f218: f32,
    pub stick_y: f32,
    pub brake: f32,
    pub slip_dig: f32,
    pub uber: bool,
    pub board_press: bool,
    pub stopped: bool,
    pub height_above_lowest: f32,
    pub boost: BoostThrust,
    pub grind: GrindGround,
    pub wind: Vec3,
    pub ice_axe: crate::ice_axe::IceAxeFrame,
    pub avalanche_drag: f32,
    pub avalanche_friction: f32,
    pub dt: f32,
}

pub fn ice_axe_surface(sp: &SurfaceProperties, ice: &crate::ice_axe::IceAxeFrame) -> SurfaceProperties {
    if !ice.active {
        return *sp;
    }
    let (k, e) = (ice.dig_modifier, ice.effectiveness);
    let mix = |base: f32, lo: f32, hi: f32| {
        let v = (hi - lo) * k + lo;
        (v - base) * e + base
    };
    SurfaceProperties {
        adhesion_accel: mix(sp.adhesion_accel, sp.adhesion_accel_ice_axe_low, sp.adhesion_accel_ice_axe_high),
        dig_lean: mix(sp.dig_lean, sp.dig_lean_ice_axe_low, sp.dig_lean_ice_axe_high),
        friction: mix(sp.friction, sp.friction_ice_axe_low, sp.friction_ice_axe_high),
        friction_brake: mix(sp.friction_brake, sp.friction_brake_ice_axe_low, sp.friction_brake_ice_axe_high),
        ..*sp
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GrindGround {
    pub state: u32,
    pub blend: f32,
    pub or_press_blend: f32,
    pub thrust: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BoostThrust {
    pub is_boosting: bool,
    pub modifier: f32,
    pub accel_low: f32,
    pub accel_high: f32,
    pub max_speed: f32,
}

impl BoostThrust {
    pub fn accel(&self, c: &CarvingTuning, speed: f32, h: f32, forward_sign: f32) -> f32 {
        if !self.is_boosting {
            return 0.0;
        }
        let hb = (1.0 - (h - c.height_to_allow_full_boost) / c.height_to_allow_partial_boost).clamp(0.0, 1.0);
        let r = (speed / self.max_speed).clamp(0.0, 1.0);
        let k = (1.0 - r * r) * self.modifier.max(0.0);
        (self.accel_low + (self.accel_high - self.accel_low) * k) * forward_sign * hb
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GroundOutput {
    pub dp: Vec3,
    pub v: Vec3,
    pub a: Vec3,
    pub normal_force: f32,
    pub pitch_balance_vel: f32,
    pub use_leg_spring: bool,
    pub grounded: bool,
    pub is_uphill: Option<bool>,
    pub deceleration: Vec3,
}

pub fn integrate_ground(i: &GroundInputs) -> GroundOutput {
    let (t, r, c, st, kin) = (i.t, i.r, &i.t.carving, i.st, i.kin);
    let ice_sp = ice_axe_surface(i.sp, &i.ice_axe);
    let sp = &ice_sp;
    let dt = i.dt;
    let (q, q1) = (i.q, i.q.conjugate());
    let nfm = c.normal_force_multiplier;
    let g_n_world = (q1 * r.g_ground).y;
    let v_local = q1 * i.v_world;
    let tuck = if i.board_press { i.stick_y } else { i.stick_y * (1.0 - i.grind.or_press_blend) };
    let kh = (r.k_h_ground + (r.k_h_ground_tuck - r.k_h_ground) * tuck) * i.avalanche_drag;
    let kv = r.k_v_ground + (r.k_v_ground_tuck - r.k_v_ground) * tuck;
    let mut out = GroundOutput {
        pitch_balance_vel: i.balance.pitch_balance_vel,
        use_leg_spring: st.use_leg_spring || v_local.y > c.landing_absorption_threshold || i.balance.bail_on_landing,
        ..GroundOutput::default()
    };
    if !i.contact {
        let a = r.g_ground + drag_accel(kh, kv, i.v_world, i.wind, dt);
        out.a = a;
        out.v = if i.stopped { Vec3::ZERO } else { i.v_world + a * dt };
        out.dp = out.v * dt;
        return out;
    }
    out.grounded = i.kin34 >= i.f218;
    let mut v_l = v_local;
    let mut g_l = q1 * r.g_ground;
    let (mut pos_off_n, mut landed, mut vn_final, mut nf) = (0.0, false, 0.0, 0.0);
    if st.have_made_first_contact && -i.kin34 < sp.adhesion_dist_above {
        let li = LandingInputs {
            c,
            sp,
            st,
            balance: i.balance,
            v_n: v_local.y,
            g_n: g_n_world,
            normal_y: i.n.y,
            kin30: i.kin30,
            kin34: i.kin34,
            f210: i.f210,
            uls: out.use_leg_spring,
            pitch_balance_vel: out.pitch_balance_vel,
            adhesion_mod: if i.grind.state != 0 { 1.0 + 300.0 * i.grind.blend } else { 1.0 },
            dt,
        };
        (pos_off_n, vn_final, nf) = landing_and_adhesion(&li);
        v_l.y = 0.0;
        g_l.y = 0.0;
        landed = true;
    }
    let speed = i.v_world.length();
    let mut a = g_l;
    if out.grounded {
        let h = i.height_above_lowest;
        out.is_uphill = Some(h > c.height_considered_uphill);
        if !i.uber {
            if !i.board_press {
                let hu = (1.0 - (h - c.height_to_allow_full_unstuck_assist) / c.height_to_allow_partial_unstuck_assist)
                    .clamp(0.1, 1.0);
                let ratio = (speed / r.max_unstuck_speed).clamp(0.0, 1.0);
                let k = (1.0 - ratio * ratio) * i.stick_y.max(0.0);
                a.x += (r.unstuck_accel_low + (r.unstuck_accel_high - r.unstuck_accel_low) * k) * hu * kin.fwd;
            }
            let half = if i.board_press { 0.5 } else { 1.0 };
            a.x += i.boost.accel(c, speed, h, kin.fwd) * half;
        }
        if i.grind.state == 2 {
            a.x += i.grind.thrust * kin.fwd;
        }
    }
    let mut a_drag = q1 * drag_accel(kh, kv, q * v_l, i.wind, dt);
    if out.grounded {
        a_drag.y = 0.0;
    }
    a += a_drag;
    let v_new = v_l + a * dt;
    let raw_fric = if out.grounded {
        let grind_scale = if i.grind.state == 0 && !i.uber { 1.0 - i.grind.blend } else { 0.0 };
        let ice_slip = i.ice_axe.active.then(|| i.ice_axe.steering_value.sin());
        friction_accel_grind(t, sp, v_new, kin.lean, i.slip_dig, i.n.y, i.brake, i.uber, grind_scale, ice_slip)
            * i.avalanche_friction
    } else {
        Vec3::ZERO
    };
    let a_fric = clamp_not_reversing(raw_fric, v_new, dt);
    out.deceleration = q * Vec3::new((a + a_fric).x.min(0.0), 0.0, raw_fric.z);
    out.a = q * (a + a_fric);
    out.v = if i.stopped {
        landed = false;
        Vec3::ZERO
    } else {
        q * v_l + out.a * dt
    };
    out.dp = q * Vec3::new(0.0, pos_off_n, 0.0) + out.v * dt;
    floor_and_arrival(i, &mut out, &mut landed, &mut vn_final, &mut nf, g_n_world, nfm);
    out.normal_force = nf;
    out
}

fn floor_and_arrival(
    i: &GroundInputs,
    out: &mut GroundOutput,
    landed: &mut bool,
    vn_final: &mut f32,
    nf: &mut f32,
    g_n: f32,
    nfm: f32,
) {
    let (c, kin, dt) = (&i.t.carving, i.kin, i.dt);
    let (q, q1) = (i.q, i.q.conjugate());
    let v_local_start = q1 * i.v_world;
    let dpl = q1 * out.dp;
    let upright = if i.uber { 0.0 } else { 1.0 };
    let rad = c.collision_radius_base + c.collision_radius_upright_offset * upright * kin.lean.abs().cos();
    let mut y_ref = dpl;
    if i.f210 - dpl.y > -rad {
        let corr = i.f210 - dpl.y + rad;
        out.dp += q * Vec3::new(0.0, corr, 0.0);
        let vl = q1 * out.v;
        out.v = q * Vec3::new(vl.x, 0.0, vl.z);
        out.a = q * Vec3::new(0.0, -vl.y, 0.0);
        if out.pitch_balance_vel.abs() <= 1.192_092_9e-7 && !out.use_leg_spring {
            let vn = if *landed { vn_final.abs() } else { v_local_start.y.abs() };
            out.pitch_balance_vel = 2.0 * kin.fwd * fsel_sign(i.balance.pitch) * vn;
            out.use_leg_spring = true;
        }
        *nf += (nfm / dt * vl.y).abs() + g_n.abs();
        *vn_final = 0.0;
        y_ref = Vec3::new(0.0, corr, 0.0);
    }
    if !i.st.have_made_first_contact && i.kin34 - y_ref.y >= i.f218 && y_ref.y.abs() > 0.0 {
        let t_rem = (y_ref.y - i.kin34) / y_ref.y * dt;
        let mut vn = v_local_start.y + g_n * dt;
        let mut extra = 0.0;
        if vn < 0.0 {
            vn *= c.landing_velocity_decay_curve.eval(i.st.on_ground_ticks);
            extra = t_rem * vn;
        }
        *nf += ((vn - v_local_start.y) * nfm / dt).abs();
        out.dp = q * Vec3::new(y_ref.x, i.kin34 + extra, y_ref.z);
        let vl = q1 * out.v;
        out.v = q * Vec3::new(vl.x, 0.0, vl.z);
        out.a = q * Vec3::new(0.0, -vl.y, 0.0);
        if !*landed {
            *landed = true;
            *vn_final = vn;
        }
    }
    if *landed {
        let mut vl = q1 * out.v;
        vl.y = *vn_final;
        out.v = q * vl;
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Spray {
    pub carving: Vec3,
    pub ice: Vec3,
    pub velocity: Vec3,
    pub impact: Vec3,
    pub normal_acceleration: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SprayInputs {
    pub contact: bool,
    pub first_contact: bool,
    pub stopped: bool,
    pub board_x: Vec3,
    pub board_z: Vec3,
    pub n: Vec3,
    pub velocity: Vec3,
    pub deceleration: Vec3,
    pub new_velocity: Vec3,
    pub normal_force: f32,
    pub snow_coverage: f32,
    pub ice_coverage: f32,
    pub responsiveness: f32,
    pub dt: f32,
}

const SPRAY_EPS: f32 = 1.192_092_9e-7;

fn all_tiny(v: Vec3) -> bool {
    v.abs().max_element() <= SPRAY_EPS
}

pub fn spray_source(i: &SprayInputs) -> Vec3 {
    if i.stopped {
        return Vec3::ZERO;
    }
    let p = i.board_z - i.n * i.board_z.dot(i.n);
    if all_tiny(p) {
        return Vec3::ZERO;
    }
    let p = p.normalize();
    let g = i.deceleration.length();
    let g_dir = if g > 0.0 { i.deceleration / g } else { Vec3::ZERO };
    let u = g_dir * g.min(i.velocity.length() / i.dt);
    let s = p * (2.0 * u.dot(p)) - u;
    let mut h = i.board_x.cross(s.cross(i.n));
    if h.dot(i.n) < 0.0 {
        h = -h;
    }
    if all_tiny(s) || all_tiny(h) {
        return Vec3::ZERO;
    }
    let (sn, hn) = (s.normalize(), h.normalize());
    let d = sn.dot(hn);
    if (d - 1.0).abs() <= 1.525_879e-5 {
        return s;
    }
    if (d + 1.0).abs() <= 1.525_879e-5 {
        let a = sn.abs();
        let e = if a.y.min(a.z) > a.x {
            Vec3::X
        } else if a.x.min(a.z) > a.y {
            Vec3::Y
        } else {
            Vec3::Z
        };
        return Quat::from_axis_angle(sn.cross(e).normalize(), std::f32::consts::FRAC_PI_2) * s;
    }
    (sn + hn).normalize() * s.length()
}

pub fn critically_damped_step(omega: f32, x: f32, v: f32, target: f32, t: f32) -> (f32, f32) {
    let e = (-omega * t).exp();
    let d = x - target;
    let c = v + omega * d;
    (target + (d + c * t) * e, (v - omega * c * t) * e)
}

pub fn spray_filter(st: &mut CarvingState, i: &SprayInputs) -> Spray {
    if !i.contact {
        return Spray { normal_acceleration: i.normal_force, ..Spray::default() };
    }
    let d = spray_source(i);
    let mag = d.length();
    let dir = if mag > 0.0 { d / mag } else { Vec3::ZERO };
    if i.first_contact || mag <= SPRAY_EPS {
        (st.spray_magnitude, st.spray_derivative) = (mag, 0.0);
    } else {
        let omega = i.responsiveness.max(0.0).sqrt();
        (st.spray_magnitude, st.spray_derivative) =
            critically_damped_step(omega, st.spray_magnitude, st.spray_derivative, mag, i.dt);
    }
    let spray = dir * st.spray_magnitude;
    let snow = i.snow_coverage.clamp(0.0, 1.0);
    let v_ground = i.new_velocity - i.n * i.new_velocity.dot(i.n);
    Spray {
        carving: spray * snow,
        ice: spray * i.ice_coverage.clamp(0.0, 1.0),
        velocity: -v_ground * snow,
        impact: i.n * (i.normal_force * snow),
        normal_acceleration: i.normal_force,
    }
}

#[cfg(test)]
mod tests;
