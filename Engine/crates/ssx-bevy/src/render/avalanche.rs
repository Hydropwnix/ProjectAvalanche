use std::path::Path;

use anyhow::{Context, Result};
use bevy::mesh::MeshTag;
use bevy::prelude::{
    App, Assets, Commands, Component, Entity, FixedPostUpdate, Image, IntoScheduleConfigs, Mesh, Mesh3d,
    MeshMaterial3d, Name, Quat, Query, Res, ResMut, Resource, Transform, Vec3, Visibility, With, info, resource_exists,
    warn,
};
use ssx_assets::avalanche_entities::{self, AvalancheEntities};
use ssx_assets::particles::ParticleNode;
use ssx_physics::SIM_HZ;
use ssx_physics::avalanche::{Particle, ParticleTraits};

use crate::render::lighting::game::GameLightAssets;
use crate::render::objects::foreground::ForegroundMaterial;
use crate::render::objects::material::ObjectMaterial;
use crate::render::objects::{LoadedPart, MaterialHandle, load_part};
use crate::ride::Ride;

const FADE_TICKS: f32 = 15.0;
const EPS: f32 = 1.192_092_9e-7;

pub struct Mt19937 {
    state: [u32; 624],
    index: usize,
}

impl Mt19937 {
    pub fn new(seed: u32) -> Self {
        let mut state = [0u32; 624];
        state[0] = seed;
        for i in 1..624 {
            state[i] = 1_812_433_253u32.wrapping_mul(state[i - 1] ^ (state[i - 1] >> 30)).wrapping_add(i as u32);
        }
        Mt19937 { state, index: 624 }
    }

    pub fn next_u32(&mut self) -> u32 {
        if self.index >= 624 {
            for i in 0..624 {
                let y = (self.state[i] & 0x8000_0000) | (self.state[(i + 1) % 624] & 0x7FFF_FFFF);
                let mut v = self.state[(i + 397) % 624] ^ (y >> 1);
                if y & 1 != 0 {
                    v ^= 0x9908_B0DF;
                }
                self.state[i] = v;
            }
            self.index = 0;
        }
        let mut y = self.state[self.index];
        self.index += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9D2C_5680;
        y ^= (y << 15) & 0xEFC6_0000;
        y ^ (y >> 18)
    }

    pub fn below(&mut self, n: u32) -> u32 {
        ((u64::from(self.next_u32()) * u64::from(n)) >> 32) as u32
    }
}

pub fn bind_order(n: usize, rng: &mut Mt19937) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::with_capacity(n);
    for i in 0..n {
        let at = if i == 0 { 0 } else { rng.below(i as u32) as usize };
        out.insert(at, i);
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChunkPose {
    pub scale: f32,
    pub rotation: Quat,
    pub position: Vec3,
    pub shown: bool,
}

impl Default for ChunkPose {
    fn default() -> Self {
        ChunkPose { scale: 1.0, rotation: Quat::IDENTITY, position: Vec3::ZERO, shown: false }
    }
}

pub struct Chunk {
    pub model: usize,
    pub entity: Option<Entity>,
    pub prev: ChunkPose,
    pub curr: ChunkPose,
    pub weight: f32,
}

pub struct ChunkSet {
    pub sim_type: u8,
    pub row: u8,
    pub chunks: Vec<Chunk>,
}

pub struct ChunkModel {
    pub name: String,
    pub nodes: Vec<ParticleNode>,
    part: Option<LoadedPart>,
    bounds: (Vec3, Vec3),
}

#[derive(Resource)]
pub struct AvalancheScene {
    pub models: Vec<ChunkModel>,
    pub sets: Vec<ChunkSet>,
    pub steps: u64,
    pub serial: u64,
    pub shown: usize,
    last_ids: [Vec<u16>; 4],
}

fn event_avalanche_scripts(ride: &Ride) -> Vec<String> {
    let want = std::env::var("SSX_RIDE_EVENT").unwrap_or_else(|_| "race".into());
    if want == "none" {
        return Vec::new();
    }
    ssx_physics::event::choose_event(ride.world().events.iter(), &want)
        .map_or_else(Vec::new, |d| d.avalanche_scripts.clone())
}

impl AvalancheScene {
    pub fn load(big_root: &Path, track: &str, ride: &Ride, spray: bool) -> Result<AvalancheScene> {
        let scripts = event_avalanche_scripts(ride);
        let AvalancheEntities { mut models, mut sets, .. } = avalanche_entities::load(big_root, track, &scripts)?;
        if spray {
            let chunks = ssx_assets::particles::spray_chunks::load(big_root, track)?;
            let base = models.len();
            models.extend(chunks.models);
            sets.extend(chunks.sets.into_iter().map(|mut s| {
                s.entities.iter_mut().for_each(|e| *e += base);
                s
            }));
        }
        let mut loaded = Vec::with_capacity(models.len());
        let content = ssx_assets::content::GameContent::of(big_root);
        for m in &models {
            let id = ssx_assets::content::models::model_id(&m.geometry)
                .filter(|id| content.has(id))
                .with_context(|| format!("{} not found under {}", m.geometry, big_root.display()))?;
            let material = content.material_for(&m.material, ssx_assets::content::resolve::MaterialUse::Objects);
            let part = load_part(big_root, &id, ssx_assets::content::resolve::ModelUse::Static, &material)
                .with_context(|| m.geometry.clone())?;
            let bounds = part.bounds;
            loaded.push(ChunkModel { name: m.schema.clone(), nodes: m.nodes.clone(), part: Some(part), bounds });
        }
        // TODO(guess): a fixed seed (5489) for the bind's shuffle instead of the game's per-play one (its MT19937 is seeded from the CPU time base: the global generator `GlobalRandom_Below` 0x831500D0 draws from is created with seed −1 = `mftb` and reseeded at once from a second time-seeded twister); settle: none in the game.
        let mut mt = Mt19937::new(5489);
        let sets = sets
            .iter()
            .map(|s| {
                let order = bind_order(s.entities.len(), &mut mt);
                let chunks = order
                    .iter()
                    .map(|&i| Chunk {
                        model: s.entities[i],
                        entity: None,
                        prev: ChunkPose::default(),
                        curr: ChunkPose::default(),
                        weight: 0.0,
                    })
                    .collect();
                ChunkSet { sim_type: s.binding.sim_type, row: s.binding.row, chunks }
            })
            .collect();
        Ok(AvalancheScene { models: loaded, sets, steps: 0, serial: 0, shown: 0, last_ids: Default::default() })
    }

    pub fn summary(&self) -> String {
        let sets: Vec<String> =
            self.sets.iter().map(|s| format!("type {} row {} x {}", s.sim_type, s.row, s.chunks.len())).collect();
        let models: Vec<String> = self
            .models
            .iter()
            .map(|m| format!("{} ({} trail nodes, bounds {:.2}..{:.2})", m.name, m.nodes.len(), m.bounds.0, m.bounds.1))
            .collect();
        format!("mesh particle entities: {} | {}", sets.join(", "), models.join("; "))
    }
}

pub fn record_pose(p: &Particle, tr: &ParticleTraits) -> (ChunkPose, f32) {
    let (roll, lift) = (p.roll, p.lift);
    let dur = tr.spawn_offset_duration;
    let (dur, inv_dur, depth) = if dur.abs() > EPS { (dur, 1.0 / dur, tr.spawn_offset_depth) } else { (0.0, 0.0, 0.0) };
    let f = ((dur - p.age) * inv_dur).clamp(0.0, 1.0);
    let position = p.pos + p.frame * Vec3::new(0.0, lift - f * depth, 0.0);
    let scale = p.scale * ((tr.max_lifetime - p.age) / FADE_TICKS).clamp(0.0, 1.0);
    let weight = (p.age / tr.max_lifetime).clamp(0.0, 1.0);
    let rotation = (p.frame * roll).normalize();
    (ChunkPose { scale, rotation, position, shown: false }, weight)
}

impl AvalancheScene {
    pub fn step(&mut self, ride: &Ride) {
        if ride.steps == self.steps {
            return;
        }
        let restart = ride.steps < self.steps || self.steps == 0;
        let sim = &ride.session.avalanche;
        let world = ride.world();
        let set = ride.state.avalanche.switches.tuning_set;
        let tuning = world.tuning.avalanche.particles(set);
        let board = [ride.state.board_particle.record];
        let records: [(&[Particle], &ParticleTraits); 4] = [
            (&sim.spray, &tuning.spray),
            (&board, world.tuning.board_particle.traits(set)),
            (&sim.sloughs, &tuning.slough),
            (&sim.slabs, &tuning.slab),
        ];
        let mut shown = 0;
        for s in self.sets.iter_mut() {
            let Some(&(recs, traits)) = records.get(s.sim_type as usize) else { continue };
            let last = &self.last_ids[s.sim_type as usize];
            let n = recs.len().min(s.chunks.len());
            let mut k = 0;
            for (j, r) in recs[..n].iter().enumerate() {
                if r.row != s.row {
                    continue;
                }
                let c = &mut s.chunks[k];
                k += 1;
                c.prev = if restart { ChunkPose::default() } else { c.curr };
                if r.id == 0 {
                    (c.curr, c.weight) = (ChunkPose::default(), 0.0);
                    continue;
                }
                let (mut pose, w) = record_pose(r, traits);
                pose.shown = !restart && last.get(j) == Some(&r.id);
                shown += usize::from(pose.shown);
                (c.curr, c.weight) = (pose, w);
            }
            for c in &mut s.chunks[k..] {
                c.prev = if restart { ChunkPose::default() } else { c.curr };
                c.curr.shown = false;
            }
        }
        for (ids, (recs, _)) in self.last_ids.iter_mut().zip(records) {
            *ids = recs.iter().map(|p| p.id).collect();
        }
        (self.steps, self.serial, self.shown) = (ride.steps, self.serial + 1, shown);
    }
}

#[derive(Component)]
pub struct AvalancheChunk;

pub fn register(app: &mut App) {
    app.add_systems(crate::shell::level::LevelStartup, spawn_chunks.run_if(resource_exists::<AvalancheScene>))
        .add_systems(FixedPostUpdate, step_chunks.run_if(resource_exists::<AvalancheScene>));
    crate::shell::level::scoped::<AvalancheScene>(app);
}

pub fn load(big_root: &Path, track: &str, ride: &Ride) -> Option<AvalancheScene> {
    if crate::render::post::env_is("SSX_AVALANCHE_DRAW", "off") {
        info!("avalanche: entities not drawn (SSX_AVALANCHE_DRAW=off)");
        return None;
    }
    let spray = !crate::render::post::env_is("SSX_SPRAY_CHUNKS", "off");
    match AvalancheScene::load(big_root, track, ride, spray) {
        Ok(scene) => {
            info!("{}", scene.summary());
            Some(scene)
        }
        Err(e) => {
            warn!("avalanche: entities unavailable: {e:#}");
            None
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_chunks(
    mut commands: Commands,
    mut scene: ResMut<AvalancheScene>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut objects: ResMut<Assets<ObjectMaterial>>,
    mut foreground: ResMut<Assets<ForegroundMaterial>>,
    light: Res<GameLightAssets>,
) {
    let scene = &mut *scene;
    let mut handles = Vec::with_capacity(scene.models.len());
    for m in &mut scene.models {
        let Some(part) = m.part.take() else {
            handles.push(None);
            continue;
        };
        for p in &part.problems {
            warn!("avalanche {}: {p}", m.name);
        }
        let (mesh, material) = part.add(&mut meshes, &mut images, &light, &mut objects, &mut foreground);
        handles.push(match material {
            MaterialHandle::Object(h) => Some((mesh, h)),
            MaterialHandle::Foreground(_) => None,
        });
    }
    for set in &mut scene.sets {
        for (k, c) in set.chunks.iter_mut().enumerate() {
            let Some((mesh, material)) = handles[c.model].clone() else { continue };
            // TODO(guess): the entity's `WorldAo` (the rider's AO) is not wired into the object shader, slot 0 of the placement buffer (AO 1) stands in; settle: trace 0x82987148's per-object list into `instanceAOMult` (0x829C1D10).
            let id = commands
                .spawn((
                    Name::new(format!(
                        "mesh particle {}/{} {k} ({})",
                        set.sim_type, set.row, scene.models[c.model].name
                    )),
                    AvalancheChunk,
                    Mesh3d(mesh),
                    MeshMaterial3d(material),
                    MeshTag::new(0),
                    Transform::default(),
                    Visibility::Hidden,
                ))
                .id();
            c.entity = Some(id);
        }
    }
}

fn step_chunks(ride: Res<Ride>, scene: Option<ResMut<AvalancheScene>>) {
    if let Some(mut s) = scene {
        s.step(&ride);
    }
}

pub fn update(
    ride: Res<Ride>,
    scene: Option<ResMut<AvalancheScene>>,
    mut chunks: Query<(&mut Transform, &mut Visibility), With<AvalancheChunk>>,
) {
    let Some(mut scene) = scene else { return };
    let serial = scene.serial;
    scene.step(&ride);
    if scene.serial != serial && scene.steps % 30 == 0 && crate::render::post::env_is("SSX_AVALANCHE_LOG", "on") {
        let av = &ride.state.avalanche;
        info!(
            "avalanche t {:.1}: {} entities shown | aggregate mass {:.1} at {:.1} ({:.1} m), probe {:.2}, A {:.2}, \
             tumbling {} | rider {:.1}",
            ride.seconds(),
            scene.shown,
            av.published.aggregate.mass,
            av.published.aggregate_pos,
            av.published.aggregate_pos.distance(ride.state.traj),
            av.published.probe.mass,
            av.frame.value,
            ride.state.landing.bail_on_landing && av.bailed_because_of_avalanche,
            ride.state.traj,
        );
    }
    let t = (ride.render_time * SIM_HZ - (scene.steps as f32 - 1.0)).clamp(0.0, 1.0);
    for set in &scene.sets {
        for c in &set.chunks {
            let Some((mut tr, mut vis)) = c.entity.and_then(|e| chunks.get_mut(e).ok()) else { continue };
            if !c.curr.shown {
                if *vis != Visibility::Hidden {
                    *vis = Visibility::Hidden;
                }
                continue;
            }
            let (a, b) = (&c.prev, &c.curr);
            let q = if a.rotation.dot(b.rotation) >= 0.0 { b.rotation } else { -b.rotation };
            *tr = Transform {
                translation: a.position.lerp(b.position, t),
                rotation: a.rotation.lerp(q, t).normalize(),
                scale: Vec3::splat(a.scale + (b.scale - a.scale) * t),
            };
            *vis = Visibility::Inherited;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mt19937_matches_the_reference() {
        let mut mt = Mt19937::new(5489);
        assert_eq!(mt.next_u32(), 3_499_211_612);
        let last = (1..10_000).map(|_| mt.next_u32()).last();
        assert_eq!(last, Some(4_123_659_995));
    }

    #[test]
    fn bind_order_is_a_permutation() {
        let mut mt = Mt19937::new(1);
        let o = bind_order(240, &mut mt);
        let mut sorted = o.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..240).collect::<Vec<_>>());
        assert_ne!(o.last(), Some(&239));
    }
}
