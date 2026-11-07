use super::*;

pub(super) const EPS_FLT: f32 = 1.192_092_9e-7;

#[derive(Clone, Debug, PartialEq)]
pub struct HeliPosing {
    pub axis: Vec3,
    pub wanted_axis: Vec3,
    pub wanted_angle: f32,
    pub angle: f32,
    pub rate: f32,
    pub applied: f32,
    first: bool,
    pub root: Option<Quat>,
    pub orientation: Quat,
}

impl Default for HeliPosing {
    fn default() -> Self {
        Self {
            axis: Vec3::NEG_Z,
            wanted_axis: Vec3::NEG_Z,
            wanted_angle: 0.0,
            angle: 0.0,
            rate: 0.0,
            applied: 0.0,
            first: true,
            root: None,
            orientation: Quat::IDENTITY,
        }
    }
}

impl HeliPosing {
    pub fn update(&mut self, t: &HeliPosingTuning, q_traj: Quat, target_velocity: Vec3, acceleration: Vec3, dt: f32) {
        if dt.abs() <= EPS16 {
            (self.orientation, self.root) = (q_traj, None);
            return;
        }
        if self.first {
            // TODO(guess): the seed is the defaults — it reads the `DeltaTrajectory` set's slot at +0x10 (`angularVelocity`) in the trajectory's frame (0x82CBF3D0 → `Quat_ExpressInFrame` 0x83180650), which has no tilt after a yaw-only update (the NIS's TOC, every movement node's `conj(Q)·q_y`); a heli trick's clip may leave a tilted one; settle: the trick nodes' last angular velocity carried into the seed (the port's posing gets no angular velocity).
            crate::guess_hit!();
            *self = Self { first: false, ..Self::default() };
        }
        let (v, a) = (target_velocity.with_y(0.0), acceleration.with_y(0.0));
        let s_v = t.velocity_tilt.eval(v.length());
        let s_a = t.acceleration_tilt.eval(a.length());
        let (has_v, has_a) = (v.length_squared() > EPS_FLT, a.length_squared() > EPS_FLT);
        let tilt = match (has_v, has_a) {
            (false, false) => None,
            (true, false) => Some(v.normalize() * s_v),
            (false, true) => Some(a.normalize() * s_a),
            (true, true) => Some(a.normalize() * s_a + v.normalize() * s_v),
        };
        let angle = tilt.map_or(0.0, |x| x.length()).max(0.0).min(t.max_tilt);
        let up = q_traj * Vec3::Y;
        if angle.abs() <= EPS_FLT {
            (self.wanted_axis, self.wanted_angle) = (self.axis, 0.0);
        } else {
            let axis = tilt.unwrap_or(Vec3::ZERO).cross(up);
            (self.wanted_axis, self.wanted_angle) = (axis / axis.length(), angle);
        }
        if self.applied.abs() > EPS_FLT {
            let heading = |a: Vec3| (-a.z).atan2(a.x);
            let d = (heading(self.axis) - heading(self.wanted_axis) - std::f32::consts::PI) / std::f32::consts::TAU;
            let mut f = d - d.floor();
            if f > 0.5 {
                f -= 1.0;
            }
            if (f * std::f32::consts::TAU).abs() <= t.brake_tolerance {
                (self.wanted_axis, self.wanted_angle) = (-self.wanted_axis, -self.wanted_angle);
            }
        }
        if self.wanted_angle.abs() > EPS_FLT {
            let from = self.axis * self.applied.abs();
            let m = from + (self.wanted_axis * self.wanted_angle.abs() - from) * t.axis_convergence;
            self.axis = if m.abs().max_element() > EPS_FLT { m.normalize() } else { self.wanted_axis };
        }
        (self.angle, self.rate) = spring(t.strength.sqrt(), self.angle, self.rate, self.wanted_angle, dt);
        self.applied = self.angle * dt;
        let q = Quat::from_axis_angle(self.axis, self.applied);
        let root = (q_traj.conjugate() * q * q_traj).normalize();
        self.root = Some(root);
        self.orientation = (q_traj * root).normalize();
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct HeightGrid {
    pub min: Vec3,
    pub max: Vec3,
    pub inv_cell: [f32; 2],
    pub width: usize,
    pub height: usize,
    pub cells: Vec<f32>,
}

impl HeightGrid {
    pub fn load(track_dir: &Path, track: &str) -> Result<Self> {
        let path = track_dir.join(format!("{track}_track_helicopter.spdt"));
        let data = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        let f = ssx_formats::spdt::SpdtFile::parse(&data)?;
        let [cx, cz] = f.cell_size();
        let ssx_formats::spdt::SpdtCells::Height(cells) = f.cells else {
            anyhow::bail!("{} is no height grid", path.display());
        };
        Ok(Self {
            min: Vec3::from_array(f.bbox_min),
            max: Vec3::from_array(f.bbox_max),
            inv_cell: [1.0 / cx, 1.0 / cz],
            width: usize::from(f.width),
            height: usize::from(f.height),
            cells,
        })
    }

    pub(super) fn cell(&self, ix: i64, iz: i64) -> f32 {
        let i = iz * self.width as i64 + ix;
        usize::try_from(i).ok().and_then(|i| self.cells.get(i)).copied().unwrap_or(self.min.y)
    }

    pub(super) fn grid(&self, p: Vec3) -> (f32, f32) {
        let c = p.clamp(self.min, self.max) - self.min;
        (c.x * self.inv_cell[0], c.z * self.inv_cell[1])
    }

    pub fn height_at(&self, p: Vec3) -> f32 {
        let (gx, gz) = self.grid(p);
        let (fx, fz) = (gx.floor(), gz.floor());
        let (ix, iz) = (fx as i64, fz as i64);
        let ix1 = (ix + 1).min(self.width as i64 - 1);
        let iz1 = (iz + 1).min(self.height as i64 - 1);
        let (tx, tz) = (gx - fx, gz - fz);
        let a = self.cell(ix, iz) + (self.cell(ix1, iz) - self.cell(ix, iz)) * tx;
        let b = self.cell(ix, iz1) + (self.cell(ix1, iz1) - self.cell(ix, iz1)) * tx;
        a + tz * (b - a)
    }

    pub fn ray(&self, from: Vec3, to: Vec3) -> Option<Vec3> {
        let (sx, sz) = self.grid(from);
        let (ex, ez) = self.grid(to);
        let cell = [1.0 / self.inv_cell[0], 1.0 / self.inv_cell[1]];
        let (mut ix, mut iz) = (sx.floor() as i64, sz.floor() as i64);
        let (ixe, ize) = (ex.floor() as i64, ez.floor() as i64);
        let (sgx, mut nx) = if ixe >= ix { (1, ixe - ix) } else { (-1, ix - ixe) };
        let (sgz, mut nz) = if ize >= iz { (1, ize - iz) } else { (-1, iz - ize) };
        let mut rx = if sgx > 0 { (1.0 - (sx - ix as f32)) * cell[0] } else { (sx - ix as f32) * cell[0] };
        let mut rz = if sgz > 0 { (1.0 - (sz - iz as f32)) * cell[1] } else { (sz - iz as f32) * cell[1] };
        let d = to - from;
        let (slope, bias) = if d.x.abs() <= 1.192_092_9e-7 { (1.0, cell[1] + 1.0) } else { ((d.z / d.x).abs(), 0.0) };
        let len = ((ex - sx).powi(2) + (ez - sz).powi(2)).sqrt();
        let len = if len > 1.192_092_9e-7 { len } else { 1.0 };
        let mut hit = None;
        while nx >= 0 && nz >= 0 {
            let dc = ((ix as f32 - sx).powi(2) + (iz as f32 - sz).powi(2)).sqrt();
            let h = from.y + d.y * (dc / len).min(1.0);
            let g = self.cell(ix, iz);
            if g > h {
                hit = Some((ix, iz, g));
                break;
            }
            let along = slope * rx + bias;
            if along > rz {
                rx *= 1.0 - rz / along;
                iz += sgz;
                rz = cell[1];
                nz -= 1;
            } else {
                ix += sgx;
                rz -= slope * rx;
                rx = cell[0];
                nx -= 1;
            }
        }
        hit.map(|(ix, iz, g)| {
            Vec3::new(self.min.x + (ix as f32 + 0.5) * cell[0], g, self.min.z + (iz as f32 + 0.5) * cell[1])
        })
    }
}

pub(super) const EPS16: f32 = 1.525_878_9e-5;
pub(super) const EPS23: f32 = 1.192_092_9e-7;

pub(super) fn snap(v: Vec3, eps: f32) -> Vec3 {
    if v.abs().max_element() <= eps { Vec3::ZERO } else { v }
}

pub(super) fn yaw_of(d: Vec3, fallback: Quat) -> f32 {
    let d = if d.x.abs() <= EPS16 && d.z.abs() <= EPS16 { fallback * Vec3::X } else { d };
    (-d.z).atan2(d.x)
}

pub(super) fn twist_y(q: Quat) -> f32 {
    2.0 * q.y.atan2(q.w)
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pid {
    pub integral: Vec3,
    pub prev_error: Vec3,
}

impl Pid {
    pub(super) fn update(&mut self, gains: [f32; 3], dt: f32, set: Vec3, measured: Vec3) -> Vec3 {
        let e = set - measured;
        self.integral += e * dt;
        let raw = e * gains[0] + self.integral * gains[1] + (e - self.prev_error) / dt * gains[2];
        self.prev_error = e;
        let len = raw.length();
        let out = if len > EPS23 { raw / len * len.min(1.0) } else { Vec3::ZERO };
        if gains[1].abs() > EPS23 {
            let back = (raw - out) / gains[1];
            let lo = self.integral.min(Vec3::ZERO);
            let hi = self.integral.max(Vec3::ZERO);
            self.integral -= back.clamp(lo, hi);
        }
        out
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoveInputs {
    pub target: Vec3,
    pub target_orientation: Option<Quat>,
    pub avoid: Option<Vec3>,
    pub fixed: Option<FixedTarget>,
    pub avoid_boarder: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FixedTarget {
    pub time: f32,
    pub initial: f32,
    pub orientation: Option<Quat>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MoveOutputs {
    pub displacement: Vec3,
    pub rotation: Quat,
    pub velocity: Vec3,
    pub target_velocity: Vec3,
    pub acceleration: Vec3,
    pub intended_destination: Vec3,
    pub downwash: f32,
    pub at_target: bool,
    pub initial_time: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MoveController {
    pub pid: Pid,
    pub prev_target: Vec3,
    pub first: bool,
    pub posing: HeliPosing,
    pub ticks: f32,
}

impl Default for MoveController {
    fn default() -> Self {
        Self { pid: Pid::default(), prev_target: Vec3::ZERO, first: true, posing: HeliPosing::default(), ticks: 0.0 }
    }
}

impl MoveController {
    pub fn update(
        &mut self,
        t: &HeliMoveTuning,
        posing: Option<&HeliPosingTuning>,
        inp: &MoveInputs,
        traj: (Quat, Vec3),
        block: (Vec3, Quat),
        dt: f32,
    ) -> Option<MoveOutputs> {
        if dt.abs() <= EPS16 {
            return None;
        }
        let (q, p) = traj;
        let flat = (p - inp.target).with_y(0.0).length();
        let near = flat < 30.0 && !t.fixed_orientation;
        let within = flat < 2.0;
        let fixed = inp.fixed.filter(|_| t.fixed_target);
        let (time, initial) = match fixed {
            Some(f) => (f.time, if f.initial.abs() > EPS23 { f.initial } else { f.time }),
            None => (f32::MAX, 0.0),
        };
        let fixed_active = f32::MAX > time;
        let orient = match fixed {
            Some(f) => f.orientation.unwrap_or(q),
            None => inp.target_orientation.unwrap_or(q),
        };
        let target_yaw = yaw_of(orient * Vec3::X, q);
        let avoid = if near { None } else { inp.avoid };
        let avoid_boarder = inp.avoid_boarder.filter(|_| !near && t.avoid_boarder);
        let target = match avoid.filter(|_| fixed.is_none()) {
            Some(a) => {
                let to_a = a - p;
                if to_a.abs().max_element() <= EPS23 {
                    inp.target
                } else {
                    p + to_a.normalize_or_zero() * (to_a.length() + (inp.target - a).length())
                }
            }
            None => inp.target,
        };
        if self.first {
            (self.pid, self.prev_target, self.first) = (Pid::default(), target, false);
        }
        let v = snap(block.0, EPS16);
        let target_motion = snap((target - self.prev_target) / dt, EPS16);
        self.prev_target = target;
        let factor = if near { 2.0 } else { 1.0 };
        let d = target - p;
        let dist = d.length();
        let dir = if dist > 0.0 { d / dist } else { Vec3::ZERO };
        let wanted_motion = if avoid.is_some() { Vec3::ZERO } else { target_motion };
        let mut vertical = 1.0;
        if let Some(c) = &t.vertical_error_to_speed
            && v.y < 0.0
        {
            vertical = c.eval(d.y) * factor;
        }
        let mut speed = t.position_error_to_speed.eval(dist) / factor * t.max_velocity;
        if speed.abs() <= EPS16 {
            speed = 0.0;
        }
        let wanted = dir * speed + wanted_motion;
        let control = self.pid.update(t.pid, dt, wanted, v);
        let boarder = avoid_boarder.map_or(1.0, |d| {
            let k = t.avoid_boarder_thrust_scalar * t.avoid_boarder_thrust.eval(d);
            if k.abs() <= EPS23 { 1.0 } else { k }
        });
        let thrust = snap(control * (vertical * boarder) * t.nominal_thrust, EPS16);
        let drag = if v.length_squared() > EPS23 {
            snap(v.normalize() * (v.length_squared() * t.drag), EPS16)
        } else {
            Vec3::ZERO
        };
        let acceleration = thrust - drag;
        let mut velocity = v + snap(acceleration * dt, EPS16);
        if fixed_active {
            let gone = initial - time;
            let x = if EPS23 >= initial.abs() { 1.0 } else { gone * (1.0 / initial) };
            let tau = if dt > time { dt } else { t.time_speed_fixed_target.eval(x) * initial };
            let held = if dt > gone { dt } else { gone };
            let w = if t.frames_to_blend_fixed > held { held } else { t.frames_to_blend_fixed };
            let w = w * (1.0 / t.frames_to_blend_fixed);
            velocity = d.mul_add(Vec3::splat(1.0 / tau), velocity * (1.0 - w));
        }
        if dist <= EPS23 {
            velocity = Vec3::ZERO;
        }
        let displacement = snap(velocity * dt, EPS16);
        let wanted_speed = wanted.length();
        let to_target = yaw_of(if dist > 0.0 { dir } else { Vec3::ZERO }, q);
        let use_orientation =
            wanted_speed < t.velocity_facing_speed && dist < t.velocity_facing_distance || fixed_active;
        let desired = if use_orientation { target_yaw } else { to_target };
        let (yaw, rate) = (twist_y(q), twist_y(block.1));
        let omega = if fixed_active { t.facing_stiffness_fixed } else { t.facing_stiffness }.sqrt();
        let ks: &[f32] = if desired > yaw {
            &[-1.0, 0.0]
        } else if yaw > desired {
            &[0.0, 1.0]
        } else {
            &[0.0]
        };
        let (mut best, mut new_yaw) = (1e35f32, desired);
        for &k in ks {
            let (x, r) = spring(omega, yaw, rate, desired + k * std::f32::consts::TAU, dt);
            let f = (r - rate) / std::f32::consts::TAU;
            let mut f = f - f.floor();
            if f > 0.5 {
                f -= 1.0;
            }
            let change = (f * std::f32::consts::TAU).abs();
            if change < best {
                (best, new_yaw) = (change, x);
            }
        }
        let out = MoveOutputs {
            displacement,
            rotation: Quat::from_rotation_y(new_yaw),
            velocity,
            target_velocity: wanted,
            acceleration: if dt > time || dist <= EPS23 { Vec3::ZERO } else { acceleration },
            intended_destination: inp.target,
            downwash: thrust.length(),
            at_target: within,
            initial_time: if fixed.is_some() { initial } else { 0.0 },
        };
        if let Some(pt) = posing {
            self.posing.update(pt, q, out.target_velocity, out.acceleration, dt);
        }
        self.ticks += dt;
        Some(out)
    }
}

pub(super) fn spring(omega: f32, x0: f32, v0: f32, target: f32, t: f32) -> (f32, f32) {
    let a = x0 - target;
    let b = v0 + omega * a;
    let e = (-omega * t).exp();
    (target + (a + b * t) * e, (b - omega * (a + b * t)) * e)
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HeliBoard {
    pub speed_ratio: f32,
    pub target_position: Vec3,
    pub target_velocity: Vec3,
    pub desired_position: Vec3,
    pub avoid: Option<Vec3>,
}

impl HeliBoard {
    pub fn update(
        &mut self,
        t: &HeliUpdateTuning,
        grid: Option<&HeightGrid>,
        (p, v): (Vec3, Vec3),
        ib: &mut interaction::HeliInteractionBoard,
        reverse_cam: bool,
    ) {
        self.speed_ratio = (v.length() * 60.0 / t.max_speed_ratio).min(1.0);
        if ib.interact {
            if let Some(g) = grid
                && g.height_at(self.desired_position) > self.desired_position.y
            {
                ib.interact = false;
            }
            return;
        }
        let h = (self.target_velocity + if reverse_cam { Vec3::ONE } else { Vec3::ZERO }).with_y(0.0);
        let len = h.length();
        let mut dir = if len > 0.0 { h / len } else { Vec3::ZERO };
        if len.abs() <= EPS23 {
            dir += Vec3::Y;
        }
        let dir = dir.normalize_or_zero();
        let distance = if ib.prepare { t.prepare_target_distance } else { t.target_distance };
        self.desired_position = self.target_position + (dir - Vec3::Y * t.target_drop_scale) * distance;
        self.avoid = None;
        if ib.avoid {
            self.avoid = Some(ib.avoid_direction * t.avoid_boarder_strength + p);
            return;
        }
        let Some(grid) = grid else { return };
        let floor = grid.height_at(self.desired_position) + t.desired_elevation_agl;
        self.desired_position.y = self.desired_position.y.max(floor);
        let speed = v.length();
        if speed > EPS23 {
            let ahead = p + v * t.velocity_avoid_distance;
            if grid.ray(p, ahead).is_some() {
                let n = v / speed;
                let d = Vec3::new(0.5 * n.x, -(2.0 * n.y).abs(), 0.5 * n.z);
                let mut dest = p - d * t.velocity_avoid_target_distance;
                dest.y += 1.0;
                self.avoid = Some(match self.avoid {
                    Some(a) if a.y >= dest.y => a,
                    _ => dest,
                });
            }
        }
    }
}
