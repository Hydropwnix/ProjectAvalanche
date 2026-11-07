use bevy::asset::{RenderAssetUsages, uuid_handle};
use bevy::math::Affine3A;
use bevy::prelude::*;
use bevy::render::storage::ShaderBuffer;
use bevy::shader::load_shader_library;
use ssx_assets::light_script::LightBehaviour;
use ssx_assets::track_lights::SceneLight;
use ssx_physics::controllers::{Controllers, LightUpdate, OfflineHost};

use crate::render::lighting::game::GameLightUniform;
use crate::render::lighting::{BigRoot, GameLighting};
use encode::{Words, assign_layers, bin_of, bounds, collect, encode, record};
use textures::{blank_array, texture_array};

mod encode;
mod textures;

pub const LOCAL_LIGHT_BUFFER: Handle<ShaderBuffer> = uuid_handle!("7c1d6a3e-2f4b-4e8a-9b51-0c3d2e1f4a77");
pub const LOCAL_LIGHT_TEXTURES: Handle<Image> = uuid_handle!("3e9b1c55-7a2d-4f0e-8c64-91d2a5b7e013");
pub const LOCAL_LIGHTS_BINDING: u32 = 140;

fn sphere_cos() -> f32 {
    (0.314_159_274_101_257_3_f64).cos() as f32
}

const CELL: f32 = 32.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Point = 0,
    Spot = 1,
    Box = 2,
    Textured = 3,
}

struct Placed {
    world: Affine3A,
    light: SceneLight,
    factor: Option<f32>,
    ctrl: Option<(u32, [u32; 3])>,
    node: Affine3A,
    scale: Vec3,
    order: (u32, u32, usize, usize, usize),
    behaviour: Option<LightBehaviour>,
    entity: u32,
    texture: Option<Projected>,
    texture_layer: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Projected {
    File(String),
    Text { string: String, colour: [u32; 3] },
}

const TEXT_LAYER: u32 = 8;

pub struct LocalLightsPlugin;

impl Plugin for LocalLightsPlugin {
    fn build(&self, app: &mut App) {
        load_shader_library!(app, "local_lights.wesl");
        app.init_resource::<DynamicLocalLights>()
            .init_resource::<DynamicLightReserve>()
            .init_resource::<LiveLights>()
            .add_systems(crate::shell::level::LevelPreStartup, setup_local_lights)
            .add_systems(PostUpdate, (follow_controllers, upload_dynamic_lights.in_set(DynamicLightsUpload)).chain());
        crate::shell::level::scoped::<LocalLightBytes>(app);
        use crate::render::lighting::fade::{LightingFadeSet, faded};
        app.add_systems(Update, follow_lighting.after(LightingFadeSet).run_if(faded));
    }
}

pub fn reset_level(world: &mut World) {
    world.insert_resource(DynamicLocalLights::default());
    world.insert_resource(DynamicLightReserve::default());
    world.insert_resource(LiveLights::default());
}

#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DynamicLightsUpload;

#[derive(Resource, Default)]
pub struct DynamicLightReserve(pub u32);

#[derive(Resource, Default)]
pub struct DynamicLocalLights {
    pub lights: Vec<DynamicLight>,
}

#[derive(Clone, Debug)]
pub struct DynamicLight {
    pub world: Affine3A,
    pub light: SceneLight,
    pub factor: f32,
}

#[derive(Resource)]
struct LocalLightBytes {
    words: Words,
    dynamic_at: usize,
    slots: u32,
    last_count: usize,
    last_clock: (u32, u32),
    animated: bool,
    placed: Vec<Placed>,
    light: GameLightUniform,
}

const LIGHT_WORDS: usize = 11;

fn light_words(l: &GameLightUniform) -> [[u32; 4]; LIGHT_WORDS] {
    let v = [
        l.ramp_lit,
        l.ramp_shade,
        l.ramp_bounce,
        l.light_dir,
        l.bounce_dir,
        l.rim_shadow,
        l.specular,
        l.fog_color,
        l.fog_data,
        l.sparkle,
        l.misc,
    ];
    v.map(|v| v.to_array().map(f32::to_bits))
}

fn follow_lighting(
    lighting: Res<GameLighting>,
    bytes: Option<ResMut<LocalLightBytes>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
) {
    let Some(mut bytes) = bytes else { return };
    bytes.light = GameLightUniform::from_preset(&lighting);
    let at = bytes.words.0[3][3] as usize;
    let block = light_words(&bytes.light);
    bytes.words.0[at..at + LIGHT_WORDS].copy_from_slice(&block);
    if let Some(mut buffer) = buffers.get_mut(&LOCAL_LIGHT_BUFFER) {
        buffer.clear();
        buffer.extend_from_slice(&bytes.words.bytes());
    }
}

#[derive(Resource, Default)]
pub struct LiveLights {
    offline: Option<(Box<Controllers>, OfflineHost)>,
    pending: f32,
    clock: (f32, u32),
    warned: bool,
}

pub const DYNAMIC_SLOTS: u32 = 4;

fn setup_local_lights(
    mut commands: Commands,
    reserve: Res<DynamicLightReserve>,
    lighting: Option<Res<GameLighting>>,
    root: Option<Res<BigRoot>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    mut images: ResMut<Assets<Image>>,
) {
    let t0 = std::time::Instant::now();
    let disabled = std::env::var("SSX_LOCAL_LIGHTS").is_ok_and(|v| v == "off");
    let lights = match (&lighting, &root, disabled) {
        (_, _, true) => Ok((Vec::new(), "local lights off (SSX_LOCAL_LIGHTS=off)".to_string())),
        (Some(l), Some(r), false) => match l.disc_track() {
            Some(track) => collect(&r.0, track),
            None => Ok((Vec::new(), "local lights: a map's new track places none".to_string())),
        },
        _ => Ok((Vec::new(), "local lights: no track lighting".to_string())),
    };
    let (mut placed, mut summary) = lights.unwrap_or_else(|e| {
        warn!("local lights: {e:#}");
        (Vec::new(), format!("local lights unavailable: {e:#}"))
    });
    let paths = assign_layers(&mut placed);
    let textures = match (&root, paths.is_empty()) {
        (Some(r), false) => texture_array(&r.0, &paths).map(Some),
        _ => Ok(None),
    };
    let image = match textures {
        Ok(Some(image)) => {
            if let Some(r) = &root {
                let ids = paths
                    .iter()
                    .filter_map(|p| match p {
                        Projected::File(rel) => ssx_assets::content::textures::texture_id(rel),
                        Projected::Text { .. } => None,
                    })
                    .collect();
                let (big, all) = (r.0.clone(), paths.clone());
                let recipe = crate::render::reload::Recipe::new(ids, move || texture_array(&big, &all));
                crate::render::reload::track(LOCAL_LIGHT_TEXTURES.id(), recipe);
            }
            image
        }
        Ok(None) => blank_array(),
        Err(e) => {
            warn!("local light textures: {e:#}");
            summary.push_str(" | projected textures undecodable");
            blank_array()
        }
    };
    let slots = DYNAMIC_SLOTS + reserve.0;
    let light = lighting.as_deref().map_or_else(GameLightUniform::viewer_default, GameLightUniform::from_preset);
    let (words, dynamic_at) = encode(&placed, slots, &light);
    if let Err(e) =
        buffers.insert(LOCAL_LIGHT_BUFFER.id(), ShaderBuffer::new(words.bytes(), RenderAssetUsages::default()))
    {
        warn!("local light buffer: {e:?}");
    }
    if let (Some(l), Some(r), false) = (&lighting, &root, disabled)
        && let Some(track) = l.disc_track()
    {
        let drop = (l.drop_count > 0).then_some(l.drop_key);
        match ssx_physics::spawn::load_trigger_volumes(&r.0, track, drop) {
            Ok(t) => {
                commands.insert_resource(LiveLights {
                    offline: Some((Box::new(Controllers::offline(&t)), OfflineHost::default())),
                    ..LiveLights::default()
                });
            }
            Err(e) => warn!("local lights: the track's entities: {e:#}"),
        }
    }
    commands.insert_resource(LocalLightBytes {
        words,
        dynamic_at,
        slots,
        last_count: 0,
        last_clock: (0, 0),
        animated: false,
        placed,
        light,
    });
    if let Err(e) = images.insert(LOCAL_LIGHT_TEXTURES.id(), image) {
        warn!("local light texture array: {e:?}");
    }
    info!("{summary} ({:.3} s)", t0.elapsed().as_secs_f32());
}

fn follow_controllers(
    ride: Option<Res<crate::ride::Ride>>,
    time: Res<Time>,
    mut live: ResMut<LiveLights>,
    bytes: Option<ResMut<LocalLightBytes>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
) {
    let Some(mut bytes) = bytes else { return };
    let live = &mut *live;
    let controllers: Option<&Controllers> = match ride.as_deref() {
        Some(r) => r.session.controllers.as_ref(),
        None => match live.offline.as_mut() {
            Some((c, host)) => {
                live.pending += time.delta_secs();
                let n = (live.pending * 30.0).floor();
                live.pending -= n / 30.0;
                for _ in 0..(n as u32).min(64) {
                    c.update_offline(1.0 / 30.0, host);
                }
                Some(&**c)
            }
            None => None,
        },
    };
    let mut state: std::collections::HashMap<(u32, [u32; 3]), LightBehaviour> = Default::default();
    let mut moved: std::collections::HashMap<(u32, [u32; 3]), (Vec3, Quat)> = Default::default();
    let (mut lo, mut hi) = ((f32::MAX, u32::MAX), (f32::MIN, 0u32));
    for e in controllers.map_or(&[][..], |c| &c.entities).iter().filter(|e| e.owner.is_none()) {
        if e.body.is_some() {
            moved.insert((e.object, e.placement_position.to_array().map(f32::to_bits)), (e.position, e.rotation));
        }
        let Some(run) = e.fx.lights.get("").filter(|r| !r.group && r.on) else { continue };
        let b = match run.update {
            LightUpdate::Blink { strength, frequency, gap } => LightBehaviour::Blink { strength, frequency, gap },
            LightUpdate::Flicker { strength } => LightBehaviour::Flicker { strength },
            _ => LightBehaviour::On,
        };
        state.insert((e.object, e.placement_position.to_array().map(f32::to_bits)), b);
        if matches!(b, LightBehaviour::Blink { .. } | LightBehaviour::Flicker { .. }) {
            (lo, hi) = ((lo.0.min(run.time), lo.1.min(run.updates)), (hi.0.max(run.time), hi.1.max(run.updates)));
        }
    }
    if lo.0 <= hi.0 {
        if (hi.0 - lo.0 > 1e-6 || hi.1 != lo.1) && !live.warned {
            warn!(
                "local lights: the running light tracks' clocks differ ({lo:?} … {hi:?}); the shader takes the latest"
            );
            live.warned = true;
        }
        live.clock = hi;
    }
    let mut changed = false;
    for p in bytes.placed.iter_mut() {
        if let Some(k) = p.ctrl {
            let b = Some(state.get(&k).copied().unwrap_or(LightBehaviour::Off));
            changed |= p.behaviour != b;
            p.behaviour = b;
            if let Some(&(position, rotation)) = moved.get(&k) {
                let world = Affine3A::from_scale_rotation_translation(p.scale, rotation, position) * p.node;
                if world != p.world {
                    p.world = world;
                    changed = true;
                }
            }
        }
    }
    if changed {
        let (mut words, dynamic_at) = encode(&bytes.placed, bytes.slots, &bytes.light);
        let tracks = (live.clock.0.to_bits(), live.clock.1);
        (words.0[2][3], words.0[3][2]) = tracks;
        let animated = bytes
            .placed
            .iter()
            .any(|p| matches!(p.behaviour, Some(LightBehaviour::Flicker { .. } | LightBehaviour::Blink { .. })));
        (bytes.words, bytes.dynamic_at, bytes.last_clock, bytes.animated) = (words, dynamic_at, tracks, animated);
        bytes.last_count = usize::MAX;
        if let Some(mut buffer) = buffers.get_mut(&LOCAL_LIGHT_BUFFER) {
            buffer.clear();
            buffer.extend_from_slice(&bytes.words.bytes());
        }
    }
}

static DYNAMIC_OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

fn upload_dynamic_lights(
    dynamic: Res<DynamicLocalLights>,
    clock: Res<LiveLights>,
    bytes: Option<ResMut<LocalLightBytes>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
) {
    let Some(mut bytes) = bytes else { return };
    let tracks = (clock.clock.0.to_bits(), clock.clock.1);
    let off = *DYNAMIC_OFF.get_or_init(|| crate::render::post::env_is("SSX_DYNAMIC_LIGHTS", "off"));
    let none: &[DynamicLight] = &[];
    let requested = if off { none } else { &dynamic.lights[..] };
    if requested.is_empty() && bytes.last_count == 0 && (!bytes.animated || tracks == bytes.last_clock) {
        return;
    }
    (bytes.words.0[2][3], bytes.words.0[3][2]) = tracks;
    bytes.last_clock = tracks;
    let mut lights: Vec<Placed> = requested
        .iter()
        .take(bytes.slots as usize)
        .map(|d| Placed {
            world: d.world,
            light: d.light.clone(),
            factor: Some(d.factor),
            ctrl: None,
            node: Affine3A::IDENTITY,
            scale: Vec3::ONE,
            order: (0, 1, 0, 0, 0),
            behaviour: None,
            entity: 0,
            texture: None,
            texture_layer: 0,
        })
        .collect();
    lights.sort_by_key(bin_of);
    let mut w = Words::default();
    lights.iter().for_each(|p| record(&mut w, p));
    let bounds_at = bytes.words.0[3][3] as usize + LIGHT_WORDS + bytes.words.0[0][3] as usize;
    for (k, p) in lights.iter().enumerate() {
        bytes.words.0[bounds_at + k] = bounds(p).map(f32::to_bits);
    }
    let at = bytes.dynamic_at;
    if lights.len() != bytes.last_count {
        info!(
            "local lights: {} dynamic record(s) of {} reserved ({} requested)",
            lights.len(),
            bytes.slots,
            dynamic.lights.len()
        );
    }
    bytes.words.0[3][1] = lights.len() as u32;
    bytes.words.0[at..at + w.0.len()].copy_from_slice(&w.0);
    bytes.last_count = lights.len();
    if let Some(mut buffer) = buffers.get_mut(&LOCAL_LIGHT_BUFFER) {
        buffer.clear();
        buffer.extend_from_slice(&bytes.words.bytes());
    }
}
