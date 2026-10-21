use glam::{Quat, Vec3};

use crate::character::PlayerTuning;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct JumpState {
    pub on_ground_ticks: f32,
    pub in_air_ticks: f32,
    pub allow: bool,
    pub trigger: bool,
    pub held: bool,
    pub launch: Vec3,
    pub a_hold_filtered: f32,
    pub compression: f32,
}

pub fn calculated_compression(a_hold_filtered: f32, rstick_y_filtered: f32, compression: f32) -> f32 {
    let x = a_hold_filtered.max(-rstick_y_filtered).clamp(0.0, 1.0);
    (1.5 * x - 1.0).max(x + (1.0 - 0.5 * x) * compression).min(0.7 + 0.3 * x)
}

impl JumpState {
    pub fn update(&mut self, held: bool, on_ground: bool, dt: f32, p: &PlayerTuning) {
        let request = self.held && !held;
        self.held = held;
        if on_ground {
            let pending = !self.allow && self.trigger;
            self.on_ground_ticks += dt;
            self.trigger = request || pending;
            self.allow = self.on_ground_ticks >= p.min_on_ground_jump_ticks;
            self.in_air_ticks = 0.0;
        } else {
            self.in_air_ticks += dt;
            self.allow = self.allow && self.in_air_ticks <= p.max_in_air_jump_ticks;
            self.trigger = request;
            self.on_ground_ticks = 0.0;
        }
    }

    pub fn update_compression(&mut self, held: bool, dt: f32, p: &PlayerTuning) {
        let target = if held { 1.0 } else { 0.0 };
        let k = (-dt * p.filter_gain).exp();
        self.a_hold_filtered = (self.a_hold_filtered - target) * k + target;
        self.compression = calculated_compression(self.a_hold_filtered, 0.0, 0.0);
    }

    pub fn take_trigger(&mut self) -> bool {
        let fire = self.allow && self.trigger;
        if fire {
            self.trigger = false;
        }
        fire
    }
}

fn blend_dir(a: Vec3, b: Vec3, t: f32) -> Vec3 {
    let cos = a.dot(b);
    if cos > 0.996_194_7 {
        return (a + (b - a) * t).try_normalize().unwrap_or(a);
    }
    let th = cos.clamp(-1.0, 1.0).acos();
    let s = th.sin();
    if s.abs() <= 1e-7 {
        return a;
    }
    (a * ((1.0 - t) * th).sin() + b * (t * th).sin()) / s
}

#[derive(Clone, Copy, Debug)]
pub struct LaunchInputs {
    pub velocity: Vec3,
    pub in_air: bool,
    pub ground_normal: Vec3,
    pub traj_up: Vec3,
    pub traj_pos: Vec3,
    pub compression: f32,
    pub dt: f32,
    pub grinding: bool,
    pub stick_angle: f32,
    pub stick_mag: f32,
    pub wind: Vec3,
}

pub fn grind_launch_dir(v: Vec3, n: Vec3, stick_angle: f32, stick_mag: f32, max_lateral: f32) -> Vec3 {
    let n = n.normalize_or_zero();
    if stick_mag.abs() <= 1.192_092_9e-7 || v.cross(n).length() <= 1.192_092_9e-7 {
        return Vec3::ZERO;
    }
    let th = stick_angle.clamp(-max_lateral, max_lateral);
    let h = n.cross(v.cross(n).normalize());
    h * th.cos() + h.cross(n) * th.sin()
}

pub fn jump_launch(w: &crate::rider::World, i: &LaunchInputs) -> Option<Vec3> {
    let q = &w.tuning.queries;
    let v = i.velocity;
    if i.dt.abs() <= 1.525_88e-5 {
        return None;
    }
    let speed = v.length();
    if speed <= 1.192_092_9e-7 {
        return None;
    }
    let dir = if i.grinding {
        let g = grind_launch_dir(v, i.ground_normal, i.stick_angle, i.stick_mag, q.jump_launch_max_lateral_angle);
        if g == Vec3::ZERO { v } else { g }
    } else {
        v / speed
    };
    let n = i.ground_normal;
    let s = q.jump_launch_speed_vs_speed.eval(speed) * (1.0 + q.jump_launch_compression_factor * i.compression);
    let pitch = if i.in_air { 0.0 } else { q.jump_launch_angle_vs_speed.eval(speed) };
    let up0 = i.traj_up.try_normalize().unwrap_or(Vec3::Y);
    let t_up = q.up_bias_vs_ground_normal_angle.eval(n.y.clamp(-1.0, 1.0).acos());
    let t_n = q.normal_bias_vs_lean_angle.eval(n.dot(up0).clamp(-1.0, 1.0).acos());
    let up1 = if up0.y < -0.98 { up0 } else { blend_dir(up0, Vec3::Y, t_up) };
    let up = if up1.dot(n) < -0.98 { up1 } else { blend_dir(up1, n, t_n) };
    if dir.dot(up).clamp(-1.0, 1.0).abs() > q.jump_launch_cos_small_angle {
        return Some(up * s);
    }
    let launch_dir = (up + q.jump_launch_forward_factor * (dir - up)).try_normalize().unwrap_or(up);
    let side = dir.cross(up).try_normalize().unwrap_or(Vec3::X);
    let target = Quat::from_axis_angle(side, pitch) * (dir * speed + launch_dir * s);
    Some(obstacle_fan(w, target, side, i) - v)
}

fn obstacle_fan(w: &crate::rider::World, t: Vec3, side: Vec3, i: &LaunchInputs) -> Vec3 {
    let q = &w.tuning.queries;
    let n_rays = q.num_jump_launch_probe_rays as usize;
    let big_t = t.length();
    if n_rays == 0 || big_t <= 1.192_092_9e-7 {
        return t;
    }
    let d = t / big_t;
    let u = side.cross(d).try_normalize().unwrap_or(Vec3::Y);
    let mut c = vec![t];
    for k in 0..=n_rays / 2 {
        let th = std::f32::consts::TAU * k as f32 / n_rays as f32;
        let (sin, cos) = th.sin_cos();
        let up_down = if cos >= 0.0 { q.jump_launch_max_up_angle } else { q.jump_launch_max_down_angle };
        let a = big_t * up_down.sin() * cos;
        let b = big_t * q.jump_launch_max_side_angle.sin() * sin;
        let cc = (big_t * big_t - a * a - b * b).max(0.0).sqrt();
        c.push(d * cc + u * a + side * b);
        if sin.abs() > 1.192_092_9e-7 {
            c.push(d * cc + u * a - side * b);
        }
    }
    let r = &w.props;
    let sim = |mut vel: Vec3| {
        let (mut p, mut ticks) = (i.traj_pos, q.jump_launch_lookahead_ticks);
        while ticks > 1.525_88e-5 {
            let mut a = crate::rider::drag_accel(r.k_h_air, r.k_v_air, vel, i.wind, i.dt);
            ticks -= i.dt;
            a.y -= r.g_air.y.abs();
            vel += a * i.dt;
            p += vel * i.dt;
        }
        p
    };
    let ends: Vec<Vec3> = c.iter().map(|&vel| sim(vel)).collect();
    let n_c = c.len() - 1;
    let mut segs = vec![ends[0]];
    segs.extend((1..=n_c).map(|k| (ends[0] + ends[k]) * 0.5));
    segs.extend(ends[1..].iter().copied());
    let (mut best, mut best_f) = (0usize, 0.0f32);
    let mut chosen = None;
    for (k, &e) in segs.iter().enumerate() {
        let len = (e - i.traj_pos).length();
        let hit = w.collision.segment_query(&crate::collision::SegmentQuery::line(i.traj_pos, e, 0));
        match hit {
            None => {
                chosen = Some(k);
                break;
            }
            Some(h) => {
                let f = if len > 0.0 { (h.t / len).clamp(0.0, 1.0) } else { 0.0 };
                if f > best_f {
                    (best, best_f) = (k, f);
                }
            }
        }
    }
    let k = chosen.unwrap_or(best);
    if k == 0 {
        t
    } else if k > n_c {
        c[k - n_c]
    } else {
        t + (c[k] - t) * 0.5
    }
}
