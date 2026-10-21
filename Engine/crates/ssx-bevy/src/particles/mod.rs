pub mod geotag;
pub mod inputs;
pub mod light;
mod load;
pub mod render;
pub mod rider;
pub mod shadow_query;
pub mod sim;
pub mod spray;
pub mod tombstone;
pub mod track;
mod update;
pub mod weather;

use std::sync::Arc;

use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use ssx_assets::particles::ParticleNode;
use ssx_formats::lynx::Behaviour;

use crate::particles::sim::{Draw, EffectInstance, Rng, SetArena};
use crate::particles::spray::SnowSprayDriver;
use crate::particles::track::{DefCache, TrackEffects};
use crate::particles::weather::WeatherManager;
use crate::render::post::GamePostCamera;
use crate::ride::Ride;
use load::build;
use update::update;

pub struct ParticlesPlugin;

impl Plugin for ParticlesPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ExtractResourcePlugin::<ParticleFrame>::default()).init_resource::<ParticleFrame>();
        crate::shell::level::scoped::<ParticleWorld>(app);
        if crate::render::post::env_is("SSX_PARTICLES", "off") {
            return;
        }
        app.add_systems(Update, sample_depth)
            .add_systems(PostUpdate, update.after(bevy::transform::TransformSystems::Propagate));
        crate::particles::render::build(app);
        shadow_query::plugin(app);
    }
}

pub fn reset_level(world: &mut World) {
    world.insert_resource(ParticleFrame::default());
}

pub struct ParticleTexture {
    pub name: String,
    pub size: UVec2,
    pub levels: Vec<Vec<u8>>,
}

#[derive(Resource, Clone, ExtractResource)]
pub struct ParticleFrame {
    pub draws: Arc<Vec<Draw>>,
    pub textures: Arc<Vec<ParticleTexture>>,
    pub light: crate::render::lighting::game::GameLightUniform,
    pub global_light: ParticleLight,
    pub serial: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct ParticleLight {
    pub position: Vec3,
    pub colour: Vec4,
    pub radius: f32,
    pub falloff: f32,
}

impl Default for ParticleLight {
    fn default() -> Self {
        ParticleLight { position: Vec3::ZERO, colour: Vec4::ZERO, radius: 1.0, falloff: 1.0 }
    }
}

impl Default for ParticleFrame {
    fn default() -> Self {
        ParticleFrame {
            draws: Default::default(),
            textures: Default::default(),
            light: crate::render::lighting::game::GameLightUniform::viewer_default(),
            global_light: ParticleLight::default(),
            serial: 0,
        }
    }
}

struct SlotEffect {
    node: ParticleNode,
    instance: EffectInstance,
    behaviour: Option<Behaviour>,
    start: f32,
    stop: f32,
    instant: bool,
}

struct Spray {
    effects: Vec<SlotEffect>,
    table: [[Option<usize>; 6]; 3],
    driver: SnowSprayDriver,
    inputs: crate::particles::inputs::Trackers,
}

struct TrailSlot {
    set: usize,
    chunk: usize,
    node: ParticleNode,
    instance: EffectInstance,
    start: f32,
    stop: f32,
    blend: Vec<(u8, f32)>,
}

struct SunSlot {
    node: ParticleNode,
    instance: EffectInstance,
    dir: Vec3,
}

const SUN_DISTANCE: f32 = 500.0;

#[derive(Resource)]
struct ParticleWorld {
    spray: Option<Spray>,
    bot_sprays: Vec<Spray>,
    sun: Option<SunSlot>,
    rider: Option<rider::RiderEffects>,
    rider_tried: bool,
    heli: Option<rider::RiderEffects>,
    heli_tried: bool,
    defs: DefCache,
    texture_names: Vec<String>,
    trails: Vec<TrailSlot>,
    geotags: geotag::GeotagSlots,
    tombstones: tombstone::TombstoneSlots,
    track: TrackEffects,
    weather: WeatherManager,
    arena: SetArena,
    rng: Rng,
    clock: f32,
    last_render_time: f32,
    frame: u32,
    dt_smoothed: f32,
    last_step: u64,
    pending: f32,
    textures: Arc<Vec<ParticleTexture>>,
    registry: std::collections::BTreeMap<String, Vec4>,
    registry_snapshot: Option<Arc<Vec<(String, Vec4)>>>,
    light: Option<crate::particles::light::NearestLight>,
}

pub struct LoadedParticles(ParticleWorld);

impl LoadedParticles {
    pub fn insert(self, world: &mut World) {
        world.insert_resource(self.0);
    }
}

pub fn load_level(
    root: &std::path::Path,
    ride: Option<&Ride>,
    lighting: Option<&crate::render::lighting::GameLighting>,
    avalanche: Option<&crate::render::avalanche::AvalancheScene>,
) -> Option<LoadedParticles> {
    if crate::render::post::env_is("SSX_PARTICLES", "off") {
        return None;
    }
    Some(LoadedParticles(build(root, ride, lighting, avalanche)))
}

fn sample_depth(mut cameras: Query<&mut Camera3d, With<GamePostCamera>>) {
    for mut cam in &mut cameras {
        let usage = bevy::render::render_resource::TextureUsages::from(cam.depth_texture_usages);
        if !usage.contains(bevy::render::render_resource::TextureUsages::TEXTURE_BINDING) {
            cam.depth_texture_usages = (usage | bevy::render::render_resource::TextureUsages::TEXTURE_BINDING).into();
        }
    }
}
