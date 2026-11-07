#![allow(clippy::neg_cmp_op_on_partial_ord, clippy::manual_range_contains)]

pub mod cylcyl;
pub mod features;
pub mod fixup;
pub mod narrow;
pub mod setup;
pub mod solver;

use glam::{Quat, Vec3};

use crate::collision::CollisionWorld;
use narrow::{CylinderGp, TriangleGp};
use solver::{Contact, Motion};

pub const GRAVITY: Vec3 = Vec3::new(0.0, -9.80665, 0.0);
pub const MAX_ITERATIONS: u32 = 20;
pub const MIN_PADDING: f32 = 0.1;
pub const TERRAIN_QUERY_WIDENING: f32 = 2.0 * 0.008;
pub const MAX_COOL_DOWN: f32 = 0.3;
pub const ACTIVATION_ENERGY: f32 = 0.2;
pub const COOLING_TOLERANCE: f32 = 0.1;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RigidBodyDef {
    pub offset: Vec3,
    pub axis: Vec3,
    pub half_height: f32,
    pub radius: f32,
    pub fat: f32,
    pub inv_mass: f32,
    pub inv_inertia: Vec3,
    pub max_speed: f32,
    pub max_angular_speed: f32,
    pub linear_damping: f32,
    pub angular_damping: f32,
    pub material: [f32; 3],
}

impl From<&ssx_content::object::RigidBody> for RigidBodyDef {
    fn from(b: &ssx_content::object::RigidBody) -> Self {
        RigidBodyDef {
            offset: Vec3::from_array(b.offset),
            axis: Vec3::from_array(b.axis),
            half_height: b.half_height,
            radius: b.radius,
            fat: b.fat,
            inv_mass: b.inv_mass,
            inv_inertia: Vec3::from_array(b.inv_inertia),
            max_speed: b.max_speed,
            max_angular_speed: b.max_angular_speed,
            linear_damping: b.linear_damping,
            angular_damping: b.angular_damping,
            material: b.material,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Body {
    pub def: RigidBodyDef,
    pub motion: Motion,
    pub frozen: bool,
    pub tracked: bool,
    pub cool_down: f32,
    pub last_energy: f32,
}

impl Body {
    pub fn new(def: RigidBodyDef, position: Vec3, orientation: Quat) -> Self {
        let motion = Motion {
            position,
            orientation,
            velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            inv_mass: def.inv_mass,
            inv_inertia: def.inv_inertia,
            max_speed: def.max_speed,
            max_angular_speed: def.max_angular_speed,
            linear_damping: def.linear_damping,
            angular_damping: def.angular_damping,
        };
        Body { def, motion, frozen: true, tracked: false, cool_down: 0.0, last_energy: 0.0 }
    }

    pub fn cylinder(&self) -> CylinderGp {
        let q = self.motion.orientation;
        CylinderGp {
            center: self.motion.position + q * self.def.offset,
            axis: q * self.def.axis,
            half_height: self.def.half_height,
            radius: self.def.radius,
            fat: self.def.fat,
        }
    }

    pub fn energy(&self) -> f32 {
        let m = &self.motion;
        let mass = 1.0 / m.inv_mass;
        let wb = m.orientation.inverse() * m.angular_velocity;
        let rot = wb * wb * (Vec3::splat(mass) / m.inv_inertia);
        0.5 * mass * m.velocity.length_squared() + 0.5 * (rot.x + rot.y + rot.z)
    }

    pub fn push(&mut self, linear: Vec3, angular: Vec3) {
        self.frozen = false;
        self.motion.velocity += linear;
        self.motion.angular_velocity += angular;
    }
}

pub fn freeze_test(b: &mut Body, dt: f32) -> bool {
    let e = b.energy();
    let mut t = b.cool_down;
    let freeze = if e > ACTIVATION_ENERGY {
        t = MAX_COOL_DOWN;
        false
    } else {
        if !(t > 0.0) {
            t = dt;
        }
        if !(e - b.last_energy > COOLING_TOLERANCE) {
            t = (t - dt).max(0.0);
        }
        t <= 0.0
    };
    b.cool_down = t;
    b.last_energy = e;
    freeze
}

#[derive(Clone, Debug, Default)]
pub struct RigidWorld {
    pub bodies: Vec<Body>,
    pub touches: Vec<(usize, usize)>,
    pub touch_count: u32,
}

impl RigidWorld {
    pub fn step(&mut self, dt: f32, geo: &CollisionWorld) {
        for b in self.bodies.iter_mut().filter(|b| !b.frozen && b.tracked) {
            if freeze_test(b, dt) {
                (b.frozen, b.tracked) = (true, false);
                b.motion.velocity = Vec3::ZERO;
                b.motion.angular_velocity = Vec3::ZERO;
            }
        }
        let pairs = body_pairs(&self.bodies, dt);
        self.touches = pairs.iter().map(|p| (p.0, p.1)).collect();
        self.touch_count += self.touches.len() as u32;
        let mut island: Vec<usize> = (0..self.bodies.len()).collect();
        fn root(island: &mut [usize], i: usize) -> usize {
            let mut r = i;
            while island[r] != r {
                r = island[r];
            }
            island[i] = r;
            r
        }
        for &(i, j, _) in &pairs {
            if !self.bodies[i].frozen && !self.bodies[j].frozen {
                let (ri, rj) = (root(&mut island, i), root(&mut island, j));
                island[ri.max(rj)] = ri.min(rj);
            }
        }
        let active: Vec<usize> = (0..self.bodies.len()).filter(|&i| !self.bodies[i].frozen).collect();
        let mut done = vec![false; self.bodies.len()];
        for &i in &active {
            if done[i] {
                continue;
            }
            let r = root(&mut island, i);
            let members: Vec<usize> = active.iter().copied().filter(|&k| root(&mut island, k) == r).collect();
            let slot = |k: usize| members.iter().position(|&x| x == k);
            let mut links = Vec::new();
            for (pi, pj, m) in &pairs {
                let (si, sj) =
                    (slot(*pi).filter(|_| !self.bodies[*pi].frozen), slot(*pj).filter(|_| !self.bodies[*pj].frozen));
                if si.is_none() && sj.is_none() {
                    continue;
                }
                let [ms_i, md_i, e_i] = self.bodies[*pi].def.material;
                let [ms_j, md_j, e_j] = self.bodies[*pj].def.material;
                let (oi, oj) = (self.bodies[*pi].motion.position, self.bodies[*pj].motion.position);
                let static_frame = match (si, sj) {
                    (None, _) => self.bodies[*pi].motion.orientation,
                    (_, None) => self.bodies[*pj].motion.orientation,
                    _ => Quat::IDENTITY,
                };
                for &(pa, pb) in &m.points {
                    let contact = solver::Contact {
                        mu_s: ms_i.max(ms_j),
                        mu_d: md_i.max(md_j),
                        restitution: e_i.min(e_j),
                        normal: m.normal,
                        r_a: if si.is_some() { pa - oi } else { pa },
                        r_b: if sj.is_some() { pb - oj } else { pb },
                        static_frame,
                    };
                    links.push(solver::Link { a: si, b: sj, contact });
                }
            }
            for (k, &bi) in members.iter().enumerate() {
                let b = &self.bodies[bi];
                for contact in contacts(b, geo, padding(b, dt)) {
                    links.push(solver::Link { a: None, b: Some(k), contact });
                }
            }
            if members.len() == 1 && links.iter().all(|l| l.a.is_none() || l.b.is_none()) {
                let contacts: Vec<solver::Contact> = links
                    .iter()
                    .map(|l| match l.b {
                        Some(_) => l.contact,
                        None => solver::Contact {
                            normal: -l.contact.normal,
                            r_a: l.contact.r_b,
                            r_b: l.contact.r_a,
                            ..l.contact
                        },
                    })
                    .collect();
                solver::step(&mut self.bodies[members[0]].motion, &contacts, dt, GRAVITY, MAX_ITERATIONS);
            } else {
                let mut ms: Vec<solver::Motion> = members.iter().map(|&k| self.bodies[k].motion).collect();
                solver::step_partition(&mut ms, &links, dt, GRAVITY, MAX_ITERATIONS);
                for (k, &bi) in members.iter().enumerate() {
                    self.bodies[bi].motion = ms[k];
                }
            }
            for &k in &members {
                done[k] = true;
            }
        }
    }
}

fn padding_parts(b: &Body, dt: f32) -> (f32, f32) {
    let m = &b.motion;
    let l1 = |v: Vec3| v.x.abs() + v.y.abs() + v.z.abs();
    let lin = l1(m.velocity * dt).min(m.max_speed * dt);
    let (lo, hi) = cylinder_box(&b.cylinder());
    let rot = 0.5 * l1(m.angular_velocity * dt).min(m.max_angular_speed * dt).min(1.0) * l1(hi - lo);
    (lin, rot)
}

pub fn body_pairs(bodies: &[Body], dt: f32) -> Vec<(usize, usize, narrow::Manifold)> {
    let mut out = Vec::new();
    for i in 0..bodies.len() {
        for j in i + 1..bodies.len() {
            let (a, b) = (&bodies[i], &bodies[j]);
            if a.frozen && b.frozen {
                continue;
            }
            let (la, ra) = if a.frozen { (0.0, 0.0) } else { padding_parts(a, dt) };
            let (lb, rb) = if b.frozen { (0.0, 0.0) } else { padding_parts(b, dt) };
            let tol = (la + lb).max(ra + rb).max(MIN_PADDING);
            let (ca, cb) = (a.cylinder(), b.cylinder());
            let ((alo, ahi), (blo, bhi)) = (cylinder_box(&ca), cylinder_box(&cb));
            if (alo - Vec3::splat(tol)).cmpgt(bhi).any() || (ahi + Vec3::splat(tol)).cmplt(blo).any() {
                continue;
            }
            if let Some(m) = cylcyl::cylinder_cylinder(&ca, &cb, tol) {
                // TODO(guess): with more than one body pair the pairs' order (here index order) is not the game's (a hash order of the parts' handles, 0x01000000 + the insertion count; emulated: A is always the earlier-inserted body, four flares give (1,3), (2,3), (0,3), (1,2), (0,2), (0,1), and parts inserted before them change it);
                // settle: the pair table that 0x831FED48 fills and `FUN_82B113F0` walks (its hash and size), and the level's insertion order of its parts.
                crate::guess_hit!();
                out.push((i, j, m));
            }
        }
    }
    out
}

pub fn cylinder_box(c: &CylinderGp) -> (Vec3, Vec3) {
    let k = c.axis.abs();
    let s = (Vec3::ONE - k * k).max(Vec3::ZERO);
    let ext = k * c.half_height + Vec3::new(s.x.sqrt(), s.y.sqrt(), s.z.sqrt()) * c.radius + Vec3::splat(c.fat);
    (c.center - ext, c.center + ext)
}

pub fn padding(b: &Body, dt: f32) -> f32 {
    let (lin, rot) = padding_parts(b, dt);
    lin.max(rot).max(MIN_PADDING)
}

// TODO(guess): the order across parts (cluster index order) and inside a prop's clustered mesh (storage order) are not the decoded ones (the pair list 0x831FED48 / `FUN_82B113F0`, the mesh query 0x82EEF428); settle: the pair generation and that query, run in the emulator.
pub fn contacts(b: &Body, geo: &CollisionWorld, tolerance: f32) -> Vec<Contact> {
    crate::guess_hit!();
    let c = b.cylinder();
    let (lo, hi) = cylinder_box(&c);
    let (min, max) = (lo - Vec3::splat(tolerance), hi + Vec3::splat(tolerance));
    let mut out = Vec::new();
    let [bs, bd, be] = b.def.material;
    for ci in geo.clusters_in_box(min, max) {
        let cl = &geo.clusters[ci as usize];
        let [gs, gd, ge] =
            geo.cluster_material.get(ci as usize).copied().unwrap_or(crate::collision::DEFAULT_PART_MATERIAL);
        let static_frame = geo.cluster_frame.get(ci as usize).copied().unwrap_or(Quat::IDENTITY);
        let widen = Vec3::splat(if cl.terrain { TERRAIN_QUERY_WIDENING } else { 0.0 });
        let (qmin, qmax) = (min - widen, max + widen);
        for local in geo.query_order(ci) {
            let ti = cl.tri_start + local;
            let v = geo.triangle_vertices(ci, ti).map(|i| geo.vertices[i]);
            let (tmin, tmax) = (v[0].min(v[1]).min(v[2]), v[0].max(v[1]).max(v[2]));
            if tmin.cmpgt(qmax).any() || tmax.cmplt(qmin).any() {
                continue;
            }
            let e = geo.triangle_edges(ci, ti);
            let t = TriangleGp { v, radius: 0.0, flags: e.flags, edge_cos: e.edge_cos };
            let Some(m) = narrow::cylinder_triangle(&c, &t, tolerance) else { continue };
            for (tp, cp) in m.points {
                out.push(Contact {
                    mu_s: bs.max(gs),
                    mu_d: bd.max(gd),
                    restitution: be.min(ge),
                    normal: m.normal,
                    r_a: tp,
                    r_b: cp - b.motion.position,
                    static_frame,
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flare() -> RigidBodyDef {
        RigidBodyDef {
            offset: Vec3::new(-0.19, 0.0373, 0.0),
            axis: Vec3::X,
            half_height: 0.18,
            radius: 0.04,
            fat: 0.0,
            inv_mass: 1.0,
            inv_inertia: Vec3::new(1250.0, 89.2857, 89.2857),
            max_speed: 1e32,
            max_angular_speed: 100.0,
            linear_damping: 0.0,
            angular_damping: 0.0,
            material: [0.2, 0.2, 0.1],
        }
    }

    #[test]
    fn the_contact_distance_takes_the_l1_step_or_the_rotation() {
        let mut b = Body::new(flare(), Vec3::ZERO, Quat::IDENTITY);
        b.motion.velocity = Vec3::new(3.0, -4.0, 0.0);
        assert!((padding(&b, 1.0 / 30.0) - 7.0 / 30.0).abs() < 1e-6);
        b.motion.velocity = Vec3::new(0.0, -2.0, 0.0);
        assert_eq!(padding(&b, 1.0 / 30.0), MIN_PADDING);
        b.motion.velocity = Vec3::new(0.0, -6.0, 0.0);
        b.motion.angular_velocity = Vec3::new(0.0, 0.0, 50.0);
        assert!((padding(&b, 1.0 / 30.0) - 0.26).abs() < 1e-5);
    }

    #[test]
    fn a_resting_body_freezes_after_the_cool_down() {
        let mut b = Body::new(flare(), Vec3::ZERO, Quat::IDENTITY);
        b.frozen = false;
        b.motion.velocity = Vec3::X;
        assert!(!freeze_test(&mut b, 1.0 / 30.0), "0.5 J > 0.2: the cool-down restarts");
        assert_eq!(b.cool_down, MAX_COOL_DOWN);
        b.motion.velocity = Vec3::ZERO;
        let steps = (1..=20).find(|_| freeze_test(&mut b, 1.0 / 30.0)).unwrap();
        assert_eq!(steps, 10);
    }

    #[test]
    fn a_dropped_flare_lands_and_comes_to_rest_on_flat_ground() {
        let mut geo = CollisionWorld::default();
        let v = [Vec3::new(-50.0, 0.0, -50.0), Vec3::new(-50.0, 0.0, 50.0), Vec3::new(50.0, 0.0, 0.0)];
        geo.push_cluster(&v, None, None, &[[0, 1, 2]], 0, 0);
        geo.finish();
        let mut b = Body::new(flare(), Vec3::new(0.0, 0.5, 0.0), Quat::IDENTITY);
        b.push(Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 2.0, 0.0));
        let mut tracked = b;
        tracked.tracked = true;
        let mut w = RigidWorld { bodies: vec![tracked, b], ..Default::default() };
        w.bodies[1].motion.position.z = 5.0;
        for _ in 0..300 {
            w.step(1.0 / 30.0, &geo);
        }
        let u = &w.bodies[1];
        assert!(!u.frozen && u.motion.velocity.length() < 0.05, "a kicked flare at rest stays active: {:?}", u.motion);
        let b = &w.bodies[0];
        assert!(b.frozen, "at rest: {:?}", b.motion);
        let c = b.cylinder();
        let k = c.axis.y.abs();
        let lowest = c.center.y - (k * c.half_height + (1.0 - k * k).max(0.0).sqrt() * c.radius);
        assert!(lowest.abs() < 0.01, "{c:?}");
    }
}
