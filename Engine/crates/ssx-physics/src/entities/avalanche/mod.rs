use anyhow::Result;
use glam::{Quat, Vec3};

use crate::collision::SEGMENT_MIN_DELTA;
use crate::curve::Curve;
use crate::rider::grind::surface_analysis::SurfaceAnalysis;
use crate::tuning::{AssetView, Bundle};

mod emit;
mod probe;
mod tuning;

pub use probe::*;
pub use tuning::*;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DropAvalanche {
    pub objective_tuning_set: Option<i32>,
    pub avalanche_drop: bool,
    pub reverse_cam: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AvalancheSwitches {
    pub tuning_set: i32,
    pub carving_sensitivity: f32,
    pub impact_sensitivity: f32,
    pub chunk_scale: f32,
}

impl Default for AvalancheSwitches {
    fn default() -> Self {
        Self { tuning_set: DEFAULT_TUNING_SET, carving_sensitivity: 1.0, impact_sensitivity: 0.0, chunk_scale: 1.0 }
    }
}

pub const SPRAY_RECORDS: usize = 30;
pub const SPRAY_ATTACHED: usize = 240;
pub const SLOUGH_RECORDS: usize = 240;
pub const SLAB_RECORDS: usize = 80;

pub const FLAG_SIMULATED: u8 = 0x01;
pub const FLAG_CONTACT: u8 = 0x02;
pub const FLAG_ROLLING: u8 = 0x04;
pub const FLAG_FROZEN: u8 = 0x08;
pub const FLAG_SPAWN_FRAME: u8 = 0x10;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    Spray,
    #[default]
    Slough,
    Slab,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Particle {
    pub kind: Kind,
    pub pos: Vec3,
    pub scale: f32,
    pub vel: Vec3,
    pub frame: Quat,
    pub contact: Option<(Vec3, Vec3)>,
    pub impact_cos: f32,
    pub roll: Quat,
    pub lift: f32,
    pub age: f32,
    pub row: u8,
    pub flags: u8,
    pub id: u16,
    pub parent: Option<(u16, u16)>,
}

impl Particle {
    pub fn alive(&self) -> bool {
        self.id != 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rng(pub [u32; 4]);

impl Default for Rng {
    fn default() -> Self {
        Self([0x00F1_FBDC, 0x43BB_7EB2, 0x5CAD_6A98, 0x53DF_C8C1])
    }
}

impl Rng {
    pub fn next4(&mut self) -> [f32; 4] {
        let mut out = [0.0; 4];
        for (x, o) in self.0.iter_mut().zip(out.iter_mut()) {
            *x ^= *x << 13;
            *x ^= *x >> 17;
            *x ^= *x << 5;
            *o = f32::from_bits(0x3F80_0000 | (*x & 0x007F_FFFF)) - 1.0;
        }
        out
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Measure {
    pub orientation: Quat,
    pub span: Vec3,
    pub speed: f32,
    pub mass: f32,
}

impl Default for Measure {
    fn default() -> Self {
        Self { orientation: Quat::IDENTITY, span: Vec3::ZERO, speed: 0.0, mass: 0.0 }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SimInputs {
    pub dt: f32,
    pub pos: Vec3,
    pub vel: Vec3,
    pub prev_vel: Vec3,
    pub stopped: bool,
    pub warped: bool,
    pub in_air: bool,
    pub big_landing: bool,
    pub ground_valid: bool,
    pub ground_normal: Vec3,
    pub spray: Vec3,
    pub snow_coverage: f32,
    pub descent: f32,
    pub ice_spray: Vec3,
    pub ice_weight: f32,
    pub powder_weight: f32,
    pub board_pos: Vec3,
    pub board_z: Vec3,
    pub switches: AvalancheSwitches,
    pub snow_depth: f32,
}

const EPS: f32 = 1.192_092_9e-7;

#[derive(Clone, Debug)]
pub struct AvalancheSim {
    pub spray: Vec<Particle>,
    pub sloughs: Vec<Particle>,
    pub slabs: Vec<Particle>,
    pub rng: Rng,
    next_id: u16,
    first: bool,
    pub inputs: SimInputs,
    in_air: bool,
    prev_in_air: bool,
    pub landing_dir: Vec3,
    pub landing_impact: f32,
    pub landing_speed: f32,
    pub spray_filtered: f32,
    pub tip: Vec3,
    pub tip_prev: Vec3,
    burst: i32,
    accumulator: f32,
    pub spray_vel: Vec3,
    spray_acc: f32,
    pub aggregate_pos: Vec3,
    pub aggregate: Measure,
    pub probe: Measure,
    hits: Vec<Option<(Vec3, Vec3)>>,
    spray_hits: Vec<Option<(Vec3, Vec3)>>,
    pub spawned_slabs: u64,
    pub spawned_sloughs: u64,
    pub spawned_spray: u64,
    pub splosions: u64,
    pub impact_bursts: u64,
    pub spray_landed: u64,
    pub spray_expired: u64,
    pub spray_landings: Vec<Particle>,
}

impl Default for AvalancheSim {
    fn default() -> Self {
        Self {
            spray: vec![Particle::default(); SPRAY_RECORDS + SPRAY_ATTACHED],
            sloughs: vec![Particle::default(); SLOUGH_RECORDS],
            slabs: vec![Particle::default(); SLAB_RECORDS],
            rng: Rng::default(),
            next_id: 0,
            first: true,
            inputs: SimInputs::default(),
            in_air: false,
            prev_in_air: false,
            landing_dir: Vec3::NEG_Y,
            landing_impact: 0.0,
            landing_speed: 0.0,
            spray_filtered: 0.0,
            tip: Vec3::ZERO,
            tip_prev: Vec3::ZERO,
            burst: 0,
            accumulator: 0.0,
            spray_vel: Vec3::ZERO,
            spray_acc: 0.0,
            aggregate_pos: Vec3::ZERO,
            aggregate: Measure::default(),
            probe: Measure::default(),
            hits: vec![None; SLOUGH_RECORDS + SLAB_RECORDS],
            spray_hits: vec![None; SPRAY_RECORDS],
            spawned_slabs: 0,
            spawned_sloughs: 0,
            spawned_spray: 0,
            splosions: 0,
            impact_bursts: 0,
            spray_landed: 0,
            spray_expired: 0,
            spray_landings: Vec::new(),
        }
    }
}

pub fn random_roll(r: [f32; 4], y_only: bool) -> Quat {
    let v = glam::Vec4::from_array(r) * 2.0 - glam::Vec4::ONE;
    let mask = if y_only { glam::Vec4::new(0.0, 1.0, 0.0, 0.0) } else { glam::Vec4::ONE };
    let m = v * mask;
    if m.abs().max_element() <= EPS {
        return Quat::IDENTITY;
    }
    let xyz = m.truncate();
    let d = xyz.length_squared();
    let s = if d > 0.0 { ((1.0 - v.w * v.w) / d).sqrt() } else { 0.0 };
    Quat::from_xyzw(xyz.x * s, xyz.y * s, xyz.z * s, v.w)
}

pub fn roll_step(p: &mut Particle, roll_factor: f32, dt: f32) {
    if p.flags & FLAG_ROLLING == 0 || p.scale <= 0.0 {
        return;
    }
    let (s, c) = (roll_factor * p.vel.length() * dt / p.scale * 0.5).sin_cos();
    p.roll = Quat::from_xyzw(0.0, 0.0, -s, c) * p.roll;
}

fn norm_or_zero(v: Vec3) -> Vec3 {
    let l = v.length();
    if l > EPS { v / l } else { Vec3::ZERO }
}

fn decay(rate: f32, dt: f32) -> f32 {
    (-rate * dt).exp()
}

pub fn particle_frame(old: Quat, vel: Vec3, normal: Option<Vec3>) -> Quat {
    let tiny = |v: Vec3| v.abs().max_element() <= EPS;
    let unit = |v: Vec3| v / v.length();
    let x0 = if tiny(vel) { old * Vec3::X } else { unit(vel) };
    let (x, y, z) = match normal {
        Some(n) => {
            let z = x0.cross(n);
            if tiny(z) {
                let x = n.cross(old * Vec3::Z);
                let x = if tiny(x) { old * Vec3::X } else { unit(x) };
                (x, n, unit(x.cross(n)))
            } else {
                let z = unit(z);
                (unit(n.cross(z)), n, z)
            }
        }
        None => {
            let z = x0.cross(old * Vec3::Y);
            let z = if tiny(z) { old * Vec3::Z } else { unit(z) };
            (x0, unit(z.cross(x0)), z)
        }
    };
    Quat::from_mat3(&glam::Mat3::from_cols(x, y, z)).normalize()
}

impl AvalancheSim {
    pub fn warp(&mut self) {
        self.first = true;
    }

    pub fn particles(&self) -> impl Iterator<Item = &Particle> {
        self.sloughs.iter().chain(&self.slabs).filter(|p| p.alive())
    }

    pub fn dispatch(&mut self, t: &AvalancheTuning, i: SimInputs, world: &crate::collision::CollisionWorld) {
        let pt = t.particles(i.switches.tuning_set);
        let warped = i.warped || self.first;
        self.first = false;
        let mut i = i;
        if i.stopped {
            (i.vel, i.prev_vel) = (Vec3::ZERO, Vec3::ZERO);
        }
        let target = if i.stopped { 0.0 } else { i.spray.length() };
        self.spray_filtered = if warped || i.stopped {
            target
        } else {
            let e = decay(pt.spray_vector_filter_lag, i.dt);
            e * (target - self.spray_filtered) + self.spray_filtered
        };
        self.prev_in_air = if warped { i.in_air } else { self.in_air };
        self.in_air = i.in_air;
        let landing = !self.in_air && self.prev_in_air && i.ground_valid && i.prev_vel.length() > EPS;
        (self.landing_dir, self.landing_impact, self.landing_speed) = if landing {
            let n = i.ground_normal;
            (norm_or_zero(i.prev_vel), (-i.prev_vel.dot(n)).max(0.0), i.prev_vel.length())
        } else {
            (Vec3::NEG_Y, 0.0, 0.0)
        };
        i.warped = warped;
        self.inputs = i;
        let dt = i.dt;
        for (k, p) in self.spray.iter().take(SPRAY_RECORDS).enumerate() {
            self.spray_hits[k] = sweep(p, world, dt);
        }
        for (k, p) in self.sloughs.iter().chain(&self.slabs).enumerate() {
            self.hits[k] = sweep(p, world, dt);
        }
    }

    pub fn next_record_id(&mut self) -> u16 {
        self.next_id = self.next_id.wrapping_add(1);
        if self.next_id == 0 {
            self.next_id = 1;
        }
        self.next_id
    }

    fn allocate(&mut self, kind: Kind, parent: Option<usize>) -> usize {
        let (pool, range) = match (kind, parent) {
            (Kind::Spray, None) => (&self.spray, 0..SPRAY_RECORDS),
            (Kind::Spray, Some(_)) => (&self.spray, SPRAY_RECORDS..SPRAY_RECORDS + SPRAY_ATTACHED),
            (Kind::Slough, _) => (&self.sloughs, 0..SLOUGH_RECORDS),
            (Kind::Slab, _) => (&self.slabs, 0..SLAB_RECORDS),
        };
        let mut best: Option<usize> = None;
        let mut free = None;
        for k in range.clone() {
            let p = &pool[k];
            if !p.alive() {
                free = Some(k);
                break;
            }
            if best.is_none_or(|b| p.age > pool[b].age) {
                best = Some(k);
            }
        }
        let k = free.or(best).unwrap_or(range.start);
        let link = parent.map(|i| (i as u16, self.spray[i].id));
        let p = Particle {
            kind,
            frame: Quat::IDENTITY,
            roll: Quat::IDENTITY,
            lift: self.inputs.snow_depth,
            id: self.next_record_id(),
            flags: if parent.is_none() { FLAG_SIMULATED } else { 0 },
            parent: link,
            ..Particle::default()
        };
        *self.get_mut(kind, k) = p;
        k
    }

    fn get_mut(&mut self, kind: Kind, k: usize) -> &mut Particle {
        match kind {
            Kind::Spray => &mut self.spray[k],
            Kind::Slough => &mut self.sloughs[k],
            Kind::Slab => &mut self.slabs[k],
        }
    }
}

pub fn sweep(p: &Particle, world: &crate::collision::CollisionWorld, dt: f32) -> Option<(Vec3, Vec3)> {
    if !p.alive() || p.flags & FLAG_SIMULATED == 0 || p.flags & FLAG_FROZEN != 0 {
        return None;
    }
    world.particle_sweep(p.pos, p.vel * (dt * 2.0))
}

fn drag_gravity(tr: &ParticleTraits, v: Vec3, dt: f32) -> Vec3 {
    let (kh, kv, g) = tr.drag();
    let mut a = crate::rider::drag_accel(kh, kv, v, Vec3::ZERO, dt);
    a.y -= g;
    a
}

pub fn integrate(p: &mut Particle, tr: &ParticleTraits, eps: f32, rng: &mut Rng, dt: f32) {
    p.flags &= !FLAG_CONTACT;
    p.impact_cos = 0.0;
    if p.flags & FLAG_FROZEN != 0 {
        return;
    }
    let mut pos = p.pos + p.vel * dt;
    let mut v = p.vel;
    let contact = p.contact.filter(|(_, n)| n.abs().max_element() > EPS);
    if let Some((c, n)) = contact
        && (pos - c).dot(n) <= eps
    {
        let mut cos = 0.0;
        if v.abs().max_element() > EPS {
            p.flags |= FLAG_CONTACT;
            cos = -norm_or_zero(v).dot(n);
        }
        pos = if p.flags & FLAG_SIMULATED != 0 { c + n * eps } else { pos - n * ((pos - c).dot(n) - eps) };
        if cos > 0.0 {
            p.impact_cos = cos;
            let r = rng.next4();
            let noise = Vec3::new(r[0] - 0.5, r[1] - 0.5, r[2] - 0.5);
            let n2 = norm_or_zero(noise * tr.surface_noise + n);
            let vn = n2 * v.dot(n2);
            let vt = v - vn;
            let bounced = vt - vn * tr.restitution;
            let t =
                if tr.stopping_speed.abs() <= EPS { 1.0 } else { (vt.length() / tr.stopping_speed).clamp(0.0, 1.0) };
            let mu = (tr.roll_friction - tr.stop_friction) * t + tr.stop_friction;
            let (s0, s1) = (v.length(), bounced.length());
            let speed = (s0 - s1) * tr.energy_preservation + s1;
            v = norm_or_zero(bounced) * speed * (1.0 - mu);
        }
    }
    let mut v_new = drag_gravity(tr, v, dt) * dt + v;
    if p.flags & FLAG_CONTACT != 0
        && let Some((_, n)) = contact
        && v_new.dot(n) < 0.0
    {
        let vt = v_new - n * v_new.dot(n);
        if vt.length() <= 2.0 * eps {
            v_new = Vec3::ZERO;
            p.flags |= FLAG_FROZEN;
            p.age = p.age.max(tr.max_lifetime - tr.max_lifetime_frozen);
        } else {
            let step = (v_new * dt).length_squared() - 4.0 * eps * eps;
            let len = if step > 0.0 { step.sqrt() } else { 0.0 };
            v_new = (norm_or_zero(vt) * len - n * (2.0 * eps)) / dt;
        }
    }
    p.pos = pos;
    p.vel = v_new;
}

pub fn update_record(
    p: &mut Particle,
    tr: &ParticleTraits,
    eps: f32,
    rng: &mut Rng,
    actor: Vec3,
    culling_sq: f32,
    dt: f32,
) -> bool {
    integrate(p, tr, eps, rng, dt);
    if p.flags & FLAG_SPAWN_FRAME == 0 || p.flags & FLAG_CONTACT != 0 {
        let n = (p.flags & FLAG_CONTACT != 0).then(|| p.contact.map(|c| c.1)).flatten();
        p.frame = particle_frame(p.frame, p.vel, n);
    }
    roll_step(p, tr.roll_factor, dt);
    age_and_cull(p, tr, actor, culling_sq, dt);
    !p.alive()
}

pub fn age_and_cull(p: &mut Particle, tr: &ParticleTraits, actor: Vec3, culling_sq: f32, dt: f32) {
    if (p.pos - actor).length_squared() > culling_sq {
        *p = Particle::default();
        return;
    }
    let mult = if p.flags & FLAG_CONTACT != 0 { tr.aging_ground } else { tr.aging_air };
    p.age += dt * mult;
    if p.age > tr.max_lifetime {
        *p = Particle::default();
    }
}

#[cfg(test)]
mod tests;
