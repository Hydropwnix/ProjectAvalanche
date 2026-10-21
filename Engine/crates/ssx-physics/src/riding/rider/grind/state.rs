#![allow(clippy::neg_cmp_op_on_partial_ord)]

use glam::{Quat, Vec3};

use crate::collision::{CollisionWorld, SegmentQuery};

pub const RAIL_MASK: u32 = !0x1004;
pub const NON_RAIL_CLASSES: u32 = !0x1004;
pub const OBSTACLE_CLASSES: u32 = 0x802;

pub fn rail_at(c: &CollisionWorld, p: Vec3) -> bool {
    let q = SegmentQuery {
        start: p + Vec3::Y * 0.1,
        end: p - Vec3::Y * 0.1,
        radius: 0.5,
        two_sided: true,
        class_mask: RAIL_MASK,
    };
    c.segment_query(&q).is_some()
}

pub fn clear_to(c: &CollisionWorld, traj: Vec3, p: Vec3, d: Vec3, depth: f32, classes: u32) -> bool {
    c.segment_query(&SegmentQuery::line(traj, p + d * depth, !classes)).is_none()
}

pub fn landing_rail(c: &CollisionWorld, ground: Option<Vec3>, traj: Vec3, depth: f32) -> bool {
    ground.is_some_and(|p| rail_at(c, p) && clear_to(c, traj, p, Vec3::Y, depth, NON_RAIL_CLASSES))
}

pub fn normalize0(v: Vec3) -> Vec3 {
    let l = v.length();
    if l > 0.0 { v / l } else { Vec3::ZERO }
}

pub fn angle_between(a: Vec3, b: Vec3) -> f32 {
    let c = (a / a.length()).dot(b / b.length());
    if c.is_nan() { f32::NAN } else { c.clamp(-1.0, 1.0).acos() }
}

pub fn turn_towards(from: Vec3, to: Vec3, angle: f32) -> Vec3 {
    let axis = from.cross(to);
    let axis = axis / axis.length();
    Quat::from_axis_angle(axis, angle) * from
}

pub fn on_plane(v: Vec3, n: Vec3) -> Vec3 {
    v - n * v.dot(n)
}

pub fn step_point(target: Vec3, joint: Vec3, traj: Vec3, n: Vec3, dp: Vec3, v: Vec3, dt: f32) -> Vec3 {
    joint + normalize0(target - joint) * v.length() * dt + (traj - joint) + n * dp.dot(n)
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PathFrame {
    pub valid: bool,
    pub dir_a: Vec3,
    pub pos_a: Vec3,
    pub peak_a: f32,
    pub ridge_a: f32,
    pub dir_b: Vec3,
    pub pos_b: Vec3,
    pub peak_b: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GrindCarving {
    pub state: u32,
    pub ticks: f32,
    pub blend: f32,
    pub grind_or_press_blend: f32,
    pub press: super::Press,
    pub prev_input: bool,
    pub veering_angle: f32,
    pub veering_vel: f32,
    pub auto_grind: bool,
    pub auto_grind_clearing: bool,
    pub super_grind: bool,
    pub is_grinding: bool,
    pub spin: f32,
    pub spin_vel: f32,
    pub board_angle: f32,
    pub board_angle_init: bool,
}

pub struct MachineInputs<'a> {
    pub c: &'a CollisionWorld,
    pub t: &'a super::GrindTuning,
    pub sa: &'a super::surface_analysis::SurfaceAnalysis,
    pub dt: f32,
    pub traj: Vec3,
    pub joint: Vec3,
    pub n: Vec3,
    pub f210: f32,
    pub path: PathFrame,
    pub button: bool,
    pub stick_dir: Vec3,
    pub tail_dir: Option<Vec3>,
    pub depth: f32,
    pub follow_buried: bool,
    pub wiping_out: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MachineDebug {
    pub rail: bool,
    pub rail_point: Vec3,
    pub super_tuning: bool,
    pub selected: Vec3,
    pub selected_ridge: f32,
    pub target: Vec3,
    pub ridge_a: f32,
    pub speed: f32,
    pub path_angle: f32,
    pub max_path_angle: f32,
    pub steep_a: bool,
    pub out_of_window: bool,
    pub clear: bool,
    pub in_cone: bool,
    pub under: bool,
    pub exit: Option<GrindExit>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum GrindExit {
    WipingOut,
    Ridge,
    Speed,
    Angle,
    Steep,
    Button,
    Buried,
}

impl GrindExit {
    pub const ALL: [GrindExit; 7] = [
        GrindExit::WipingOut,
        GrindExit::Ridge,
        GrindExit::Speed,
        GrindExit::Angle,
        GrindExit::Steep,
        GrindExit::Button,
        GrindExit::Buried,
    ];

    pub fn name(self) -> &'static str {
        match self {
            GrindExit::WipingOut => "wipeout",
            GrindExit::Ridge => "ridge-end",
            GrindExit::Speed => "speed",
            GrindExit::Angle => "angle",
            GrindExit::Steep => "steep",
            GrindExit::Button => "button",
            GrindExit::Buried => "buried",
        }
    }
}

pub fn state_machine(g: &mut GrindCarving, i: &MachineInputs, dp: &mut Vec3, v: &mut Vec3) -> MachineDebug {
    let (dt, n, joint, traj) = (i.dt, i.n, i.joint, i.traj);
    let speed = v.length();
    let v_out = *v;
    let dp_in = *dp;
    let p = &i.path;
    let (pos_a, dir_a) = if p.valid { (p.pos_a, p.dir_a) } else { (joint, v_out) };
    let (mut pos_sel, mut dir_sel, mut ridge_sel, mut ridge_a) =
        if p.valid { (p.pos_b, p.dir_b, p.peak_b, p.ridge_a) } else { (joint, v_out, 0.0, 0.0) };
    let mut clear = p.valid && clear_to(i.c, traj, pos_sel, n, i.depth, OBSTACLE_CLASSES);
    let v_p = on_plane(v_out, n);
    let mut proj_a = on_plane(dir_a, n);
    let mut proj_sel = on_plane(dir_sel, n);
    let mut rail_point = joint;
    let rail = if rail_at(i.c, pos_a) {
        rail_point = pos_a;
        true
    } else if rail_at(i.c, pos_sel) {
        rail_point = pos_sel;
        true
    } else {
        false
    };
    let not_buried = rail && clear_to(i.c, traj, rail_point, n, i.depth, NON_RAIL_CLASSES);
    let rail_ok = if i.follow_buried { rail } else { not_buried };
    g.auto_grind_clearing = g.auto_grind_clearing && rail_ok;
    g.super_grind = rail_ok;
    if g.auto_grind && (!rail_ok || i.button) {
        g.auto_grind = false;
    }
    let gp = i.sa.grid(traj);
    let (cx, cy) = ((gp.x + 0.5).floor().max(0.0) as usize, (gp.y + 0.5).floor().max(0.0) as usize);
    let under = f32::from(i.sa.height(cx.min(255), cy.min(255))) > gp.z;
    let tun = if rail { &i.t.super_ } else { &i.t.regular };
    let max_ang = tun.max_active_angular_vel_on_entry
        + (tun.max_active_angular_vel_steady_state - tun.max_active_angular_vel_on_entry) * g.blend;
    if tun.min_ridgiprocity_active >= ridge_sel && ridge_a > tun.min_ridgiprocity_active {
        (pos_sel, ridge_sel, dir_sel, proj_sel) = (pos_a, ridge_a, dir_a, proj_a);
        clear = clear_to(i.c, traj, pos_sel, n, i.depth, OBSTACLE_CLASSES);
    } else if g.state == 2 && tun.min_ridgiprocity_active >= ridge_a && ridge_sel > tun.min_ridgiprocity_active {
        (proj_a, ridge_a) = (proj_sel, ridge_sel);
    }
    let off_a = pos_a - joint;
    let h_a = off_a.dot(n);
    let lat_a = on_plane(off_a, n).length();
    let steep_a = h_a > lat_a * tun.max_path_height_rise || lat_a * -tun.max_path_height_fall > h_a;
    let h_sel = (pos_sel - joint).dot(n);
    let out_of_window = h_sel > tun.max_path_height_rise || -tun.max_path_height_fall > h_sel;
    let tail = i.tail_dir.unwrap_or(v_out);
    let in_cone = tun.max_tail_angle > angle_between(on_plane(tail, n), on_plane(pos_sel - joint, n));
    let auto_allowed = rail && !g.auto_grind_clearing;
    let (path_angle, max_path_angle) = (angle_between(v_p, proj_a), 2.0 * max_ang);
    let mut exit_cause = None;
    match g.state {
        0 => {
            if !i.wiping_out
                && ridge_sel > tun.min_ridgiprocity_start
                && speed > tun.min_speed_threshold_start
                && in_cone
                && !under
            {
                if g.auto_grind && rail && not_buried {
                    g.state = 1;
                } else if (i.button || auto_allowed)
                    && tun.max_path_angle > angle_between(v_p, proj_sel)
                    && !out_of_window
                    && clear
                {
                    g.auto_grind = rail;
                    g.state = 1;
                }
            }
        }
        1 => {
            let keep = (i.button || auto_allowed)
                && !i.wiping_out
                && ridge_sel > tun.min_ridgiprocity_active
                && speed > tun.min_speed_threshold_active
                && v_p.dot(proj_a) > 0.0
                && !out_of_window
                && clear
                && in_cone
                && !under;
            if !keep {
                g.state = 0;
            }
        }
        2 => {
            exit_cause = if i.wiping_out {
                Some(GrindExit::WipingOut)
            } else if !(ridge_a > tun.min_ridgiprocity_active) {
                Some(GrindExit::Ridge)
            } else if !(speed > tun.min_speed_threshold_active) {
                Some(GrindExit::Speed)
            } else if !(max_path_angle > path_angle) {
                Some(GrindExit::Angle)
            } else if steep_a {
                Some(GrindExit::Steep)
            } else if g.prev_input && !i.button {
                Some(GrindExit::Button)
            } else if !rail || not_buried {
                None
            } else {
                Some(GrindExit::Buried)
            };
            g.state = match exit_cause {
                Some(GrindExit::Button) => 3,
                Some(GrindExit::Buried) => 1,
                Some(_) => 0,
                None => 2,
            };
        }
        _ => {}
    }
    let target = joint + normalize0(pos_a - joint) * speed * dt;
    let mut dbg = MachineDebug {
        rail,
        rail_point,
        super_tuning: rail,
        selected: pos_sel,
        selected_ridge: ridge_sel,
        target,
        ridge_a,
        speed,
        path_angle,
        max_path_angle,
        steep_a,
        out_of_window,
        clear,
        in_cone,
        under,
        exit: exit_cause,
    };
    match g.state {
        1 => {
            let a = Approach { pos_sel, dir_sel, speed, button: i.button };
            ridge_a = approach(i, tun, &a, dp, v, dp_in, v_out);
        }
        2 => dbg.target = grind(i, tun, target, speed, dp, v, dp_in, v_out),
        3 => exit(i, tun, target, proj_a, speed, dp, v, dp_in, v_out),
        _ => {}
    }
    if g.state == 3 {
        g.state = 0;
        g.auto_grind_clearing = true;
    } else if g.state == 1 && ridge_a > tun.min_ridgiprocity_start && (!rail || not_buried) {
        g.state = 2;
    }
    dbg
}

struct Approach {
    pos_sel: Vec3,
    dir_sel: Vec3,
    speed: f32,
    button: bool,
}

fn approach(
    i: &MachineInputs,
    tun: &super::GrindProperties,
    a: &Approach,
    dp: &mut Vec3,
    v: &mut Vec3,
    dp_in: Vec3,
    v_in: Vec3,
) -> f32 {
    let (n, dt) = (i.n, i.dt);
    let floor = i.traj + n * i.f210;
    let d = a.pos_sel - floor;
    let lat = on_plane(d, n);
    let goal = floor + lat + n * (d - lat).dot(n).max(0.0);
    let p = step_point(goal, i.joint, i.traj, n, dp_in, v_in, dt);
    let mut dir = normalize0(p - i.traj) * a.speed;
    let lim = tun.max_approach_angular_vel * dt;
    if angle_between(a.dir_sel, dir) > lim {
        dir = normalize0(turn_towards(a.dir_sel, dir, lim)) * a.speed;
    }
    let lim2 = if a.button { tun.max_approach_angular_vel } else { tun.max_approach_angular_vel_gentle } * dt;
    if angle_between(v_in, dir) > lim2 {
        dir = normalize0(turn_towards(v_in, dir, lim2)) * a.speed;
    }
    *dp = dir * dt;
    *v = dir;
    super::path::max_ridge_on_segment(i.sa, i.joint, i.joint + *dp * 2.2).1
}

#[allow(clippy::too_many_arguments)]
fn grind(
    i: &MachineInputs,
    tun: &super::GrindProperties,
    target: Vec3,
    speed: f32,
    dp: &mut Vec3,
    v: &mut Vec3,
    dp_in: Vec3,
    v_in: Vec3,
) -> Vec3 {
    let (n, dt) = (i.n, i.dt);
    let e = (-tun.min_target_speed_gain * dt).exp();
    let speed2 = speed.max(tun.min_target_speed + (speed - tun.min_target_speed) * e);
    let mut tgt = target;
    if let Some(h) = i.c.segment_query(&SegmentQuery::line(target + n * i.depth, target, 0)) {
        tgt = h.point;
    }
    let p = step_point(tgt, i.joint, i.traj, n, dp_in, v_in, dt);
    let step = normalize0(p - i.traj) * speed2 * dt;
    let diff = dp_in - step;
    let dn = n * diff.dot(n);
    let dl = diff - dn;
    let clamp = |x: Vec3| normalize0(x) * x.length().min(0.2);
    *dp = (clamp(dl) + clamp(dn)) * 0.5 + step;
    *v = *dp * (1.0 / dt);
    tgt
}

#[allow(clippy::too_many_arguments)]
fn exit(
    i: &MachineInputs,
    tun: &super::GrindProperties,
    target: Vec3,
    proj_a: Vec3,
    speed: f32,
    dp: &mut Vec3,
    v: &mut Vec3,
    dp_in: Vec3,
    v_in: Vec3,
) {
    if i.stick_dir.abs().max_element() <= 1.192_092_9e-7 {
        return;
    }
    let (n, dt) = (i.n, i.dt);
    let p = step_point(target, i.joint, i.traj, n, dp_in, v_in, dt);
    *dp = normalize0(p - i.traj) * speed * dt;
    let stick = on_plane(i.stick_dir, n);
    let mut e = stick;
    if angle_between(proj_a, stick) > tun.max_joystick_exit_angle {
        e = turn_towards(proj_a, stick, tun.max_joystick_exit_angle);
    }
    *v = normalize0(e) * speed;
}

pub fn grind_update(
    g: &mut GrindCarving,
    i: &MachineInputs,
    have_made_first_contact: bool,
    dp: &mut Vec3,
    v: &mut Vec3,
) -> Option<MachineDebug> {
    if v.abs().max_element() <= 1.192_092_9e-7 || !(have_made_first_contact || g.auto_grind) {
        return None;
    }
    let mut dbg = None;
    if i.sa.cell(i.traj).is_some() {
        dbg = Some(state_machine(g, i, dp, v));
    } else {
        (g.auto_grind, g.super_grind, g.state) = (false, false, 0);
    }
    g.ticks = match g.state {
        2 => g.ticks + i.dt,
        3 => g.ticks,
        _ => 0.0,
    };
    dbg
}

pub fn grind_contact_normal(state: u32, contact: Vec3, path: &mut PathFrame, global_velocity: Vec3, n: Vec3) -> Vec3 {
    if state == 0 || !path.valid || !(path.peak_b > 0.25) {
        return n;
    }
    if state == 2 && 0.25 > path.ridge_a {
        path.dir_a = normalize0(path.pos_b - contact) * global_velocity.length();
        path.pos_a = (contact + path.pos_b) * 0.5;
        path.ridge_a = path.peak_b;
        path.peak_a = path.peak_b;
    }
    let from = if path.pos_a.y > contact.y - 1.0 { path.pos_a } else { contact };
    let d = path.pos_b - from;
    let up = normalize0(d.cross(Vec3::Y).cross(d));
    if up.abs().max_element() <= 1.192_092_9e-7 { Vec3::Y } else { up }
}
