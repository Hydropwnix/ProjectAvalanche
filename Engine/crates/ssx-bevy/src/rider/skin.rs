use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use bevy::camera::visibility::DynamicSkinnedMeshBounds;
use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::prelude::{
    AlphaMode, Assets, ChildOf, Commands, Entity, Handle, Image, Mat4, Mesh, Mesh3d, MeshMaterial3d, Name, Quat, Query,
    Resource, Transform, Vec3,
};
use ssx_assets::character::{Character, Loadout, PartMaterial, Srt, model_matrix};
use ssx_assets::content::characters::dress::{DressRequest, dress};
use ssx_assets::track_lights::SceneLight;
use ssx_formats::crsf::SceneGraph;

use crate::render::lighting::game::GameLightAssets;
use crate::render::objects::foreground::ForegroundMaterial;
use crate::render::objects::material::ObjectMaterial;
use crate::render::objects::{LoadedPart, MaterialHandle, part_from_mesh};
use crate::rider::anim::{JointRecord, RiderAnimation, world_records};
use crate::rider::material::{CharacterLightUniform, CharacterMaterial};

pub(super) struct SkinnedPart {
    slot: String,
    wingsuit: bool,
    part: LoadedPart,
    palette: Vec<usize>,
    inverse_binds: Vec<Mat4>,
}

pub struct LoadedRider {
    pub summary: String,
    rig: SceneGraph,
    bind: Vec<Srt>,
    board: usize,
    parts: Vec<SkinnedPart>,
    anim: RiderAnimation,
    gear_lights: Vec<GearLight>,
    source: (PathBuf, Option<String>, DressRequest),
    loadout: Loadout,
    pub riders: Option<ssx_assets::content::characters::budget::RiderLooks>,
    pub problems: Vec<String>,
    pub load_secs: f32,
}

#[derive(Clone, Debug)]
pub struct GearLight {
    pub slot: String,
    pub group: String,
    joint: usize,
    local: JointRecord,
    pub light: SceneLight,
}

pub fn load_rider(big_root: &Path, track: Option<&str>, req: &DressRequest) -> Result<LoadedRider> {
    let start = Instant::now();
    let name = req.stand_in.as_str();
    let character = dress(big_root, track, req, &Loadout::default())?;
    if character.parts.is_empty() {
        anyhow::bail!("{}: no part loaded ({})", req.id, character.problems.join("; "));
    }
    let summary = character.summary();
    let mut problems = character.problems.clone();
    let loaded = RiderAnimation::load(big_root, &character, name)?;
    problems.extend(loaded.problems);
    let (anim, rig, bind) = (loaded.anim, loaded.rig, loaded.bind);
    let board = rig.find("Board").context("the rig has no Board joint")?;
    let (parts, gear_lights) = load_parts(big_root, character, &rig, &mut problems);
    let source = (big_root.to_path_buf(), track.map(str::to_string), req.clone());
    let load_secs = start.elapsed().as_secs_f32();
    let loadout = Loadout::default();
    Ok(LoadedRider {
        summary,
        rig,
        bind,
        board,
        parts,
        anim,
        gear_lights,
        source,
        loadout,
        riders: None,
        problems,
        load_secs,
    })
}

pub(super) fn load_parts(
    big_root: &Path,
    character: Character,
    rig: &SceneGraph,
    problems: &mut Vec<String>,
) -> (Vec<SkinnedPart>, Vec<GearLight>) {
    let mut parts = Vec::new();
    for p in character.parts {
        let label = p.mesh_name.clone();
        let wingsuit = p.material.effect() == Some("player_wingsuit.fx");
        let material = match &p.material {
            PartMaterial::Disc(m) => ssx_assets::content::GameContent::of(big_root)
                .material_for(m, ssx_assets::content::resolve::MaterialUse::Riders),
            PartMaterial::Def(d) => (**d).clone(),
        };
        let palette: Option<Vec<usize>> = p.skin.palette.iter().map(|j| rig.find(&j.name)).collect();
        let Some(palette) = palette else {
            problems.push(format!("{label}: a palette joint the rig lacks"));
            continue;
        };
        let loaded = part_from_mesh(big_root, p.mesh, &label, &material)
            .and_then(|part| part.with_skin(p.skin.joints, p.skin.weights));
        match loaded {
            Ok(part) => {
                problems.extend(part.problems.iter().map(|e| format!("{label}: {e}")));
                parts.push(SkinnedPart {
                    slot: p.slot,
                    wingsuit,
                    part,
                    palette,
                    inverse_binds: p.skin.palette.iter().map(|j| Mat4::from_cols_array_2d(&j.inverse_bind)).collect(),
                });
            }
            Err(e) => problems.push(format!("{label}: {e:#}")),
        }
    }
    let mut lights = Vec::new();
    for g in character.light_nodes {
        let n = &g.node;
        let (Some(light), Some(joint)) = (n.light.clone(), rig.find(&n.joint_name)) else {
            problems.push(format!("{} light node on {:?}: no light or no such joint", g.slot, n.joint_name));
            continue;
        };
        let [x, y, z, w] = n.orientation;
        let local = JointRecord {
            s: Vec3::new(n.scale[0], n.scale[1], n.scale[2]),
            q: Quat::from_xyzw(x, y, z, w),
            p: Vec3::new(n.position[0], n.position[1], n.position[2]),
        };
        lights.push(GearLight { slot: g.slot, group: n.group_name.clone(), joint, local, light });
    }
    (parts, lights)
}

impl LoadedRider {
    pub fn has_board_part(&self) -> bool {
        self.parts.iter().any(|p| p.slot == "snowboard")
    }

    pub fn dress(&mut self, loadout: &Loadout) -> Result<()> {
        let start = Instant::now();
        let (big_root, track, req) = self.source.clone();
        let character = dress(&big_root, track.as_deref(), &req, loadout)?;
        if character.parts.is_empty() {
            anyhow::bail!("{}: no part loaded ({})", req.id, character.problems.join("; "));
        }
        self.summary = character.summary();
        let mut problems = character.problems.clone();
        let (parts, lights) = load_parts(&big_root, character, &self.rig, &mut problems);
        (self.parts, self.gear_lights) = (parts, lights);
        self.loadout = loadout.clone();
        self.problems.extend(problems);
        self.load_secs += start.elapsed().as_secs_f32();
        Ok(())
    }
}

#[derive(Resource)]
pub struct RiderRig {
    nodes: Vec<Entity>,
    rig: SceneGraph,
    bind: Vec<Srt>,
    board: usize,
    board_entity: Option<(Entity, Vec3)>,
    pub anim: RiderAnimation,
    pub drawn_trajectory: (Vec3, Quat),
    pub drawn_board: Vec3,
    pub summary: String,
    pub anim_ms: f32,
    pub character_materials: Vec<Handle<CharacterMaterial>>,
    /// Each character material's part slot (`jackets`, `gloves`…), in the same order.
    pub character_slots: Vec<String>,
    pub wingsuit_materials: Vec<Handle<CharacterMaterial>>,
    pub gear_lights: Vec<GearLight>,
    pub drawn_gear_lights: Vec<JointRecord>,
    props: Vec<PropInHands>,
    pub(super) root: Entity,
    pub(super) part_entities: Vec<Entity>,
    pub(super) redress: Redress,
    /// The menus' showcase: the board's heading when the idle started (the idle is held to it).
    idle_heading: Option<f32>,
    /// The menus' showcase: where the board stands and the hand grips it (fixed when the idle starts).
    idle_grip: Option<(Vec3, Vec3)>,
    /// The menus' showcase: the head's and the hands' size (a suit's BigHead / BigHands perk; 1: as they are).
    idle_big: [f32; 2],
}

#[derive(Clone)]
pub(super) struct Redress {
    pub big_root: PathBuf,
    pub track: Option<String>,
    pub request: DressRequest,
    pub loadout: Loadout,
    pub riders: Option<ssx_assets::content::characters::budget::RiderLooks>,
}

impl RiderRig {
    pub(super) fn rig(&self) -> &SceneGraph {
        &self.rig
    }

    pub(super) fn nodes(&self) -> &[Entity] {
        &self.nodes
    }
}

pub(super) struct SpawnedParts {
    pub entities: Vec<Entity>,
    pub character_materials: Vec<Handle<CharacterMaterial>>,
    pub character_slots: Vec<String>,
    pub wingsuit_materials: Vec<Handle<CharacterMaterial>>,
}

pub(super) struct PartAssets<'a> {
    pub meshes: &'a mut Assets<Mesh>,
    pub images: &'a mut Assets<Image>,
    pub light: &'a GameLightAssets,
    pub rider_light: &'a CharacterLightUniform,
    pub objects: &'a mut Assets<ObjectMaterial>,
    pub foreground: &'a mut Assets<ForegroundMaterial>,
    pub characters: &'a mut Assets<CharacterMaterial>,
    pub bindposes: &'a mut Assets<SkinnedMeshInverseBindposes>,
}

pub(super) fn spawn_parts(
    commands: &mut Commands,
    root: Entity,
    nodes: &[Entity],
    parts: Vec<SkinnedPart>,
    a: &mut PartAssets,
) -> SpawnedParts {
    let mut out =
        SpawnedParts {
            entities: Vec::new(),
            character_materials: Vec::new(),
            character_slots: Vec::new(),
            wingsuit_materials: Vec::new(),
        };
    for p in parts {
        let inverse_bindposes = a.bindposes.add(SkinnedMeshInverseBindposes::from(p.inverse_binds));
        let joints = p.palette.iter().filter_map(|&n| nodes.get(n).copied()).collect();
        let bundle = (
            Name::new(format!("rider {}", p.slot)),
            SkinnedMesh { inverse_bindposes, joints },
            DynamicSkinnedMeshBounds,
            Transform::default(),
            ChildOf(root),
        );
        if p.part.is_character() {
            let (mesh, material) = p.part.add_character(a.meshes, a.images, a.light, a.rider_light, a.characters);
            if p.wingsuit {
                if let Some(mut m) = a.characters.get_mut(&material) {
                    m.base.alpha_mode = AlphaMode::Mask(0.25);
                }
                out.wingsuit_materials.push(material.clone());
            }
            out.character_materials.push(material.clone());
            out.character_slots.push(p.slot.clone());
            out.entities.push(commands.spawn((bundle, Mesh3d(mesh), MeshMaterial3d(material))).id());
            continue;
        }
        let (mesh, material) = p.part.add(a.meshes, a.images, a.light, a.objects, a.foreground);
        let mut entity = commands.spawn((bundle, Mesh3d(mesh)));
        match material {
            MaterialHandle::Object(m) => entity.insert(MeshMaterial3d(m)),
            MaterialHandle::Foreground(m) => entity.insert(MeshMaterial3d(m)),
        };
        out.entities.push(entity.id());
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PropWeight {
    IceAxe,
    WingsuitHandles,
}

const PROPS_IN_HANDS: [(&str, &str, PropWeight); 4] = [
    ("LeftAxeAux", "LeftIceAxe", PropWeight::IceAxe),
    ("RightAxeAux", "RightIceAxe", PropWeight::IceAxe),
    ("LeftHandleAux", "LeftHandle", PropWeight::WingsuitHandles),
    ("RightHandleAux", "RightHandle", PropWeight::WingsuitHandles),
];

#[derive(Clone, Copy, Debug)]
struct PropInHands {
    source: usize,
    target: usize,
    parent: usize,
    weight: PropWeight,
}

pub(crate) fn compose(parent: &JointRecord, local: &JointRecord) -> JointRecord {
    JointRecord {
        s: parent.s * local.s,
        q: (parent.q * local.q).normalize(),
        p: parent.p + parent.q * (parent.s * local.p),
    }
}

fn relative(parent: &JointRecord, x: &JointRecord) -> JointRecord {
    let inv = parent.q.conjugate();
    JointRecord { s: x.s / parent.s, q: (inv * x.q).normalize(), p: (inv * (x.p - parent.p)) / parent.s }
}

// TODO(guess): applied to the frame's interpolated records with the newest step's weights, the game runs it per step in `RigOpList_Rigamate` before the record; settle: move it into the per-step pose of `rider::anim` (the deploy ramps' 10–15 ticks).
fn props_in_hands(rig: &RiderRig, records: &mut [JointRecord], state: &ssx_physics::rider::RiderState) {
    for op in &rig.props {
        let w = match op.weight {
            PropWeight::IceAxe => state.ice_axe.blend_weight,
            PropWeight::WingsuitHandles => state.wingsuit.handle_weight,
        };
        prop_in_hands(&rig.rig, records, (op.source, op.target, op.parent), w);
    }
}

pub fn prop_in_hands(
    rig: &SceneGraph,
    records: &mut [JointRecord],
    (source, target, parent): (usize, usize, usize),
    w: f32,
) {
    if w == 0.0 || target >= records.len() || source >= records.len() {
        return;
    }
    let parent = records[parent];
    let local = relative(&parent, &records[target]);
    let src = relative(&parent, &records[source]);
    let q = if local.q.dot(src.q) > 0.0 { local.q + (src.q - local.q) * w } else { local.q - (src.q + local.q) * w };
    let moved = JointRecord { s: local.s, q: q.normalize(), p: local.p + (src.p - local.p) * w };
    let old = records[target];
    records[target] = compose(&parent, &moved);
    let mut changed = vec![(target, old)];
    for (i, node) in rig.nodes.iter().enumerate().skip(target + 1) {
        let Some(p) = node.parent else { continue };
        if let Some(&(_, before)) = changed.iter().find(|(c, _)| *c == p) {
            let child = records[i];
            records[i] = compose(&records[p], &relative(&before, &child));
            changed.push((i, child));
        }
    }
}

fn record_transform(r: &JointRecord) -> Transform {
    Transform { translation: r.p, rotation: r.q, scale: r.s }
}

fn board_transform(rig: &RiderRig, board: &JointRecord, extra: Vec3) -> Transform {
    let w = Mat4::from_scale_rotation_translation(board.s, board.q, board.p);
    let b0 = Mat4::from_cols_array_2d(&model_matrix(&rig.rig, &rig.bind, rig.board));
    Transform::from_matrix(w * b0.inverse() * Mat4::from_translation(extra))
}

#[allow(clippy::too_many_arguments)]
pub fn spawn(
    commands: &mut Commands,
    root: Entity,
    rider: LoadedRider,
    board: Option<(Entity, bool)>,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    light: &GameLightAssets,
    rider_light: &CharacterLightUniform,
    objects: &mut Assets<ObjectMaterial>,
    foreground: &mut Assets<ForegroundMaterial>,
    characters: &mut Assets<CharacterMaterial>,
    bindposes: &mut Assets<SkinnedMeshInverseBindposes>,
) -> RiderRig {
    let rig_root = commands.spawn((Name::new("rig"), Transform::default(), ChildOf(root))).id();
    let bind_records = world_records(&rider.rig, &rider.bind, Vec3::ZERO);
    let nodes: Vec<Entity> = (rider.rig.nodes.iter().zip(&bind_records))
        .map(|(node, r)| commands.spawn((Name::new(node.name.clone()), record_transform(r), ChildOf(rig_root))).id())
        .collect();
    let mut assets = PartAssets { meshes, images, light, rider_light, objects, foreground, characters, bindposes };
    let spawned = spawn_parts(commands, root, &nodes, rider.parts, &mut assets);
    let SpawnedParts { entities: part_entities, character_materials, character_slots, wingsuit_materials } = spawned;
    let props = PROPS_IN_HANDS
        .iter()
        .filter_map(|&(source, target, weight)| {
            let (source, target) = (rider.rig.find(source)?, rider.rig.find(target)?);
            Some(PropInHands { source, target, parent: rider.rig.nodes[target].parent?, weight })
        })
        .collect();
    let board_bind = model_matrix(&rider.rig, &rider.bind, rider.board)[3];
    let box_extra = Vec3::new(board_bind[0], board_bind[1], board_bind[2]);
    let board_entity = board.map(|(e, mesh)| (e, if mesh { Vec3::ZERO } else { box_extra }));
    RiderRig {
        nodes,
        rig: rider.rig,
        bind: rider.bind,
        board: rider.board,
        board_entity,
        anim: rider.anim,
        drawn_trajectory: (Vec3::ZERO, Quat::IDENTITY),
        drawn_board: Vec3::ZERO,
        summary: rider.summary,
        anim_ms: 0.0,
        character_materials,
        character_slots,
        wingsuit_materials,
        gear_lights: rider.gear_lights,
        drawn_gear_lights: Vec::new(),
        props,
        root,
        part_entities,
        redress: Redress {
            big_root: rider.source.0,
            track: rider.source.1,
            request: rider.source.2,
            loadout: rider.loadout,
            riders: rider.riders,
        },
        idle_heading: None,
        idle_grip: None,
        idle_big: [1.0, 1.0],
    }
}

/// A port addition (the menus' 3-D rider showcase): poses the rig with the level start's idle stepped by `dt`
/// (the rider standing at the top of a drop). False when the rider has no such animation.
/// The menus' showcase: the board shown off. It stands on its tail at the rider's right (the viewer's left),
/// upright, its base (the design) to the camera, beside the right hand where the idle starts, and stays there.
fn show_board(rig: &mut RiderRig, records: &mut [JointRecord]) {
    const HALF_LENGTH: f32 = 0.78;
    const HALF_WIDTH: f32 = 0.14;
    let find = |n: &str| rig.rig.find(n);
    let board_at = match rig.idle_grip {
        Some((at, _)) => at,
        None => {
            let Some(hand) = find("RightHand").and_then(|i| records.get(i)).map(|r| r.p) else { return };
            let floor = ["LeftToeBase", "RightToeBase", "LeftFoot", "RightFoot"]
                .iter()
                .filter_map(|n| find(n).and_then(|i| records.get(i)).map(|r| r.p.y))
                .reduce(f32::min)
                .map_or(0.0, |y| y - 0.04);
            // Beside the hand, a little further out (the viewer's left) and back, clear of the arm and leg.
            let at = Vec3::new(hand.x - HALF_WIDTH - 0.1, floor + HALF_LENGTH, hand.z - 0.15);
            rig.idle_grip = Some((at, hand));
            at
        }
    };
    // The board's rest (bind) pose lies flat under the feet, base down, its length along the rig's x: turned a
    // quarter about x (base to the camera, +z) and a quarter about z (length up). Its mesh hangs on the Board joint
    // and the joints below it (its nose, tail and bindings): all of them move with it, as one piece.
    let (_, bind_q, _) =
        Mat4::from_cols_array_2d(&model_matrix(&rig.rig, &rig.bind, rig.board)).to_scale_rotation_translation();
    let upright = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2) * Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
    let Some(board) = records.get(rig.board) else { return };
    let (p0, q0) = (board.p, board.q);
    let delta = (upright * bind_q).normalize() * q0.inverse();
    let below = |mut i: usize| loop {
        if i == rig.board {
            return true;
        }
        match rig.rig.nodes.get(i).and_then(|n| n.parent) {
            Some(p) => i = p,
            None => return false,
        }
    };
    for (i, r) in records.iter_mut().enumerate() {
        if below(i) {
            r.p = board_at + delta * (r.p - p0);
            r.q = (delta * r.q).normalize();
        }
    }
}

/// The menus' showcase: a suit's big-head / big-hands perk shown (2: twice the size).
pub fn set_big(rig: &mut RiderRig, head: f32, hands: f32) {
    rig.idle_big = [head, hands];
}

/// The head, and the hands, scaled about their joints, with all that hangs below them (the face, the fingers, what
/// the hands hold).
fn big_parts(rig: &RiderRig, records: &mut [JointRecord]) {
    let [head, hands] = rig.idle_big;
    let below = |mut i: usize, top: usize| loop {
        if i == top {
            return true;
        }
        match rig.rig.nodes.get(i).and_then(|n| n.parent) {
            Some(p) => i = p,
            None => return false,
        }
    };
    for (name, scale) in [("Head", head), ("LeftHand", hands), ("RightHand", hands)] {
        if (scale - 1.0).abs() < 1e-3 {
            continue;
        }
        let Some(top) = rig.rig.find(name) else { continue };
        let Some(at) = records.get(top).map(|r| r.p) else { continue };
        for (i, r) in records.iter_mut().enumerate() {
            if below(i, top) {
                r.p = at + (r.p - at) * scale;
                r.s *= scale;
            }
        }
    }
}

pub fn animate_idle(rig: &mut RiderRig, dt: f32, transforms: &mut Query<&mut Transform>) -> bool {
    let Some(mut records) = rig.anim.idle_records(dt) else { return false };
    // The idle comes from the helicopter scenes: the rider sways and swings the board in a hand. Held every frame by
    // the body, not the board: the hips' place over the ground stays fixed, and the body's facing (across its hips and
    // shoulders: left minus right, crossed with up) is turned to the camera (+z), so the front always faces it.
    let joint = |n: &str| rig.rig.find(n).and_then(|i| records.get(i)).map(|r| r.p);
    let mut side = Vec3::ZERO;
    for (l, r) in [("LeftUpLeg", "RightUpLeg"), ("LeftArm", "RightArm"), ("LeftShoulder", "RightShoulder")] {
        if let (Some(l), Some(r)) = (joint(l), joint(r)) {
            side += Vec3::new(l.x - r.x, 0.0, l.z - r.z).normalize_or_zero();
        }
    }
    let facing = side.cross(Vec3::Y);
    let turn_by = if facing.length_squared() > 1e-6 {
        let t = -facing.x.atan2(facing.z);
        rig.idle_heading = Some(t);
        t
    } else {
        rig.idle_heading.unwrap_or(0.0)
    };
    let turn = Quat::from_rotation_y(turn_by);
    // Planted: the point between the feet stays put (the hips sway over it).
    let feet = match (joint("LeftFoot"), joint("RightFoot")) {
        (Some(l), Some(r)) => Some((l + r) / 2.0),
        _ => None,
    };
    let centre = feet.or_else(|| joint("Hips")).or_else(|| records.get(rig.board).map(|b| b.p)).unwrap_or(Vec3::ZERO);
    let centre = Vec3::new(centre.x, 0.0, centre.z);
    for r in &mut records {
        r.p = turn * (r.p - centre);
        r.q = (turn * r.q).normalize();
    }
    show_board(rig, &mut records);
    big_parts(rig, &mut records);
    for (&entity, r) in rig.nodes.iter().zip(&records) {
        if let Ok(mut t) = transforms.get_mut(entity) {
            *t = record_transform(r);
        }
    }
    if let Some((entity, extra)) = rig.board_entity
        && let Some(board) = records.get(rig.board)
        && let Ok(mut t) = transforms.get_mut(entity)
    {
        *t = board_transform(rig, board, extra);
    }
    true
}

pub fn animate(rig: &mut RiderRig, input: &crate::rider::anim::AnimInput, transforms: &mut Query<&mut Transform>) {
    let start = Instant::now();
    let mut records = rig.anim.update(input).to_vec();
    props_in_hands(rig, &mut records, input.state);
    rig.drawn_gear_lights = rig.gear_lights.iter().map(|g| compose(&records[g.joint], &g.local)).collect();
    for (&entity, r) in rig.nodes.iter().zip(&records) {
        if let Ok(mut t) = transforms.get_mut(entity) {
            *t = record_transform(r);
        }
    }
    let root = input.roots.0.lerp(input.roots.1, input.frac.clamp(0.0, 1.0));
    rig.drawn_trajectory =
        rig.anim.trajectory_joint().and_then(|t| records.get(t)).map_or((root, Quat::IDENTITY), |r| (r.p, r.q));
    let board = records.get(rig.board).copied();
    rig.drawn_board = board.map_or(root, |r| r.p);
    if let Some((entity, extra)) = rig.board_entity
        && let Some(board) = board
        && let Ok(mut t) = transforms.get_mut(entity)
    {
        *t = board_transform(rig, &board, extra);
    }
    let ms = start.elapsed().as_secs_f32() * 1000.0;
    rig.anim_ms = if rig.anim_ms == 0.0 { ms } else { rig.anim_ms + 0.05 * (ms - rig.anim_ms) };
}
