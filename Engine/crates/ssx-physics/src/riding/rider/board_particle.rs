use anyhow::Result;
use glam::{Quat, Vec3};

use crate::avalanche::{
    AvalancheSim, FLAG_FROZEN, FLAG_ROLLING, FLAG_SIMULATED, Particle, ParticleTraits, Rng, sweep, update_record,
};
use crate::collision::CollisionWorld;
use crate::tuning::Bundle;

pub const RECOVER_CLEAR_TICKS: f32 = 40.0;

pub const DEFAULT_ORIENTATION: Quat = Quat::from_xyzw(0.0, 0.0, 1.0, 0.0);

const EPS: f32 = 1.192_092_9e-7;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BoardParticleTuning {
    pub traits: [ParticleTraits; 3],
    pub surface_epsilon: f32,
    pub culling_distance: f32,
}

impl BoardParticleTuning {
    pub fn load(bundle: &Bundle) -> Result<Self> {
        let p = bundle.view("particle_system", "ParticleSystemAsset")?;
        let board = |set: &str| ParticleTraits::load(&p.asset(set)?.asset("ParticleTraitsBoard")?);
        Ok(Self {
            traits: [board("ParticleTuningLarge")?, board("ParticleTuningMedium")?, board("ParticleTuningSmall")?],
            surface_epsilon: p.f32("SurfaceEpsilon")?,
            culling_distance: p.f32("CullingDistance")?,
        })
    }

    pub fn data() -> Self {
        let tr = ParticleTraits {
            gravity: 20.0,
            h_terminal: 80.0,
            v_terminal: 80.0,
            restitution: 0.1,
            stop_friction: 0.4,
            roll_friction: 0.1,
            energy_preservation: 0.75,
            stopping_speed: 0.0,
            surface_noise: 0.002,
            roll_factor: 0.5,
            max_lifetime: 5_184_000.0,
            max_lifetime_frozen: 60.0,
            aging_ground: 1.0,
            aging_air: 1.0,
            spawn_offset_depth: 1.0,
            spawn_offset_duration: 0.0,
        };
        Self { traits: [tr; 3], surface_epsilon: 0.01, culling_distance: 80.0 }
    }

    pub fn traits(&self, set: i32) -> &ParticleTraits {
        match set {
            2 => &self.traits[1],
            3 => &self.traits[2],
            _ => &self.traits[0],
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BoardParticle {
    pub enabled: bool,
    pub position: Vec3,
    pub orientation: Quat,
    pub velocity: Vec3,
    pub placement: Option<(Vec3, Quat)>,
    pub stay_dead: bool,
    pub record: Particle,
    pub alive: bool,
    actor: Vec3,
    dt: f32,
    hit: Option<(Vec3, Vec3)>,
    rng: Rng,
    pub spawns: u32,
}

impl Default for BoardParticle {
    fn default() -> Self {
        Self {
            enabled: false,
            position: Vec3::ZERO,
            orientation: DEFAULT_ORIENTATION,
            velocity: Vec3::ZERO,
            placement: None,
            stay_dead: false,
            record: Particle::default(),
            alive: false,
            actor: Vec3::ZERO,
            dt: 0.0,
            hit: None,
            rng: Rng::default(),
            spawns: 0,
        }
    }
}

impl BoardParticle {
    pub fn carving_update(
        &mut self,
        sliding: bool,
        dead_on_ground: bool,
        board: (Vec3, Quat),
        global_velocity: Vec3,
    ) -> bool {
        if !sliding || dead_on_ground {
            return false;
        }
        let rose = !self.enabled;
        self.enabled = true;
        if rose {
            (self.position, self.orientation, self.velocity) = (board.0, board.1, global_velocity);
        }
        rose
    }

    pub fn recover_tag(&mut self, recover_ticks: f32) {
        if !self.stay_dead && recover_ticks >= RECOVER_CLEAR_TICKS {
            self.enabled = false;
        }
    }

    pub fn keep_feature(&mut self, live: BoardParticle) {
        (self.record, self.alive, self.actor, self.dt, self.hit) =
            (live.record, live.alive, live.actor, live.dt, live.hit);
        (self.rng, self.spawns) = (live.rng, live.spawns);
    }

    pub fn rig_op_view(&mut self) {
        self.placement = self.enabled.then_some((self.position, self.orientation));
    }

    pub fn dispatch(&mut self, world: &CollisionWorld, actor: Vec3, dt: f32) {
        (self.actor, self.dt) = (actor, dt);
        self.hit = sweep(&self.record, world, dt);
    }

    pub fn update_record(&mut self, t: &BoardParticleTuning, set: i32, sim: Option<&mut AvalancheSim>) {
        let dt = self.dt;
        if dt.abs() <= EPS {
            return;
        }
        let tr = *t.traits(set);
        let mut own = self.rng;
        let rng = match sim {
            Some(s) => &mut s.rng,
            None => &mut own,
        };
        let p = &mut self.record;
        if p.alive() {
            if p.flags & FLAG_SIMULATED != 0 && p.flags & FLAG_FROZEN == 0 {
                p.contact = self.hit;
            }
            let culling_sq = t.culling_distance * t.culling_distance;
            update_record(p, &tr, t.surface_epsilon, rng, self.actor, culling_sq, dt);
        }
        self.rng = own;
    }

    pub fn spawn_or_clear(&mut self, sim: Option<&mut AvalancheSim>) {
        if self.dt.abs() > EPS {
            if self.enabled {
                if !self.alive {
                    self.spawn(sim.map_or(1, |s| s.next_record_id()));
                }
            } else if self.alive {
                self.record = Particle::default();
                self.alive = false;
            }
        }
        let p = &self.record;
        if self.alive && p.alive() {
            self.position = p.pos + p.frame * Vec3::new(0.0, p.lift, 0.0);
            self.velocity = p.vel;
            self.orientation = p.frame * p.roll;
        }
    }

    pub fn gather(&mut self, t: &BoardParticleTuning, set: i32, mut sim: Option<&mut AvalancheSim>) {
        self.update_record(t, set, sim.as_deref_mut());
        self.spawn_or_clear(sim);
    }

    fn spawn(&mut self, id: u16) {
        self.record = Particle {
            pos: self.position,
            scale: 1.0,
            vel: self.velocity,
            frame: self.orientation,
            roll: Quat::IDENTITY,
            lift: 0.0,
            flags: FLAG_SIMULATED | FLAG_ROLLING,
            id,
            ..Particle::default()
        };
        self.alive = true;
        self.spawns += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tuning() -> BoardParticleTuning {
        BoardParticleTuning::data()
    }

    fn step(b: &mut BoardParticle, t: &BoardParticleTuning, world: &CollisionWorld, actor: Vec3) {
        b.dispatch(world, actor, 2.0);
        b.gather(t, 3, None);
    }

    #[test]
    fn the_flag_rises_once_and_keeps_its_start() {
        let mut b = BoardParticle::default();
        let q = Quat::from_rotation_y(0.3);
        assert!(!b.carving_update(false, false, (Vec3::ONE, q), Vec3::X));
        assert!(!b.carving_update(true, true, (Vec3::ONE, q), Vec3::X));
        assert!(!b.enabled);
        assert!(b.carving_update(true, false, (Vec3::ONE, q), Vec3::X));
        assert!(!b.carving_update(true, false, (Vec3::ZERO, Quat::IDENTITY), Vec3::Y));
        assert_eq!((b.position, b.orientation, b.velocity), (Vec3::ONE, q, Vec3::X));
        b.recover_tag(39.9);
        assert!(b.enabled);
        b.stay_dead = true;
        b.recover_tag(40.0);
        assert!(b.enabled);
        b.stay_dead = false;
        b.recover_tag(40.0);
        assert!(!b.enabled);
    }

    #[test]
    fn spawns_from_the_start_and_falls_freely() {
        let t = tuning();
        let world = CollisionWorld::default();
        let mut b = BoardParticle::default();
        let q = Quat::from_rotation_x(0.2);
        b.carving_update(true, false, (Vec3::new(0.0, 10.0, 0.0), q), Vec3::new(0.2, 0.0, 0.0));
        b.rig_op_view();
        assert_eq!(b.placement, Some((Vec3::new(0.0, 10.0, 0.0), q)));
        step(&mut b, &t, &world, Vec3::ZERO);
        assert!(b.alive && b.spawns == 1);
        assert_eq!(b.position, Vec3::new(0.0, 10.0, 0.0));
        assert!(b.orientation.dot(q) > 1.0 - 1e-6);
        step(&mut b, &t, &world, Vec3::ZERO);
        assert!(b.velocity.y < 0.0 && b.velocity.x > 0.19 && b.velocity.x < 0.2);
        assert!((b.position - Vec3::new(0.4, 10.0, 0.0)).length() < 1e-5);
        step(&mut b, &t, &world, Vec3::ZERO);
        assert!(b.position.y < 10.0 && b.position.x > 0.78);
        let x = b.record.frame * Vec3::X;
        assert!(x.dot(b.velocity.normalize()) > 0.999);
        assert!(b.orientation.is_finite());
    }

    #[test]
    fn clears_once_the_flag_is_off_and_keeps_the_last_output() {
        let t = tuning();
        let world = CollisionWorld::default();
        let mut b = BoardParticle::default();
        b.carving_update(true, false, (Vec3::ZERO, Quat::IDENTITY), Vec3::X);
        step(&mut b, &t, &world, Vec3::ZERO);
        step(&mut b, &t, &world, Vec3::ZERO);
        let last = b.position;
        b.recover_tag(40.0);
        step(&mut b, &t, &world, Vec3::ZERO);
        assert!(!b.alive && !b.record.alive());
        assert_eq!(b.position, last);
        b.dispatch(&world, Vec3::ZERO, 0.0);
        b.gather(&t, 3, None);
        assert!(!b.alive);
    }

    #[test]
    fn lands_slides_freezes_and_keeps_its_rest() {
        let t = tuning();
        let world = crate::testutil::plane(200.0, 0.0);
        let mut b = BoardParticle::default();
        b.carving_update(true, false, (Vec3::new(0.0, 1.0, 0.0), Quat::IDENTITY), Vec3::new(0.3, -0.05, 0.0));
        let (mut frozen_at, mut min_y) = (None, f32::INFINITY);
        for k in 0..400 {
            let at = b.position;
            step(&mut b, &t, &world, at);
            min_y = min_y.min(b.position.y);
            if frozen_at.is_none() && b.record.flags & FLAG_FROZEN != 0 {
                frozen_at = Some(k);
            }
        }
        let f = frozen_at.expect("the board comes to rest on flat ground");
        assert!(min_y > 0.0 && min_y < 0.02, "{min_y}");
        assert!(!b.record.alive() && b.alive, "culled after the frozen lifetime, +0x404 kept");
        assert!(b.position.x > 0.5 && b.position.y.abs() < 0.02 && b.velocity == Vec3::ZERO);
        assert!(f > 10, "{f}");
    }

    #[test]
    fn slides_down_a_slope() {
        let t = tuning();
        let slope = 0.5f32;
        let world = crate::testutil::plane(500.0, slope);
        let mut b = BoardParticle::default();
        b.carving_update(true, false, (Vec3::new(0.0, 0.5, 0.0), Quat::IDENTITY), Vec3::ZERO);
        for _ in 0..200 {
            let at = b.position;
            step(&mut b, &t, &world, at);
        }
        let n = Vec3::new(0.0, slope.cos(), slope.sin());
        assert!(b.record.alive() && b.velocity.z > 0.0 && b.velocity.length() > 0.01);
        assert!((b.record.frame * Vec3::Y).dot(n) > 0.99);
        let along = (b.velocity - n * b.velocity.dot(n)).normalize();
        assert!((b.record.frame * Vec3::X).dot(along) > 0.999);
        assert!(b.position.dot(n).abs() < 0.05);
    }

    #[test]
    fn culled_far_from_the_rider_without_a_respawn() {
        let t = tuning();
        let world = CollisionWorld::default();
        let mut b = BoardParticle::default();
        b.carving_update(true, false, (Vec3::ZERO, Quat::IDENTITY), Vec3::ZERO);
        step(&mut b, &t, &world, Vec3::ZERO);
        let last = b.position;
        step(&mut b, &t, &world, Vec3::new(100.0, 0.0, 0.0));
        assert!(b.alive && !b.record.alive() && b.spawns == 1);
        assert_eq!(b.position, last);
        step(&mut b, &t, &world, Vec3::ZERO);
        assert_eq!(b.spawns, 1);
    }
}
