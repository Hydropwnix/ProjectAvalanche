use super::*;

#[derive(Clone, Copy, Debug)]
pub(super) struct Emit {
    pub(super) kind: Kind,
    pub(super) flags: u8,
    pub(super) density: f32,
    pub(super) tip: Vec3,
    pub(super) tip_prev: Vec3,
    pub(super) vel: Vec3,
    pub(super) vel_prev: Vec3,
    pub(super) normal: Vec3,
    pub(super) side: Vec3,
    pub(super) width: f32,
    pub(super) height: f32,
    pub(super) scale_min: f32,
    pub(super) scale_max: f32,
}

impl AvalancheSim {
    pub(super) fn emit_along_tip(&mut self, pt: &ParticleTuning, e: &Emit, acc: &mut f32, dt: f32) -> i32 {
        let (tr, per_parent) = match e.kind {
            Kind::Spray => (pt.spray, SPRAY_ATTACHED / SPRAY_RECORDS + 1),
            Kind::Slough => (pt.slough, 1),
            Kind::Slab => (pt.slab, 1),
        };
        let f = (e.tip - e.tip_prev).length() * e.density + *acc;
        let n = f.floor();
        *acc = f - n;
        let n = n as i32;
        if n < 1 {
            return 0;
        }
        let normal = (e.flags & FLAG_SPAWN_FRAME != 0).then_some(e.normal);
        let mut parent = 0;
        for k in 0..n {
            let r = if k == 0 { [0.0; 4] } else { self.rng.next4() };
            let idx = if (k as usize).is_multiple_of(per_parent) {
                parent = self.allocate(e.kind, None);
                parent
            } else {
                self.allocate(e.kind, Some(parent))
            };
            let (pos, vel, age) = if k == 0 {
                let v0 = e.vel_prev;
                (e.tip_prev + v0 * dt, drag_gravity(&tr, v0, dt) * dt + v0, dt)
            } else {
                let t = r[0];
                let vt = e.vel + (e.vel_prev - e.vel) * t;
                let pt0 = e.tip + (e.tip_prev - e.tip) * t;
                let spread = e.side * (vt.dot(e.side) * (2.0 * r[2] - 1.0) * e.width)
                    + e.normal * (vt.dot(e.normal) * (2.0 * r[1] - 1.0) * e.height);
                let v1 = vt + spread;
                let h = dt * t;
                (pt0 + v1 * h, drag_gravity(&tr, v1, h) * h + v1, h)
            };
            let frame = particle_frame(Quat::IDENTITY, vel, normal);
            let roll = random_roll(self.rng.next4(), e.flags & FLAG_SPAWN_FRAME != 0);
            let rw = if k == 0 { self.rng.next4()[3] } else { r[3] };
            let row = self.variant_row(e.kind);
            let p = self.get_mut(e.kind, idx);
            (p.pos, p.vel, p.frame, p.age, p.roll, p.row) = (pos, vel, frame, age, roll, row);
            p.scale = (e.scale_max - e.scale_min) * rw + e.scale_min;
            p.flags |= e.flags;
        }
        n
    }

    pub(super) fn variant_row(&mut self, kind: Kind) -> u8 {
        let r = self.rng.next4();
        if kind != Kind::Spray {
            0
        } else if self.inputs.ice_weight > r[0] {
            2
        } else if self.inputs.powder_weight > r[1] {
            0
        } else {
            1
        }
    }
}

impl AvalancheSim {
    pub fn gather_spray_records(&mut self, t: &AvalancheTuning) {
        let i = self.inputs;
        if i.dt.abs() <= EPS {
            return;
        }
        let tr = t.particles(i.switches.tuning_set).spray;
        for k in 0..SPRAY_RECORDS {
            let p = &mut self.spray[k];
            if p.flags & FLAG_SIMULATED != 0 && p.flags & FLAG_FROZEN == 0 {
                p.contact = self.spray_hits[k];
            }
        }
        for k in SPRAY_RECORDS..self.spray.len() {
            let p = self.spray[k];
            if p.flags & (FLAG_SIMULATED | FLAG_FROZEN) != 0 {
                continue;
            }
            if let Some((at, id)) = p.parent
                && id != 0
                && self.spray[at as usize].id == id
            {
                self.spray[k].contact = self.spray[at as usize].contact;
            }
        }
        let culling_sq = t.culling_distance * t.culling_distance;
        for k in 0..self.spray.len() {
            let mut p = self.spray[k];
            if p.alive() {
                let cleared = update_record(&mut p, &tr, t.surface_epsilon, &mut self.rng, i.pos, culling_sq, i.dt);
                self.spray_expired += u64::from(cleared);
                self.spray[k] = p;
            }
        }
    }

    pub(super) fn emit_spray(&mut self, pt: &ParticleTuning, dt: f32) {
        let i = self.inputs;
        if self.in_air {
            (self.spray_vel, self.spray_acc) = (Vec3::ZERO, 0.0);
        } else {
            if i.warped {
                (self.spray_vel, self.spray_acc) = (Vec3::ZERO, 0.0);
            }
            let v = i.spray + i.ice_spray * pt.ice_spray_scale;
            let s = v.length();
            let dir = if s > 0.0 { v / s } else { Vec3::ZERO };
            let (vmin, vmax) = (pt.spray_vector_min, pt.spray_vector_max);
            if s >= vmin && vmax > vmin {
                let e = pt.spray_velocity_exponent;
                let x = (s.min(vmax) - vmin).powf(e) / (vmax - vmin).powf(e);
                let vel = dir * ((pt.spray_velocity_max - pt.spray_velocity_min) * x + pt.spray_velocity_min);
                let u = norm_or_zero(i.vel);
                let b1 = norm_or_zero(dir.cross(u));
                let emit = Emit {
                    kind: Kind::Spray,
                    flags: 0,
                    density: pt.spray_density,
                    tip: self.tip,
                    tip_prev: self.tip_prev,
                    vel,
                    vel_prev: self.spray_vel,
                    normal: b1,
                    side: norm_or_zero(u.cross(b1)),
                    width: pt.spray_width,
                    height: pt.spray_height,
                    scale_min: pt.spray_scale_min,
                    scale_max: pt.spray_scale_max,
                };
                let mut acc = self.spray_acc;
                let n = self.emit_along_tip(pt, &emit, &mut acc, dt);
                (self.spray_acc, self.spray_vel) = (acc, vel);
                self.spawned_spray += n.max(0) as u64;
            } else {
                (self.spray_vel, self.spray_acc) = (Vec3::ZERO, 0.0);
            }
        }
        self.spray_landings.clear();
        for p in self.spray.iter_mut().filter(|p| p.alive() && p.flags & FLAG_CONTACT != 0) {
            self.spray_landings.push(*p);
            *p = Particle::default();
            self.spray_landed += 1;
        }
    }

    pub fn spray_particles(&self) -> impl Iterator<Item = &Particle> {
        self.spray.iter().filter(|p| p.alive())
    }
}

impl AvalancheSim {
    pub(super) fn carve_slabs(&mut self, pt: &ParticleTuning, dt: f32) {
        let i = self.inputs;
        if self.in_air {
            (self.burst, self.accumulator) = (0, 0.0);
            return;
        }
        if i.warped {
            (self.burst, self.accumulator) = (0, 0.0);
        }
        let ramp = descent_ramp(pt, i.descent);
        let coverage = (2.0 * i.snow_coverage - 1.0).clamp(0.0, 1.0);
        let sens = i.switches.carving_sensitivity.max(0.0);
        let mut s = self.spray_filtered * (sens * ramp * coverage);
        let (vmin, vmax) = (pt.carving_vector_min, pt.carving_vector_max);
        if self.burst > 0 && self.burst < pt.burst_size {
            s = s.max(vmin);
        }
        if s.partial_cmp(&vmin).is_none_or(std::cmp::Ordering::is_lt)
            || vmax.partial_cmp(&vmin) != Some(std::cmp::Ordering::Greater)
        {
            (self.burst, self.accumulator) = (0, 0.0);
            return;
        }
        let x = (s.min(vmax) - vmin) / (vmax - vmin);
        let density = vmin + (pt.carving_density_max - pt.carving_density_min) * x.powf(pt.carving_density_exponent);
        let normal = if i.ground_valid { i.ground_normal } else { Vec3::Y };
        let e = Emit {
            kind: Kind::Slab,
            flags: FLAG_SPAWN_FRAME,
            density,
            tip: self.tip,
            tip_prev: self.tip_prev,
            vel: i.vel * pt.carving_throw,
            vel_prev: i.prev_vel * pt.carving_throw,
            normal,
            side: norm_or_zero(i.vel.cross(normal)),
            width: pt.carving_width,
            height: pt.carving_height,
            scale_min: pt.slab_scale_min,
            scale_max: pt.slab_scale_max,
        };
        let mut acc = self.accumulator;
        let n = self.emit_along_tip(pt, &e, &mut acc, dt);
        self.accumulator = acc;
        self.burst += n;
        self.spawned_slabs += n.max(0) as u64;
    }

    pub(super) fn decay_slabs(&mut self, pt: &ParticleTuning, dt: f32) {
        let half = 0.5f32.powf(dt / pt.slab_half_life);
        for k in 0..self.slabs.len() {
            let mut p = self.slabs[k];
            if !p.alive() {
                continue;
            }
            if p.flags & FLAG_CONTACT != 0 {
                p.flags &= !FLAG_SPAWN_FRAME;
            }
            let mut s1 = p.scale * half;
            if p.flags & FLAG_CONTACT != 0 {
                let t = (2.0 * p.impact_cos - 1.0).clamp(0.0, 1.0);
                s1 = s1.min((pt.slab_max_scale_post_impact - s1) * t + s1);
            }
            let budget = (p.scale - s1) * pt.slab_slough_conversion_rate;
            p.scale = s1;
            self.slabs[k] = p;
            self.shed_sloughs(pt, &p, budget, dt);
        }
    }

    pub(super) fn shed_sloughs(&mut self, pt: &ParticleTuning, slab: &Particle, budget: f32, dt: f32) {
        let r0 = self.rng.next4();
        let whole = budget.floor();
        let n = (whole + if budget - whole > r0[3] { 1.0 } else { 0.0 }) as i32;
        if n < 1 {
            return;
        }
        let (x, z) = (slab.frame * Vec3::X, slab.frame * Vec3::Z);
        let spread = slab.vel.dot(x) * pt.slough_spread_ratio;
        let back = slab.pos - slab.vel * dt;
        for k in 0..n {
            let r = self.rng.next4();
            let vel = slab.vel + z * ((2.0 * r[2] - 1.0) * spread);
            let (pos, vel) = if k == 0 {
                (slab.pos, vel)
            } else {
                let h = dt * r[0];
                let p0 = slab.pos + (back - slab.pos) * r[0];
                (p0 + vel * h, drag_gravity(&pt.slough, vel, h) * h + vel)
            };
            let idx = self.allocate(Kind::Slough, None);
            let frame = particle_frame(Quat::IDENTITY, vel, None);
            let roll = random_roll(self.rng.next4(), false);
            let p = &mut self.sloughs[idx];
            (p.pos, p.vel, p.frame, p.age, p.roll) = (pos, vel, frame, slab.age, roll);
            p.scale = (pt.slough_scale_max - pt.slough_scale_min) * r[3] + pt.slough_scale_min;
            p.flags |= FLAG_SIMULATED | FLAG_ROLLING;
            self.spawned_sloughs += 1;
        }
    }

    pub(super) fn splosion(&mut self, pt: &ParticleTuning) {
        let tip = self.tip;
        let fallback = norm_or_zero(self.inputs.vel);
        for (pool, curve) in
            [(&mut self.sloughs, &pt.splosion_slough_impulse), (&mut self.slabs, &pt.splosion_slab_impulse)]
        {
            for p in pool.iter_mut().filter(|p| p.alive()) {
                let d = p.pos - tip;
                let dist = d.length();
                let dir = if dist > EPS { d / dist } else { fallback };
                p.vel += dir * curve.eval(dist.max(0.0));
            }
        }
        self.splosions += 1;
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Area {
    pub(super) kind: Kind,
    pub(super) flags: u8,
    pub(super) dir: Vec3,
    pub(super) speed: f32,
    pub(super) density: f32,
    pub(super) ratio: f32,
    pub(super) throw: f32,
    pub(super) lift: f32,
    pub(super) spread: f32,
    pub(super) ahead: f32,
    pub(super) behind: f32,
    pub(super) side: f32,
    pub(super) scale_min: f32,
    pub(super) scale_max: f32,
}

pub(super) fn descent_ramp(pt: &ParticleTuning, descent: f32) -> f32 {
    if pt.ramp_up_max > pt.ramp_up_min {
        let f = (descent - pt.ramp_up_min) / (pt.ramp_up_max - pt.ramp_up_min);
        let f = if -f >= 0.0 { 0.0 } else { f };
        if 1.0 - f >= 0.0 { f } else { 1.0 }
    } else {
        1.0
    }
}

pub(super) fn fsel_min(a: f32, b: f32) -> f32 {
    if a - b >= 0.0 { b } else { a }
}

impl AvalancheSim {
    pub(super) fn emit_area(&mut self, sa: Option<&SurfaceAnalysis>, a: &Area) -> u64 {
        let Some(sa) = sa else { return 0 };
        let i = self.inputs;
        let n = if i.ground_valid { i.ground_normal } else { Vec3::Y };
        let mut side = a.dir.cross(n);
        if side.abs().max_element() <= EPS {
            side = if n.z >= n.x { Vec3::X } else { Vec3::Z }.cross(n);
        }
        let side = side / side.length();
        let fwd = n.cross(side);
        let fwd = fwd / fwd.length();
        let tip_g = sa.grid(self.tip);
        let dir_g = sa.grid_dir(a.dir);
        let scale = sa.to_grid.x_axis.length() * a.ratio;
        let (ahead, behind, half_side) = (a.ahead * scale, a.behind * scale, a.side * scale);
        let sign = if dir_g.y >= 0.0 { 1.0 } else { -1.0 };
        let (cx, cy) = (tip_g.x, 0.5 * (ahead - behind) * sign + tip_g.y);
        if 2.0 > cx || cx > 253.0 || 2.0 > cy || cy > 253.0 {
            return 0;
        }
        let hx = fsel_min(half_side, (cx - 2.0).min(253.0 - cx));
        let hy = fsel_min(0.5 * (ahead + behind), (cy - 2.0).min(253.0 - cy));
        let spawn_frame = a.flags & FLAG_SPAWN_FRAME != 0;
        let thrown = fwd * a.speed * a.throw;
        let per_parent = if a.kind == Kind::Spray { SPRAY_ATTACHED / SPRAY_RECORDS + 1 } else { 1 };
        let mut parent = self.allocate(a.kind, None);
        let vel = n * a.lift + thrown;
        let roll = random_roll(self.rng.next4(), spawn_frame);
        let frame = particle_frame(Quat::IDENTITY, vel, spawn_frame.then_some(n));
        let rw = self.rng.next4()[3];
        let (row, tip) = (self.variant_row(a.kind), self.tip);
        let p = self.get_mut(a.kind, parent);
        (p.pos, p.vel, p.frame, p.roll, p.row) = (tip, vel, frame, roll, row);
        p.scale = (a.scale_max - a.scale_min) * rw + a.scale_min;
        p.flags |= a.flags;
        let mut count = 1;
        for k in 1..=a.density.floor() as i32 {
            let r = self.rng.next4().map(|x| 2.0 * x - 1.0);
            let (x, y) = ((hx * r[0] + cx + 0.5).floor(), (hy * r[1] + cy + 0.5).floor());
            if !((2.0..=253.0).contains(&x) && (2.0..=253.0).contains(&y)) {
                continue;
            }
            let (x, y) = (x as usize, y as usize);
            if sa.height(x, y) == 0 {
                continue;
            }
            let nc = sa.cell_normal(x, y, 2);
            if nc.abs().max_element() <= EPS {
                continue;
            }
            let v = side * a.spread * a.speed * r[3] + thrown;
            let v = nc * (a.lift - v.dot(nc)) + v;
            let idx = if (k as usize).is_multiple_of(per_parent) {
                parent = self.allocate(a.kind, None);
                parent
            } else {
                self.allocate(a.kind, Some(parent))
            };
            let frame = particle_frame(Quat::IDENTITY, v, spawn_frame.then_some(nc));
            let roll = random_roll(self.rng.next4(), spawn_frame);
            let rw = self.rng.next4()[3];
            let row = self.variant_row(a.kind);
            let p = self.get_mut(a.kind, idx);
            (p.pos, p.vel, p.frame, p.roll, p.row) = (sa.cell_world(x, y), v, frame, roll, row);
            p.scale = (a.scale_max - a.scale_min) * rw + a.scale_min;
            p.flags |= a.flags;
            count += 1;
        }
        count
    }

    pub(super) fn impact_slabs(&mut self, pt: &ParticleTuning, sa: Option<&SurfaceAnalysis>) {
        let i = self.inputs;
        let sens = i.switches.impact_sensitivity.max(0.0)
            * descent_ramp(pt, i.descent)
            * (2.0 * i.snow_coverage - 1.0).clamp(0.0, 1.0);
        let (fmin, fmax) = (pt.impact_trigger_force_min, pt.impact_trigger_force_max);
        let f = fmax.min(fmin.max(self.landing_impact * sens));
        if fmin + SEGMENT_MIN_DELTA > f || fmin >= fmax || self.landing_speed.abs() <= EPS {
            return;
        }
        let e = pt.impact_density_exponent;
        let x = (f - fmin).powf(e) / (fmax - fmin).powf(e);
        let d = (pt.impact_density_max - pt.impact_density_min) * x + pt.impact_density_min;
        let cs = i.switches.chunk_scale;
        let a = Area {
            kind: Kind::Slab,
            flags: FLAG_SPAWN_FRAME,
            dir: self.landing_dir,
            speed: self.landing_speed,
            density: d,
            ratio: d * (1.0 / pt.impact_density_max),
            throw: pt.impact_throw,
            lift: pt.impact_lift,
            spread: pt.impact_spread,
            ahead: cs * pt.impact_size_ahead,
            behind: cs * pt.impact_size_behind,
            side: cs * pt.impact_size_left_right,
            scale_min: cs * pt.slab_scale_min,
            scale_max: cs * pt.slab_scale_max,
        };
        let n = self.emit_area(sa, &a);
        self.spawned_slabs += n;
        self.impact_bursts += u64::from(n > 0);
    }

    pub(super) fn splosion_sloughs(&mut self, pt: &ParticleTuning, sa: Option<&SurfaceAnalysis>) {
        let i = self.inputs;
        let speed = if self.landing_impact.abs() <= EPS { i.prev_vel.length() } else { self.landing_impact };
        let lr = pt.impact_size_left_right;
        let a = Area {
            kind: Kind::Slough,
            flags: 0,
            dir: norm_or_zero(i.vel),
            speed,
            density: pt.splosion_density,
            ratio: 1.0,
            throw: pt.impact_throw,
            lift: pt.impact_lift,
            spread: pt.impact_spread,
            ahead: lr,
            behind: lr,
            side: lr,
            scale_min: pt.slough_scale_min,
            scale_max: pt.slough_scale_max,
        };
        self.spawned_sloughs += self.emit_area(sa, &a);
    }
}
