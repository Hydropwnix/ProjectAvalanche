use std::path::Path;

use anyhow::{Context, Result};
use bevy::camera::visibility::DynamicSkinnedMeshBounds;
use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::prelude::*;
use ssx_assets::anim_db::AssetDb;
use ssx_assets::character::model_matrix;
use ssx_assets::ghosts::{GhostAnimation, GhostRig, load_bot_model};

use crate::render::lighting::GameLighting;
use crate::render::lighting::game::GameLightAssets;
use crate::render::objects::foreground::ForegroundMaterial;
use crate::render::objects::material::ObjectMaterial;
use crate::render::objects::{LoadedPart, MaterialHandle, part_from_mesh};
use crate::rider::anim::{JointRecord, world_records};
use crate::rider::material::{CharacterLightUniform, CharacterMaterial};

struct BotPart {
    label: String,
    part: LoadedPart,
    palette: Vec<usize>,
    inverse_binds: Vec<Mat4>,
}

struct LoadedBot {
    name: &'static str,
    rig: GhostRig,
    anim: GhostAnimation,
    lods: Vec<Vec<BotPart>>,
    thresholds: Vec<f32>,
    bounds: (Vec3, Vec3),
    lamps: Vec<Vec<BotLamp>>,
    gear_flags: u16,
    slot: Option<(Vec3, Quat)>,
    axes: Vec<(usize, usize, usize)>,
    racer_type: u32,
}

#[derive(Resource)]
pub struct PendingBots {
    bots: Vec<LoadedBot>,
    board: Option<LoadedPart>,
}

#[derive(Clone, Debug)]
struct BotLamp {
    joint: usize,
    local: JointRecord,
    light: ssx_assets::track_lights::SceneLight,
}

struct BotVisual {
    name: &'static str,
    lamps: Vec<Vec<BotLamp>>,
    gear_flags: u16,
    lamp_on: bool,
    lods: Vec<Entity>,
    thresholds: Vec<f32>,
    bounds: (Vec3, Vec3),
    lod: Option<usize>,
    wipeout_clips: Vec<String>,
    rig: GhostRig,
    anim: GhostAnimation,
    root: Entity,
    nodes: Vec<Entity>,
    board: Option<Entity>,
    slot: Option<(Vec3, Quat)>,
    idle_at: Option<(Vec3, Quat)>,
    axes: Vec<(usize, usize, usize)>,
    trigger_step: Option<u64>,
    materials: Vec<Handle<CharacterMaterial>>,
    racer_type: u32,
    fade: HoloFade,
}

#[derive(Clone, Copy, Debug, Default)]
struct HoloFade {
    frames: u32,
    fade: f32,
}

impl HoloFade {
    fn update(&mut self, dt_s: f32, d2: f32, racers: usize, mut random: impl FnMut(f32) -> f32) -> Vec4 {
        self.frames += 1;
        if self.frames > 4 {
            self.fade = (self.fade + 2.0 / 3.0 * dt_s).clamp(0.0, 1.0);
        }
        let mut fade = self.fade;
        if racers > RACER_POOL && d2 > 9025.0 {
            fade = fade.min(1.0 - ((d2 - 9025.0) / 975.0).clamp(0.0, 1.0));
        }
        // TODO(guess): beyond 4 racers the off-screen fade (0x829E97E8) is not applied and a removed racer is hidden at once (0x82501F98); settle: `GhostManager_FeedPlayback` 0x824B5E50's removal against the record's flags.
        let t = ((d2 - 4.0) / 12.0).clamp(0.0, 1.0);
        let x = 1.0 - (3.0 - 2.0 * t) * t * t;
        let y = (1.0 - fade).max(random(0.7 * x));
        Vec4::new(x, y, 0.0, 0.0)
    }
}

const RACER_POOL: usize = 4;

#[derive(Resource)]
pub struct Bots {
    list: Vec<BotVisual>,
    pub boards: Vec<Option<JointRecord>>,
    pub trajectories: Vec<Option<JointRecord>>,
    // TODO(guess): the game draws from its global generator (seeded from the CPU time base), the port from its own seeded with 1; settle: none (not reproducible).
    rng: ssx_physics::session::bots::Mt19937,
}

#[derive(Resource)]
struct BotCam(usize, Vec3);

fn follow_bot(
    cam: Res<BotCam>,
    bots: Option<Res<Bots>>,
    mode: Option<Res<crate::camera::CameraMode>>,
    mut cameras: Query<&mut Transform, With<crate::camera::FlyCam>>,
) {
    let (Some(bots), Some(mode)) = (bots, mode) else { return };
    let Some(Some(at)) = bots.trajectories.get(cam.0) else { return };
    if *mode != crate::camera::CameraMode::Fly {
        return;
    }
    let target = at.p + Vec3::Y;
    for mut t in &mut cameras {
        *t = Transform::from_translation(target + cam.1).looking_at(target, Vec3::Y);
    }
}

pub type Loaded = PendingBots;

pub fn register(app: &mut App) {
    app.add_systems(crate::shell::level::LevelStartup, spawn.run_if(resource_exists::<PendingBots>))
        .add_systems(
            PostUpdate,
            update
                .before(bevy::transform::TransformSystems::Propagate)
                .before(crate::render::lighting::local::DynamicLightsUpload),
        )
        .add_systems(
            PostUpdate,
            follow_bot
                .after(update)
                .before(bevy::transform::TransformSystems::Propagate)
                .run_if(resource_exists::<BotCam>),
        );
    crate::shell::level::scoped::<PendingBots>(app);
    crate::shell::level::scoped::<BotCam>(app);
    crate::shell::level::scoped::<Bots>(app);
}

pub fn insert(world: &mut World, loaded: Loaded) {
    let lamps: u32 = loaded.bots.iter().map(|b| b.lamps.iter().map(Vec::len).max().unwrap_or(0) as u32).sum();
    if let Some(mut reserve) = world.get_resource_mut::<crate::render::lighting::local::DynamicLightReserve>() {
        reserve.0 += lamps;
    }
    world.insert_resource(loaded);
    let cam = std::env::var("SSX_BOT_CAM").ok().and_then(|v| {
        let p: Vec<f32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
        (p.len() == 4).then(|| BotCam(p[0] as usize, Vec3::new(p[1], p[2], p[3])))
    });
    if let Some(cam) = cam {
        world.insert_resource(cam);
    }
}

pub fn load_level(
    big_root: Option<&Path>,
    track: &str,
    drop_key: Option<u32>,
    ride: &crate::ride::Ride,
) -> Option<Loaded> {
    let root = big_root?;
    let race = ride.session.bots.as_ref()?;
    let requests: Vec<_> = race.bots.iter().map(|b| (b.request, b.replay.header.gear_flags)).collect();
    let loaded = match load(root, track, drop_key, &requests) {
        Ok(l) => l,
        Err(e) => {
            warn!("bots unavailable: {e:#}");
            return None;
        }
    };
    for b in &loaded.bots {
        let parts: Vec<usize> = b.lods.iter().map(Vec::len).collect();
        let nodes = b.lamps.iter().map(Vec::len).max().unwrap_or(0);
        for l in b.lamps.first().into_iter().flatten() {
            info!("bots: {} head lamp node on joint {}: {:?}", b.name, l.joint, l.light);
        }
        info!(
            "bots: {} (parts per LOD {parts:?}, head lamp nodes {nodes}, gear flags {:#x}, slot {:?})",
            b.name,
            b.gear_flags,
            b.slot.map(|s| s.0)
        );
    }
    Some(loaded)
}

fn load(
    root: &Path,
    track: &str,
    drop_key: Option<u32>,
    requests: &[(ssx_physics::session::bots::BotRequest, u16)],
) -> Result<PendingBots> {
    let floats = ssx_assets::rig_anim::float_dofs(root)?;
    let db = AssetDb::load(&root.join(ssx_assets::rig_anim::ANIM_DIR), &[ssx_assets::rig_anim::RIG_BUNDLE])?;
    let slots = drop_key
        .and_then(|k| ssx_physics::spawn::load_drop_starts(root, track).ok()?.into_iter().find(|d| d.drop_key == k))
        .context("the drop's start locator")?;
    let mut bots = Vec::new();
    for (r, gear_flags) in requests {
        let name = ssx_physics::session::bots::character_name(r.character);
        let model = load_bot_model(root, track, name)?;
        for p in &model.problems {
            info!("bot {name} model: {p}");
        }
        let rig = GhostRig::from_rig(root, &model.rig, &model.bind)?;
        let (anim, problems) = GhostAnimation::bot(root, &db, &rig, &floats, name)?;
        for p in problems {
            info!("bot {name} animation: {p}");
        }
        let bounds = model.lods[0]
            .iter()
            .flat_map(|p| p.mesh.positions.iter())
            .fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), p| {
                (lo.min(Vec3::from_array(*p)), hi.max(Vec3::from_array(*p)))
            });
        let mut lods = Vec::new();
        for parts in model.lods {
            let mut loaded = Vec::new();
            for p in parts {
                let label = p.mesh_name.clone();
                match part_from_mesh(root, p.mesh, &label, &p.material)
                    .and_then(|part| part.with_skin(p.skin.joints, p.skin.weights))
                {
                    Ok(part) => loaded.push(BotPart {
                        label,
                        part,
                        palette: p.skin.palette.iter().map(|j| j.node).collect(),
                        inverse_binds: p
                            .skin
                            .palette
                            .iter()
                            .map(|j| Mat4::from_cols_array_2d(&j.inverse_bind))
                            .collect(),
                    }),
                    Err(e) => warn!("bot {name} part {label}: {e:#}"),
                }
            }
            lods.push(loaded);
        }
        let (thresholds, game_object) = (model.lod_thresholds, model.game_object);
        let lamps: Vec<Vec<BotLamp>> = model
            .light_nodes
            .iter()
            .map(|nodes| {
                nodes
                    .iter()
                    .filter(|n| n.group_name == "character_headlamp_group")
                    .filter_map(|n| {
                        let joint = rig.rig.find(&n.joint_name)?;
                        let [x, y, z, w] = n.orientation;
                        Some(BotLamp {
                            joint,
                            local: JointRecord {
                                s: Vec3::new(n.scale[0], n.scale[1], n.scale[2]),
                                q: Quat::from_xyzw(x, y, z, w),
                                p: Vec3::new(n.position[0], n.position[1], n.position[2]),
                            },
                            light: n.light.clone()?,
                        })
                    })
                    .collect()
            })
            .collect();
        info!(
            "bot {name}: game object {game_object:08x}, LODs {:?} at {thresholds:?}",
            lods.iter().map(|l| l.iter().map(|p| p.label.as_str()).collect::<Vec<_>>()).collect::<Vec<_>>()
        );
        let axes = [("LeftAxeAux", "LeftIceAxe"), ("RightAxeAux", "RightIceAxe")]
            .iter()
            .filter_map(|&(s, t)| {
                let (s, t) = (rig.rig.find(s)?, rig.rig.find(t)?);
                Some((s, t, rig.rig.nodes[t].parent?))
            })
            .collect();
        let slot = slots.slot(usize::try_from(r.start_slot).unwrap_or(0));
        bots.push(LoadedBot {
            name,
            rig,
            anim,
            lods,
            thresholds,
            bounds,
            lamps,
            gear_flags: *gear_flags,
            slot,
            axes,
            racer_type: r.racer_type,
        });
    }
    let board = crate::rider::visual::load_board(root, track).ok().map(crate::rider::visual::LoadedBoard::into_part);
    Ok(PendingBots { bots, board })
}

fn record_transform(r: &JointRecord) -> Transform {
    Transform { translation: r.p, rotation: r.q, scale: r.s }
}

#[allow(clippy::too_many_arguments)]
fn spawn(
    mut commands: Commands,
    mut pending: ResMut<PendingBots>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut objects: ResMut<Assets<ObjectMaterial>>,
    mut foreground: ResMut<Assets<ForegroundMaterial>>,
    mut characters: ResMut<Assets<CharacterMaterial>>,
    mut bindposes: ResMut<Assets<SkinnedMeshInverseBindposes>>,
    light: Res<GameLightAssets>,
    lighting: Option<Res<GameLighting>>,
) {
    let rider_light =
        lighting.as_deref().map_or_else(CharacterLightUniform::viewer_default, CharacterLightUniform::from_preset);
    // TODO(guess): `dynamicAOMult` stays 1 (the racer's `AoGroundContactTrack` needs interface 0x57EF8508, creator 0x82928AB0); settle: the `ghost_<character>` animatable's QueryInterface (0x82AC0440 / 0x82ACA240).
    let mut list = Vec::new();
    let board_part = pending.board.take();
    let board_mesh =
        board_part.map(|p| p.add_character(&mut meshes, &mut images, &light, &rider_light, &mut characters));
    for b in std::mem::take(&mut pending.bots) {
        let root =
            commands.spawn((Name::new(format!("bot {}", b.name)), Transform::default(), Visibility::Hidden)).id();
        let rig_root = commands.spawn((Name::new("bot rig"), Transform::default(), ChildOf(root))).id();
        let bind = world_records(&b.rig.rig, &b.rig.bind, Vec3::ZERO);
        let nodes: Vec<Entity> = b
            .rig
            .rig
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let r = bind.get(i).copied().unwrap_or(JointRecord { s: Vec3::ONE, q: Quat::IDENTITY, p: Vec3::ZERO });
                commands.spawn((Name::new(n.name.clone()), record_transform(&r), ChildOf(rig_root))).id()
            })
            .collect();
        let mut lods = Vec::new();
        let mut materials = Vec::new();
        for (k, parts) in b.lods.into_iter().enumerate() {
            let lod = commands
                .spawn((
                    Name::new(format!("bot {} LOD {k}", b.name)),
                    Transform::default(),
                    Visibility::Hidden,
                    ChildOf(root),
                ))
                .id();
            lods.push(lod);
            for p in parts {
                let inverse_bindposes = bindposes.add(SkinnedMeshInverseBindposes::from(p.inverse_binds));
                let joints = p.palette.iter().map(|&n| nodes[n]).collect();
                let bundle = (
                    Name::new(format!("bot {} {}", b.name, p.label)),
                    SkinnedMesh { inverse_bindposes, joints },
                    DynamicSkinnedMeshBounds,
                    Transform::default(),
                    ChildOf(lod),
                );
                if p.part.is_character() {
                    let (mesh, material) =
                        p.part.add_character(&mut meshes, &mut images, &light, &rider_light, &mut characters);
                    materials.push(material.clone());
                    commands.spawn((bundle, Mesh3d(mesh), MeshMaterial3d(material)));
                    continue;
                }
                let (mesh, material) = p.part.add(&mut meshes, &mut images, &light, &mut objects, &mut foreground);
                let mut e = commands.spawn((bundle, Mesh3d(mesh)));
                match material {
                    MaterialHandle::Object(m) => e.insert(MeshMaterial3d(m)),
                    MaterialHandle::Foreground(m) => e.insert(MeshMaterial3d(m)),
                };
            }
        }
        let board = board_mesh.as_ref().map(|(mesh, material)| {
            let material = characters.get(material).cloned().map_or_else(|| material.clone(), |m| characters.add(m));
            materials.push(material.clone());
            commands
                .spawn((
                    Name::new(format!("bot {} snowboard", b.name)),
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material),
                    Transform::default(),
                    ChildOf(root),
                ))
                .id()
        });
        list.push(BotVisual {
            name: b.name,
            lods,
            thresholds: b.thresholds,
            bounds: b.bounds,
            lod: None,
            lamps: b.lamps,
            gear_flags: b.gear_flags,
            lamp_on: false,
            wipeout_clips: Vec::new(),
            rig: b.rig,
            anim: b.anim,
            root,
            nodes,
            board,
            slot: b.slot,
            idle_at: None,
            axes: b.axes,
            trigger_step: None,
            materials,
            racer_type: b.racer_type,
            fade: HoloFade::default(),
        });
    }
    let n = list.len();
    let rng = ssx_physics::session::bots::Mt19937::seeded(1);
    commands.insert_resource(Bots { list, boards: vec![None; n], trajectories: vec![None; n], rng });
}

fn board_transform(rig: &GhostRig, board: usize, w: &JointRecord) -> Transform {
    let m = Mat4::from_scale_rotation_translation(w.s, w.q, w.p);
    let b0 = Mat4::from_cols_array_2d(&model_matrix(&rig.rig, &rig.bind, board));
    Transform::from_matrix(m * b0.inverse())
}

#[allow(clippy::too_many_arguments)]
fn update(
    time: Res<Time>,
    ride: Option<Res<crate::ride::Ride>>,
    bots: Option<ResMut<Bots>>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    lighting: Option<Res<GameLighting>>,
    mut lights: ResMut<crate::render::lighting::local::DynamicLocalLights>,
    mut transforms: Query<&mut Transform>,
    mut visibility: Query<&mut Visibility>,
    mut characters: ResMut<Assets<CharacterMaterial>>,
) {
    let (Some(ride), Some(mut bots)) = (ride, bots) else { return };
    let Some(race) = ride.session.bots.as_ref() else { return };
    let view = cameras.iter().find(|(c, _)| c.is_active).map(|(c, t)| {
        let clip = c.clip_from_view();
        (t.translation(), 1.0 / clip.x_axis.x.abs().max(clip.y_axis.y.abs()))
    });
    let night = lighting.as_ref().is_some_and(|l| l.f("isNightCondition") > 0.0);
    let dt_ticks = if ride.hold { 0.0 } else { (time.delta_secs() * 60.0).min(4.0) };
    let lag = (ride.steps as f32 / ssx_physics::SIM_HZ - ride.render_time).max(0.0);
    let Bots { list, boards, trajectories, rng } = &mut *bots;
    for (k, v) in list.iter_mut().enumerate() {
        let Some(b) = race.bots.get(k) else { continue };
        if matches!(v.racer_type, 0 | 2) && !b.removed {
            let d2 = b.position.distance_squared(ride.state.traj);
            let holo = v.fade.update(dt_ticks / 60.0, d2, race.bots.len(), |max| {
                let u = (rng.next_u32() as i32) as f64 * 2f64.powi(-32) + 0.5;
                (u * f64::from(max)) as f32
            });
            for m in &v.materials {
                if let Some(mut m) = characters.get_mut(m)
                    && m.extension.params.holo != holo
                {
                    m.extension.params.holo = holo;
                }
            }
        }
        if v.idle_at.is_none()
            && let Some((p, q)) = v.slot
        {
            v.idle_at = ssx_physics::levelstart::offset_level_start(ride.world(), p).map(|o| (o, q));
        }
        let records = if !race.running {
            let Some((at, q)) = v.idle_at else { continue };
            let Some((pose, p)) = v.anim.idle_pose(&v.rig, dt_ticks, at.to_array(), q.to_array()) else { continue };
            world_records(&v.rig.rig, &pose.nodes, Vec3::from_array(p))
        } else if b.removed {
            if let Ok(mut vis) = visibility.get_mut(v.root) {
                *vis = Visibility::Hidden;
            }
            boards[k] = None;
            trajectories[k] = None;
            continue;
        } else {
            let t = (b.clock.seconds - lag * b.factor).max(0.0);
            let pose = b.replay.pose(t);
            let wipeout = b.trigger_wipeout && v.trigger_step != Some(ride.steps);
            if wipeout {
                v.trigger_step = Some(ride.steps);
            }
            v.anim.set_signal("BLN_TriggerWipeout", if wipeout { 1.0 } else { 0.0 });
            v.anim.set_signal("BLN_PlayerIsGrinding", if b.player_is_grinding { 1.0 } else { 0.0 });
            v.anim.set_signal("BLN_InAir", if pose.flags & ssx_formats::ghost::FLAG_IN_AIR != 0 { 1.0 } else { 0.0 });
            if let Some(w) = &b.wipeout {
                v.anim.set_wipeout_weight(w.layer_weight);
                v.anim.set_signal("BLN_TriggerGetup", if w.trigger_getup { 1.0 } else { 0.0 });
            }
            let p = v.anim.pose(&v.rig, &pose, dt_ticks);
            let wipe: Vec<String> = v
                .anim
                .playing()
                .into_iter()
                .filter(|(n, w)| n.starts_with("bail_") && *w > 0.0)
                .map(|(n, w)| format!("{n} {w:.2}"))
                .collect();
            if wipe != v.wipeout_clips {
                info!("bot {k} ({}): wipeout layer {wipe:?} (contacts {})", v.name, b.contacts);
                v.wipeout_clips = wipe;
            }
            let at = Vec3::from_array(pose.trajectory_position);
            let mut records = world_records(&v.rig.rig, &p.nodes, at);
            let axe = if b.replay.header.gear_flags & 2 != 0 { 1.0 } else { 0.0 };
            for &op in &v.axes {
                crate::rider::skin::prop_in_hands(&v.rig.rig, &mut records, op, axe);
            }
            records
        };
        if let Ok(mut vis) = visibility.get_mut(v.root) {
            *vis = Visibility::Inherited;
        }
        // TODO(guess): the bounds are the LOD-0 bind-pose box placed by the drawn `AITrajectory`; settle: the skinned racer's render entity's visibility registration (cf. 0x829C1438).
        let traj = v.rig.rig.find("AITrajectory").and_then(|i| records.get(i)).copied();
        trajectories[k] = traj;
        let metric = view.zip(traj).map(|((eye, tan_min), t)| {
            let (lo, hi) = v.bounds;
            let (c, h) = ((lo + hi) * 0.5, (hi - lo) * 0.5);
            let m = Mat3::from_quat(t.q);
            let h_world = Vec3::new(m.row(0).abs().dot(h), m.row(1).abs().dot(h), m.row(2).abs().dot(h));
            let centre = t.p + t.q * c;
            let d = centre.distance(eye);
            if d > 0.0 { h_world.length() / (d * tan_min) } else { f32::INFINITY }
        });
        let lod = match metric {
            Some(m) => (m > 0.002).then(|| v.thresholds.iter().position(|&th| th <= m.max(0.0))).flatten(),
            None => Some(0),
        };
        let lod = lod.filter(|&k| k < v.lods.len());
        if lod != v.lod {
            info!("bot {k} ({}): LOD {lod:?} (size metric {metric:.3?}) at {:.1?}", v.name, traj.map(|t| t.p));
            v.lod = lod;
        }
        for (i, &e) in v.lods.iter().enumerate() {
            if let Ok(mut vis) = visibility.get_mut(e) {
                *vis = if Some(i) == lod { Visibility::Inherited } else { Visibility::Hidden };
            }
        }
        for (&entity, r) in v.nodes.iter().zip(&records) {
            if let Ok(mut tr) = transforms.get_mut(entity) {
                *tr = record_transform(r);
            }
        }
        if let (Some(e), Some(board)) = (v.board, v.rig.board)
            && let Some(w) = records.get(board)
            && let Ok(mut tr) = transforms.get_mut(e)
        {
            *tr = board_transform(&v.rig, board, w);
        }
        boards[k] = v.rig.board.and_then(|i| records.get(i)).copied();
        let lamp_on = night && v.gear_flags & 1 != 0 && race.running;
        if lamp_on != v.lamp_on {
            info!("bot {k} ({}): head lamp {}", v.name, if lamp_on { "on (BotHeadlampOn)" } else { "off" });
            v.lamp_on = lamp_on;
        }
        if lamp_on && !crate::render::post::env_is("SSX_BOT_LAMP", "off") {
            for lamp in lod.and_then(|i| v.lamps.get(i)).into_iter().flatten() {
                let Some(joint) = records.get(lamp.joint).filter(|_| lamp.light.enable) else { continue };
                let r = crate::rider::skin::compose(joint, &lamp.local);
                if ride.steps % 30 == 0 {
                    debug!("bot {k} ({}): lamp at {:.1} towards {:.2}", v.name, r.p, r.q * Vec3::NEG_Z);
                }
                lights.lights.push(crate::render::lighting::local::DynamicLight {
                    world: bevy::math::Affine3A::from_scale_rotation_translation(r.s, r.q, r.p),
                    light: lamp.light.clone(),
                    factor: 1.0,
                });
            }
        }
    }
}

pub fn hud_line(ride: &crate::ride::Ride) -> String {
    let Some(race) = ride.session.bots.as_ref() else { return String::new() };
    let ev = &ride.session.event;
    let split = ev.split_times.last().copied();
    let course = ev.setup.course;
    let clock = |ms: u32| format!("{}:{:06.3}", ms / 60_000, (ms % 60_000) as f32 / 1000.0);
    let items: Vec<String> = race
        .bots
        .iter()
        .enumerate()
        .map(|(k, b)| {
            let name = ssx_physics::session::bots::character_name(b.request.character).to_uppercase();
            let r = ev.racers.get(k);
            let state = match (r.and_then(|r| r.finish_ms), b.removed) {
                (Some(ms), _) => format!("FINISHED {}", clock(ms)),
                (None, true) => "gone".to_string(),
                (None, false) => format!("{:.0}%", 100.0 * r.map_or(0.0, |r| r.progress)),
            };
            let gap = split
                .zip(course)
                .and_then(|((k, ms), c)| {
                    let h = c.splits()[usize::try_from(k).ok()?];
                    b.replay.time_below(h).map(|tg| ms as f32 / 1000.0 - tg)
                })
                .map_or(String::new(), |d| format!(" {d:+.2} s"));
            format!(
                "{name} {state} | clock {:.1} s x{:.2} lead {:+.0} m{gap}{}",
                b.clock.seconds,
                b.factor,
                b.lead,
                if b.contacts > 0 { format!(" | {} contacts", b.contacts) } else { String::new() }
            )
        })
        .collect();
    let when = if race.running { "racing" } else { "waiting for the start" };
    let split = split.map_or(String::new(), |(k, _)| format!(", gaps at split #{}, + = behind", k + 1));
    format!("\nBOTS ({when}{split}):\n  {}\n", items.join("\n  "))
}
