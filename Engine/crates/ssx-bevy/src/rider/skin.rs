use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use bevy::camera::visibility::DynamicSkinnedMeshBounds;
use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::prelude::*;
use ssx_assets::character::model_matrix;
use ssx_assets::character::{Character, Loadout, Srt, load_character, load_character_with};
use ssx_assets::track_lights::SceneLight;
use ssx_formats::crsf::SceneGraph;

use crate::render::lighting::game::GameLightAssets;
use crate::render::objects::foreground::ForegroundMaterial;
use crate::render::objects::material::ObjectMaterial;
use crate::render::objects::{LoadedPart, MaterialHandle, part_from_mesh};
use crate::rider::anim::{JointRecord, RiderAnimation, world_records};
use crate::rider::material::{CharacterLightUniform, CharacterMaterial};

struct SkinnedPart {
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
    source: (PathBuf, String, String),
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

pub fn load_rider(big_root: &Path, track: &str, name: &str) -> Result<LoadedRider> {
    let start = Instant::now();
    let character = load_character(big_root, track, name)?;
    if character.parts.is_empty() {
        anyhow::bail!("{name}: no part loaded ({})", character.problems.join("; "));
    }
    let summary = character.summary();
    let mut problems = character.problems.clone();
    let loaded = RiderAnimation::load(big_root, &character, name)?;
    problems.extend(loaded.problems);
    let (anim, rig, bind) = (loaded.anim, loaded.rig, loaded.bind);
    let board = rig.find("Board").context("the rig has no Board joint")?;
    let (parts, gear_lights) = load_parts(big_root, character, &rig, &mut problems);
    let source = (big_root.to_path_buf(), track.to_string(), name.to_string());
    let load_secs = start.elapsed().as_secs_f32();
    Ok(LoadedRider { summary, rig, bind, board, parts, anim, gear_lights, source, problems, load_secs })
}

fn load_parts(
    big_root: &Path,
    character: Character,
    rig: &SceneGraph,
    problems: &mut Vec<String>,
) -> (Vec<SkinnedPart>, Vec<GearLight>) {
    let mut parts = Vec::new();
    for p in character.parts {
        let label = p.mesh_name.clone();
        let wingsuit = p.material.effect.as_deref() == Some("player_wingsuit.fx");
        let loaded = part_from_mesh(big_root, p.mesh, &label, &p.material)
            .and_then(|part| part.with_skin(p.skin.joints, p.skin.weights));
        match loaded {
            Ok(part) => {
                problems.extend(part.problems.iter().map(|e| format!("{label}: {e}")));
                parts.push(SkinnedPart {
                    slot: p.slot,
                    wingsuit,
                    part,
                    palette: p.skin.palette.iter().map(|j| j.node).collect(),
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
    pub fn dress(&mut self, loadout: &Loadout) -> Result<()> {
        let start = Instant::now();
        let (big_root, track, name) = self.source.clone();
        let character = load_character_with(&big_root, &track, &name, loadout)?;
        if character.parts.is_empty() {
            anyhow::bail!("{name}: no part loaded ({})", character.problems.join("; "));
        }
        self.summary = character.summary();
        let mut problems = character.problems.clone();
        let (parts, lights) = load_parts(&big_root, character, &self.rig, &mut problems);
        (self.parts, self.gear_lights) = (parts, lights);
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
    pub wingsuit_materials: Vec<Handle<CharacterMaterial>>,
    pub gear_lights: Vec<GearLight>,
    pub drawn_gear_lights: Vec<JointRecord>,
    props: Vec<PropInHands>,
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
    let mut character_materials = Vec::new();
    let mut wingsuit_materials = Vec::new();
    for p in rider.parts {
        let inverse_bindposes = bindposes.add(SkinnedMeshInverseBindposes::from(p.inverse_binds));
        let joints = p.palette.iter().map(|&n| nodes[n]).collect();
        let bundle = (
            Name::new(format!("rider {}", p.slot)),
            SkinnedMesh { inverse_bindposes, joints },
            DynamicSkinnedMeshBounds,
            Transform::default(),
            ChildOf(root),
        );
        if p.part.is_character() {
            let (mesh, material) = p.part.add_character(meshes, images, light, rider_light, characters);
            if p.wingsuit {
                if let Some(mut m) = characters.get_mut(&material) {
                    m.base.alpha_mode = AlphaMode::Mask(0.25);
                }
                wingsuit_materials.push(material.clone());
            }
            character_materials.push(material.clone());
            commands.spawn((bundle, Mesh3d(mesh), MeshMaterial3d(material)));
            continue;
        }
        let (mesh, material) = p.part.add(meshes, images, light, objects, foreground);
        let mut entity = commands.spawn((bundle, Mesh3d(mesh)));
        match material {
            MaterialHandle::Object(m) => entity.insert(MeshMaterial3d(m)),
            MaterialHandle::Foreground(m) => entity.insert(MeshMaterial3d(m)),
        };
    }
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
        wingsuit_materials,
        gear_lights: rider.gear_lights,
        drawn_gear_lights: Vec::new(),
        props,
    }
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
