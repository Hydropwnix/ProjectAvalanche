use anyhow::{Context, Result};
use glam::{Quat, Vec3};

use crate::intercept::DragModel;
use crate::tuning::{AssetView, Bundle};

pub const NO_TIME: f32 = f32::MAX;
const EPS16: f32 = 1.525_878_9e-5;
const EPS23: f32 = 1.192_092_9e-7;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InteractionKind {
    #[default]
    Grind,
    Cargo,
    Swing,
    Handplant,
    IceAxeUnderflip,
}

impl InteractionKind {
    fn index(self) -> usize {
        self as usize
    }
}

const GRIND: [bool; 5] = [true, true, false, false, false];
const ACROSS: [bool; 5] = [false, true, false, true, true];
const MIRROR_INVERT: [bool; 5] = [false, false, true, false, true];
const LEFT_FORWARDS: [bool; 5] = [true, true, true, false, true];
const LEFT_FORWARDS_MIRRORED: [bool; 5] = [false, false, false, true, false];
const GOOFY_FLIPS_LEFT: [bool; 5] = [false, true, true, true, false];
const ALONG: [bool; 5] = [true, false, false, false, false];
const HOLD_FACTOR: [f32; 5] = [1.0, 1.0, 1.2, 1.1, 1.2];
const GRAB_OFFSET: [[u32; 3]; 5] = [
    [0x4084_51EC, 0xBE9B_22D1, 0xC034_BC6A],
    [0x409C_4189, 0x3F2A_7EFA, 0xC033_F7CF],
    [0xC034_6A7F, 0x3F85_A1CB, 0xC033_B646],
    [0xC0C9_0E56, 0x3ED7_0A3D, 0xC018_9375],
    [0xC07B_020C, 0xBE99_999A, 0xC032_F1AA],
];

pub fn grab_offset(kind: InteractionKind) -> Vec3 {
    let o = GRAB_OFFSET[kind.index()];
    Vec3::new(f32::from_bits(o[0]), f32::from_bits(o[1]), f32::from_bits(o[2]))
}

#[derive(Clone, Debug, PartialEq)]
pub struct InteractionTuning {
    pub abort_aerial_trick_time: f32,
    pub grind_dot: f32,
    pub cargo_dot: f32,
    pub swing_dot: f32,
    pub min_horizontal_velocity: f32,
    pub anim_speed_coeff: f32,
    pub min_time: f32,
    pub max_time: f32,
    pub min_separation_interact: f32,
    pub min_separation_avoid: f32,
    pub y_velocity_low: f32,
    pub y_velocity_high: f32,
    pub start_animation: f32,
    pub hold_helicopter: f32,
    pub min_shut_out: f32,
    pub cone_cos: f32,
}

fn f(v: &AssetView, name: &str) -> Result<f32> {
    v.f32(name).with_context(|| format!("Heli Interaction SceneOp.{name}"))
}

impl InteractionTuning {
    pub fn load(b: &Bundle) -> Result<Self> {
        let v = b.view("Heli Interaction SceneOp", "HeliInteractionSceneOpAsset")?;
        Ok(Self {
            abort_aerial_trick_time: f(&v, "BoarderAbortAerialTrickTime")?,
            grind_dot: f(&v, "GrindDotThreshold")?,
            cargo_dot: f(&v, "CargoDotThreshold")?,
            swing_dot: f(&v, "SwingDotThreshold")?,
            min_horizontal_velocity: f(&v, "MinHorizontalVelocityForInteract")?,
            anim_speed_coeff: f(&v, "BoarderAnimSpeedCoeff")?,
            min_time: f(&v, "MinTimeBeforeInteract")?,
            max_time: f(&v, "MaxTimeBeforeInteract")?,
            min_separation_interact: f(&v, "MinSeperationToInitiateInteract")?,
            min_separation_avoid: f(&v, "MinSeperationToInitiateAvoidance")?,
            y_velocity_low: f(&v, "YVelLowCutoffForInteract")?,
            y_velocity_high: f(&v, "YVelHighCutoffForInteract")?,
            start_animation: f(&v, "InteractTimeStartAnimation")?,
            hold_helicopter: f(&v, "InteractTimeHoldHelicopter")?,
            min_shut_out: f(&v, "MinShutOutTimeBeforeNewInteraction")?,
            cone_cos: f(&v, "InteractionConeCosTheta")?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeliInteractionBoard {
    pub prepare: bool,
    pub interact: bool,
    pub location: Vec3,
    pub orientation: Quat,
    pub time: f32,
    pub avoid: bool,
    pub avoid_direction: Vec3,
    pub avoid_distance: f32,
    pub helicopter_interaction: bool,
    pub mirror: bool,
    pub grind: bool,
    pub across: bool,
    pub goofy: bool,
    pub ice_axe: bool,
    pub track_finished: bool,
    pub anim_time_scale: f32,
}

impl Default for HeliInteractionBoard {
    fn default() -> Self {
        Self {
            prepare: false,
            interact: false,
            location: Vec3::ZERO,
            orientation: Quat::IDENTITY,
            time: 0.0,
            avoid: false,
            avoid_direction: Vec3::ZERO,
            avoid_distance: 0.0,
            helicopter_interaction: false,
            mirror: false,
            grind: false,
            across: false,
            goofy: false,
            ice_axe: false,
            track_finished: false,
            anim_time_scale: 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RiderHeliStates {
    pub pre_anim_velocity: Vec3,
    pub goofyness: bool,
    pub abort_aerial_trick: bool,
    pub helicopter_interaction: bool,
    pub mirror: bool,
    pub grind: bool,
    pub across: bool,
    pub arrive_left_forwards: bool,
    pub anim_time_scale: f32,
    pub heli_position: Vec3,
}

impl Default for RiderHeliStates {
    fn default() -> Self {
        Self {
            pre_anim_velocity: Vec3::ZERO,
            goofyness: false,
            abort_aerial_trick: false,
            helicopter_interaction: false,
            mirror: false,
            grind: false,
            across: false,
            arrive_left_forwards: false,
            anim_time_scale: 1.0,
            heli_position: Vec3::ZERO,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InteractionNode {
    pub kind: InteractionKind,
    pub side: bool,
    pub scale: f32,
}

impl Default for InteractionNode {
    fn default() -> Self {
        Self { kind: InteractionKind::Grind, side: false, scale: 1.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InteractionInputs {
    pub dt: f32,
    pub rewind_active: bool,
    pub rewind_terminating: bool,
    pub in_air: bool,
    pub can_interact: bool,
    pub aerial_trick_priority: bool,
    pub track_finished: bool,
    pub rstick_down: bool,
    pub goofy: bool,
    pub ice_axe: bool,
    pub boarder_position: Vec3,
    pub boarder_velocity: Vec3,
    pub drag: DragModel,
    pub heli_position: Vec3,
    pub heli_orientation: Quat,
    pub heli_velocity: Vec3,
    pub floor_joint: Vec3,
}

pub const GROUND_ORIENTATION: Quat =
    Quat::from_xyzw(0.0, f32::from_bits(0x3F35_04F3), 0.0, f32::from_bits(0x3F35_04F3));

pub fn flight_step(m: &DragModel, v: Vec3, dt: f32) -> Vec3 {
    let mut a = crate::rider::drag_accel(m.kh, m.kv, v, m.wind, dt);
    a.y -= m.g;
    a.mul_add(Vec3::splat(dt), v)
}

pub fn look_rotation(forward: Vec3, up: Vec3) -> Quat {
    let x = forward.normalize_or_zero();
    let z = x.cross(up).normalize_or_zero();
    Quat::from_mat3(&glam::Mat3::from_cols(x, z.cross(x), z)).normalize()
}

fn steps(ticks: f32, dt: f32) -> i32 {
    (ticks * (1.0 / dt)) as i32
}

pub fn avoid_check(t: &InteractionTuning, h: &mut HeliInteractionBoard, x: &InteractionInputs) -> bool {
    let n = steps(t.max_time, x.dt);
    let step_heli = x.heli_velocity * x.dt;
    let (mut v, mut pb, mut ph) = (x.boarder_velocity, x.boarder_position, x.heli_position);
    let mut found = false;
    for _ in 0..n.max(0) {
        v = flight_step(&x.drag, v, x.dt);
        ph += step_heli;
        pb = v.mul_add(Vec3::splat(x.dt), pb);
        let (to_pred, to_now) = (ph - pb, x.heli_position - pb);
        let (d_pred, d_now) = (to_pred.length(), to_now.length());
        let m = d_pred.min(d_now);
        if !found && t.min_separation_avoid > m {
            h.avoid_direction = if m == d_pred { to_pred } else { to_now };
            found = true;
        }
    }
    found
}

pub fn execute(
    t: &InteractionTuning,
    n: &mut InteractionNode,
    h: &mut HeliInteractionBoard,
    r: &mut RiderHeliStates,
    x: &InteractionInputs,
) -> Option<Vec3> {
    h.track_finished = x.track_finished;
    if x.rewind_active {
        return None;
    }
    if !(x.in_air && x.can_interact) {
        ground(t, h, r, x);
        return None;
    }
    let v = x.boarder_velocity;
    let v_hat = v * (1.0 / v.dot(v).sqrt());
    let side = v_hat.cross(Vec3::Y);
    let up = side.cross(v_hat);
    let heli_x = x.heli_orientation * Vec3::X;
    let (mut check_avoid, mut abort, mut pulse, mut interact) = (true, false, false, false);
    let (mut time, mut location, mut orientation) = (h.time, h.location, h.orientation);
    let mut velocity = None;
    if h.interact {
        (check_avoid, interact) = (false, true);
        let left = h.time - x.dt;
        time = left;
        let k = n.kind.index();
        if -(t.hold_helicopter * HOLD_FACTOR[k]) * (1.0 / n.scale) >= left {
            interact = false;
        } else if 0.0 >= left {
            if r.pre_anim_velocity.abs().max_element() <= EPS23 {
                r.pre_anim_velocity = v;
                let speed = v.dot(v).sqrt();
                n.scale = speed * t.anim_speed_coeff;
                (r.anim_time_scale, h.anim_time_scale) = (n.scale, n.scale);
                pulse = true;
            } else if x.rewind_terminating {
                (interact, check_avoid, time) = (false, true, -t.hold_helicopter);
            }
        } else {
            if x.track_finished {
                interact = false;
            } else if t.abort_aerial_trick_time > left {
                if x.aerial_trick_priority {
                    interact = false;
                } else {
                    abort = true;
                }
            }
            if interact {
                (location, orientation) = approach(t, n, r, x, left, v_hat, side, up);
            } else {
                (r.pre_anim_velocity, time) = (Vec3::ZERO, NO_TIME);
            }
        }
    } else if (h.time - NO_TIME).abs() > EPS16 {
        if r.pre_anim_velocity.abs().max_element() > EPS23 {
            velocity = Some(r.pre_anim_velocity);
            r.pre_anim_velocity = Vec3::ZERO;
        }
        let left = h.time - x.dt;
        time = if -t.min_shut_out > left { NO_TIME } else { left };
    } else {
        if not_above(v_hat.length(), EPS23) {
            return None;
        }
        let toward = (x.heli_position - x.boarder_position).dot(v);
        let horizontal = v.with_y(0.0).length();
        if x.dt.abs() > EPS23 && toward > t.cone_cos && horizontal > t.min_horizontal_velocity {
            let mut found = h.time;
            if let Some((when, at)) = select(t, x) {
                (found, location, interact, check_avoid) = (when, at, true, false);
            }
            n.side = side.dot(heli_x) > 0.0;
            r.goofyness = x.goofy;
            let d = v_hat.dot(heli_x);
            n.kind = if x.ice_axe {
                InteractionKind::IceAxeUnderflip
            } else if d > t.grind_dot {
                InteractionKind::Grind
            } else if d > t.cargo_dot {
                InteractionKind::Cargo
            } else if d > t.swing_dot {
                InteractionKind::Swing
            } else {
                InteractionKind::Handplant
            };
            time = found - t.start_animation;
        }
    }
    write_out(t, n, h, r, x, (check_avoid, abort, pulse, interact), (time, location, orientation));
    velocity
}

fn select(t: &InteractionTuning, x: &InteractionInputs) -> Option<(f32, Vec3)> {
    let dt = x.dt;
    let (mut v, mut pb, mut ph) = (x.boarder_velocity, x.boarder_position, x.heli_position);
    let n1 = steps(t.min_time, dt);
    if n1 > 0 {
        let step_heli = x.heli_velocity * dt;
        for _ in 0..n1 {
            v = flight_step(&x.drag, v, dt);
            ph += step_heli;
            pb = v.mul_add(Vec3::splat(dt), pb);
        }
    }
    let n2 = steps(t.max_time - t.min_time, dt);
    let (mut clock, mut best, mut found) = (t.min_time, f32::MAX, None);
    for _ in 0..n2.max(0) {
        if not_above(v.y, t.y_velocity_low) {
            break;
        }
        clock += dt;
        v = flight_step(&x.drag, v, dt);
        pb = v.mul_add(Vec3::splat(dt), pb);
        ph = x.heli_velocity.mul_add(Vec3::splat(dt), ph);
        let (d_pred, d_now) = ((ph - pb).length(), (x.heli_position - pb).length());
        if best > d_pred || best > d_now {
            best = d_pred.min(d_now);
            if t.min_separation_interact > best && v.y > t.y_velocity_low && t.y_velocity_high > v.y {
                found = Some((clock, pb));
            }
        }
    }
    found
}

#[allow(clippy::too_many_arguments)]
fn approach(
    t: &InteractionTuning,
    n: &InteractionNode,
    r: &RiderHeliStates,
    x: &InteractionInputs,
    left: f32,
    v_hat: Vec3,
    side: Vec3,
    up: Vec3,
) -> (Vec3, Quat) {
    let (mut v, mut p) = (x.boarder_velocity, x.boarder_position);
    for _ in 0..steps(left + t.start_animation, x.dt).max(0) {
        v = flight_step(&x.drag, v, x.dt);
        p = v.mul_add(Vec3::splat(x.dt), p);
    }
    let k = n.kind.index();
    let mut z = if n.side { 1.0 } else { -1.0 };
    if ALONG[k] {
        z = if n.side == r.goofyness { -1.0 } else { 1.0 };
    }
    let offset =
        (x.heli_position - x.floor_joint) + x.heli_orientation * (grab_offset(n.kind) * Vec3::new(1.0, 1.0, z));
    let forward = if ALONG[k] {
        v_hat
    } else if n.side {
        side
    } else {
        -side
    };
    (p + offset, look_rotation(forward, up))
}

fn not_above(a: f32, b: f32) -> bool {
    a.partial_cmp(&b) != Some(std::cmp::Ordering::Greater)
}

fn not_at_least(a: f32, b: f32) -> bool {
    matches!(a.partial_cmp(&b), None | Some(std::cmp::Ordering::Less))
}

fn ground(t: &InteractionTuning, h: &mut HeliInteractionBoard, r: &mut RiderHeliStates, x: &InteractionInputs) {
    let mut time = h.time;
    if (time - NO_TIME).abs() > EPS16 {
        time -= x.dt;
        if not_at_least(time, -t.min_shut_out) {
            time = NO_TIME;
        }
    }
    h.prepare = x.rstick_down;
    h.interact = false;
    h.time = time;
    h.location = Vec3::ZERO;
    h.orientation = GROUND_ORIENTATION;
    r.abort_aerial_trick = false;
    (r.helicopter_interaction, h.helicopter_interaction) = (false, false);
    (r.mirror, r.grind, r.across) = (false, false, false);
    r.pre_anim_velocity = Vec3::ZERO;
    h.avoid = avoid_check(t, h, x);
    h.avoid_distance = (x.boarder_position - x.heli_position).length();
    r.heli_position = x.heli_position;
}

fn write_out(
    t: &InteractionTuning,
    n: &InteractionNode,
    h: &mut HeliInteractionBoard,
    r: &mut RiderHeliStates,
    x: &InteractionInputs,
    flags: (bool, bool, bool, bool),
    target: (f32, Vec3, Quat),
) {
    let (check_avoid, abort, pulse, interact) = flags;
    h.prepare = false;
    h.interact = interact;
    (h.time, h.location, h.orientation) = target;
    r.abort_aerial_trick = abort;
    (r.helicopter_interaction, h.helicopter_interaction) = (pulse, pulse);
    let k = n.kind.index();
    let mirror = if MIRROR_INVERT[k] { !n.side } else { n.side };
    let mut left = if mirror { LEFT_FORWARDS_MIRRORED[k] } else { LEFT_FORWARDS[k] };
    if GOOFY_FLIPS_LEFT[k] && r.goofyness {
        left = !left;
    }
    (r.mirror, h.mirror) = (mirror, mirror);
    (r.grind, h.grind) = (GRIND[k], GRIND[k]);
    (r.across, h.across) = (ACROSS[k], ACROSS[k]);
    r.arrive_left_forwards = left;
    h.goofy = r.goofyness;
    h.ice_axe = x.ice_axe;
    h.avoid = check_avoid && avoid_check(t, h, x);
    h.avoid_distance = (x.boarder_position - x.heli_position).length();
    r.heli_position = x.heli_position;
}
