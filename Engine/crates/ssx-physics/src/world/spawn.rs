use std::path::Path;

use anyhow::{Context, Result};
use glam::{Quat, Vec3};
use ssx_assets::track_objects::Placement;
use ssx_assets::vault_db::{fnv1, refs, single, single_text, with_track_db};
use ssx_formats::attrib::{FieldData, Value};
use ssx_formats::spdt::{SpdtCells, SpdtFile};

use crate::character::{GearStats, Tuning};
use crate::collision::CollisionWorld;
use crate::rider::World;
use crate::triggers::{TriggerScope, TriggerSet, TriggerVolume, TriggerVolumeRecord};
use crate::tuning::Bundle;
use crate::{props, terrain};

const CLASS_DROP: u32 = fnv1("ge_drop");
const CLASS_GAMEOBJECT: u32 = fnv1("ge_scene_gameobject");
const FIELD_POB: u32 = fnv1("POBFileRef");
const CLASS_SCENE_INSTANCE: u32 = fnv1("ge_scene_instance");
const FIELD_TRACK: u32 = fnv1("Track");
const FIELD_DROP_SCENE_INSTANCES: u32 = 0x5050_DE8B;
const FIELD_GAMEOBJECT: u32 = 0x3ED7_B5CE;
const FIELD_INSTANCE_DATA: u32 = 0xEE66_32AC;
const FIELD_DROP_MESHES: u32 = fnv1("Meshes");
const CLASS_MESHSUBD: u32 = 0xB9DD_6F89;
const FIELD_FILE_REF: u32 = 0xDF9E_F8B2;
pub const START_LOCATOR_OBJECT: u32 = 0x1343_4084;
const CLASS_TRACK: u32 = fnv1("ge_track");
const FIELD_TRACK_TERRAIN: u32 = 0x3132_67E8;
const FIELD_TERRAIN_INSTANCES: u32 = 0x0903_EF8F;
const FIELD_TRIGGER_VOLUMES: u32 = fnv1("TriggerVolumes");
const FIELD_TV_SCALE: u32 = fnv1("Scale");
const FIELD_TV_POSITION: u32 = fnv1("Position");
const FIELD_TV_ORIENTATION: u32 = fnv1("Orientation");
const FIELD_TV_NAME: u32 = 0xBF29_BFFE;
const FIELD_TV_TYPE: u32 = 0x7F6F_3C0B;
const FIELD_TV_SHAPE: u32 = 0xCC65_CA4E;

#[derive(Clone, Debug)]
pub struct DropStart {
    pub drop_key: u32,
    pub slots: Vec<Placement>,
    pub terrain_files: Vec<String>,
}

impl DropStart {
    pub fn slot(&self, i: usize) -> Option<(Vec3, Quat)> {
        let p = self.slots.get(i)?;
        let q = Quat::from_xyzw(p.rotation[0], p.rotation[1], p.rotation[2], p.rotation[3]).normalize();
        Some((Vec3::from_array(p.position), q))
    }
}

pub fn load_drop_starts(big_root: &Path, track: &str) -> Result<Vec<DropStart>> {
    let track_key = fnv1(&format!("{track}_track"));
    with_track_db(big_root, track, |db| {
        let mut out = Vec::new();
        for (key, _) in db.collections_of(1, CLASS_DROP).collect::<Vec<_>>() {
            let fields = db.fields(CLASS_DROP, key)?;
            let field = |k: u32| fields.iter().find(|f| f.key == k);
            if !matches!(field(FIELD_TRACK).and_then(|f| single(&f.data)), Some(Value::Ref(r)) if r.collection == track_key)
            {
                continue;
            }
            let mut terrain_files = Vec::new();
            for m in field(FIELD_DROP_MESHES).map(|f| refs(&f.data)).unwrap_or_default() {
                if let Some(path) =
                    db.field(CLASS_MESHSUBD, m.collection, FIELD_FILE_REF)?.and_then(|f| single_text(&f.data))
                {
                    terrain_files.push(path.rsplit(['\\', '/']).next().unwrap_or(&path).to_owned());
                }
            }
            let Some(instances) = field(FIELD_DROP_SCENE_INSTANCES) else { continue };
            for si in refs(&instances.data).into_iter().filter(|r| r.collection != 0) {
                let sif = db.fields(CLASS_SCENE_INSTANCE, si.collection)?;
                let reference = |k: u32| sif.iter().find(|f| f.key == k).and_then(|f| refs(&f.data).first().copied());
                if reference(FIELD_GAMEOBJECT).is_none_or(|o| o.collection != START_LOCATOR_OBJECT) {
                    continue;
                }
                let Some(data) = reference(FIELD_INSTANCE_DATA).filter(|d| d.collection != 0) else { continue };
                let slots = match db.field(data.class, data.collection, FIELD_INSTANCE_DATA)? {
                    Some(f) => decode_placements(&f.data),
                    None => Vec::new(),
                };
                if !slots.is_empty() {
                    out.push(DropStart { drop_key: key, slots, terrain_files });
                    break;
                }
            }
        }
        Ok(out)
    })
}

pub const END_LOCATOR_OBJECT: u32 = 0x9297_7821;

pub fn load_end_locator(big_root: &Path, track: &str, drop_key: u32) -> Result<Option<Vec3>> {
    with_track_db(big_root, track, |db| {
        let mut list = drop_scene_instances(db, drop_key)?;
        list.extend(track_scene_instances(db, track)?);
        for (class, key) in list {
            let Ok(sif) = db.fields(class, key) else { continue };
            let reference = |k: u32| sif.iter().find(|f| f.key == k).and_then(|f| refs(&f.data).first().copied());
            if reference(FIELD_GAMEOBJECT).is_none_or(|o| o.collection != END_LOCATOR_OBJECT) {
                continue;
            }
            let Some(data) = reference(FIELD_INSTANCE_DATA).filter(|d| d.collection != 0) else { continue };
            if let Some(f) = db.field(data.class, data.collection, FIELD_INSTANCE_DATA)?
                && let Some(p) = decode_placements(&f.data).first()
            {
                return Ok(Some(Vec3::from_array(p.position)));
            }
        }
        Ok(None)
    })
}

fn decode_placements(d: &FieldData) -> Vec<Placement> {
    let vals: Vec<&Value> = match d {
        FieldData::Single(v) => vec![v],
        FieldData::Array(vs) => vs.iter().collect(),
    };
    vals.into_iter()
        .filter_map(|v| if let Value::Raw { bytes, .. } = v { Placement::decode(bytes) } else { None })
        .collect()
}

pub fn load_trigger_volumes(big_root: &Path, track: &str, drop_key: Option<u32>) -> Result<TriggerSet> {
    with_track_db(big_root, track, |db| {
        let mut set = TriggerSet::default();
        for si in track_scene_instances(db, track)? {
            add_instance_volumes(db, &mut set, si, TriggerScope::Track)?;
        }
        if let Some(key) = drop_key {
            for si in drop_scene_instances(db, key)? {
                add_instance_volumes(db, &mut set, si, TriggerScope::Drop(key))?;
            }
        }
        let mut objects: Vec<u32> = set.entities.iter().map(|e| e.object).collect();
        objects.sort_unstable();
        objects.dedup();
        for (object, script) in ssx_assets::track_particles::object_controllers(db, &objects)? {
            let Some(path) = ssx_assets::track_objects::asset_file(big_root, &script) else { continue };
            if let Ok(src) = std::fs::read_to_string(path) {
                set.scripts.insert(object, (script, std::sync::Arc::from(src)));
            }
        }
        for &object in &objects {
            let Some(pob) = db.field(CLASS_GAMEOBJECT, object, FIELD_POB)?.and_then(|f| single_text(&f.data)) else {
                continue;
            };
            let Some(path) = ssx_assets::track_objects::asset_file(big_root, &pob) else { continue };
            let Ok(data) = std::fs::read(&path) else { continue };
            let pob = ssx_formats::pob::PobFile::parse(&data).with_context(|| path.display().to_string())?;
            if let Some(def) = crate::rigid::RigidBodyDef::from_pob(&pob) {
                set.bodies.insert(object, def);
            }
        }
        Ok(set)
    })
}

pub fn track_drop_keys(db: &ssx_assets::vault_db::Db<'_>, track: &str) -> Result<Vec<u32>> {
    let track_key = fnv1(&format!("{track}_track"));
    let mut out = Vec::new();
    for (key, _) in db.collections_of(1, CLASS_DROP).collect::<Vec<_>>() {
        if matches!(db.field(CLASS_DROP, key, FIELD_TRACK)?.and_then(|f| single(&f.data)),
            Some(Value::Ref(r)) if r.collection == track_key)
        {
            out.push(key);
        }
    }
    Ok(out)
}

pub fn track_scene_instances(db: &ssx_assets::vault_db::Db<'_>, track: &str) -> Result<Vec<(u32, u32)>> {
    let track_key = fnv1(&format!("{track}_track"));
    let mut out = Vec::new();
    let Some(terrains) = db.field(CLASS_TRACK, track_key, FIELD_TRACK_TERRAIN)? else { return Ok(out) };
    for t in refs(&terrains.data).into_iter().filter(|r| r.collection != 0) {
        let Some(cols) = db.field(t.class, t.collection, FIELD_TERRAIN_INSTANCES)? else { continue };
        for c in refs(&cols.data).into_iter().filter(|r| r.collection != 0) {
            if let Some(list) = db.field(c.class, c.collection, FIELD_DROP_SCENE_INSTANCES)? {
                out.extend(refs(&list.data).into_iter().filter(|r| r.collection != 0).map(|r| (r.class, r.collection)));
            }
        }
    }
    Ok(out)
}

pub fn drop_scene_instances(db: &ssx_assets::vault_db::Db<'_>, drop_key: u32) -> Result<Vec<(u32, u32)>> {
    Ok(match db.field(CLASS_DROP, drop_key, FIELD_DROP_SCENE_INSTANCES)? {
        Some(f) => refs(&f.data).into_iter().filter(|r| r.collection != 0).map(|r| (r.class, r.collection)).collect(),
        None => Vec::new(),
    })
}

pub fn add_instance_volumes(
    db: &ssx_assets::vault_db::Db<'_>,
    set: &mut TriggerSet,
    (class, key): (u32, u32),
    scope: TriggerScope,
) -> Result<()> {
    let sif = db.fields(class, key)?;
    let reference = |k: u32| sif.iter().find(|f| f.key == k).and_then(|f| refs(&f.data).first().copied());
    let (Some(obj), Some(data)) = (reference(FIELD_GAMEOBJECT), reference(FIELD_INSTANCE_DATA)) else {
        return Ok(());
    };
    if obj.collection == 0 || data.collection == 0 {
        return Ok(());
    }
    let volumes: Vec<_> = match db.field(obj.class, obj.collection, FIELD_TRIGGER_VOLUMES)? {
        Some(f) => refs(&f.data).into_iter().filter(|r| r.collection != 0).collect(),
        None => Vec::new(),
    };
    let controlled =
        || -> Result<bool> { Ok(!ssx_assets::track_particles::object_controllers(db, &[obj.collection])?.is_empty()) };
    if volumes.is_empty() && !controlled()? {
        return Ok(());
    }
    let placements = match db.field(data.class, data.collection, FIELD_INSTANCE_DATA)? {
        Some(f) => decode_placements(&f.data),
        None => Vec::new(),
    };
    {
        for (i, pl) in placements.iter().enumerate() {
            set.entities.push(crate::triggers::TriggerEntity {
                object: obj.collection,
                instance: key,
                placement: i,
                scope,
                position: Vec3::from_array(pl.position),
                rotation: Quat::from_array(pl.rotation).normalize(),
            });
        }
    }
    for v in volumes {
        let rec = trigger_volume_record(db, v.class, v.collection)?;
        for (i, pl) in placements.iter().enumerate() {
            let mut tv = TriggerVolume::place(&rec, pl);
            (tv.object, tv.instance, tv.placement, tv.scope) = (obj.collection, key, i, scope);
            set.volumes.push(tv);
        }
    }
    Ok(())
}

pub fn trigger_volume_record(db: &ssx_assets::vault_db::Db<'_>, class: u32, key: u32) -> Result<TriggerVolumeRecord> {
    let fields = db.fields(class, key)?;
    let get = |k: u32| fields.iter().find(|f| f.key == k).and_then(|f| single(&f.data));
    let vec4 = |k: u32| match get(k) {
        Some(Value::Vector4(v)) => Ok(*v),
        other => anyhow::bail!("trigger volume {key:#010x} field {k:#010x}: expected a Vector4, got {other:?}"),
    };
    let text = |k: u32| match get(k) {
        Some(Value::Text(t)) => t.clone().unwrap_or_default(),
        _ => String::new(),
    };
    let shape = match get(FIELD_TV_SHAPE) {
        Some(Value::Int32(s)) => *s,
        other => anyhow::bail!("trigger volume {key:#010x}: shape {other:?}"),
    };
    let (s, p, q) = (vec4(FIELD_TV_SCALE)?, vec4(FIELD_TV_POSITION)?, vec4(FIELD_TV_ORIENTATION)?);
    Ok(TriggerVolumeRecord {
        name: text(FIELD_TV_NAME),
        type_text: text(FIELD_TV_TYPE),
        shape,
        scale: Vec3::new(s[0], s[1], s[2]),
        position: Vec3::new(p[0], p[1], p[2]),
        orientation: Quat::from_xyzw(q[0], q[1], q[2], q[3]),
    })
}

pub fn track_world(big_root: &Path, track_dir: &Path, track: &str, gear_mix: f32, with_props: bool) -> Result<World> {
    build_world(big_root, track_dir, track, gear_mix, with_props, None)
}

pub fn track_world_for_drop(
    big_root: &Path,
    track_dir: &Path,
    track: &str,
    gear_mix: f32,
    with_props: bool,
    drop: usize,
) -> Result<World> {
    build_world(big_root, track_dir, track, gear_mix, with_props, Some(drop))
}

fn build_world(
    big_root: &Path,
    track_dir: &Path,
    track: &str,
    gear_mix: f32,
    with_props: bool,
    drop: Option<usize>,
) -> Result<World> {
    let starts = load_drop_starts(big_root, track).unwrap_or_default();
    let files = match drop {
        Some(d) => Some(
            starts.get(d).with_context(|| format!("{track} has no drop {d} with a start"))?.terrain_files.as_slice(),
        ),
        None => None,
    };
    let mut collision = CollisionWorld::default();
    terrain::add_track_terrain_files(&mut collision, track_dir, files)?;
    if with_props {
        props::add_track_props(&mut collision, big_root, track)?;
    }
    collision.finish();
    let bundle = Bundle::load(big_root)?;
    let tuning = Tuning::load(&bundle)?;
    let props = tuning.resolve(gear_mix)?;
    let drop_key = starts.get(drop.unwrap_or(0)).map(|d| d.drop_key);
    let triggers = load_trigger_volumes(big_root, track, drop_key)
        .with_context(|| format!("{track}: trigger volumes of drop {:?}", drop_key))?;
    let (events, end_locator) = match drop_key {
        Some(k) => load_drop_events(big_root, track, k)?,
        None => (Vec::new(), None),
    };
    let survival = match drop_key {
        Some(k) => load_drop_survival(big_root, track, k).with_context(|| format!("{track}: survival of {k:08x}"))?,
        None => Default::default(),
    };
    Ok(World { collision, tuning, props, starts, triggers, events, end_locator, survival })
}

pub fn load_drop_events(
    big_root: &Path,
    track: &str,
    drop_key: u32,
) -> Result<(Vec<crate::event::EventDef>, Option<Vec3>)> {
    let mut events: Vec<crate::event::EventDef> =
        crate::event::load_events(big_root)?.into_iter().filter(|e| e.drop_key == drop_key).collect();
    let drop = crate::objectives::DropScripts::load(big_root, drop_objective_scripts(big_root, track, drop_key)?);
    for e in &mut events {
        e.objectives = Some(std::sync::Arc::new(crate::objectives::EventObjectives::new(big_root, &drop, e)));
    }
    Ok((events, load_end_locator(big_root, track, drop_key)?))
}

pub fn track_world_with_gear(
    big_root: &Path,
    track_dir: &Path,
    track: &str,
    gear: &GearStats,
    with_props: bool,
) -> Result<World> {
    let mut w = track_world(big_root, track_dir, track, crate::character::DEFAULT_GEAR_MIX, with_props)?;
    w.props = w.tuning.resolve_gear(gear)?;
    Ok(w)
}

pub struct Boundary(SpdtFile);

impl Boundary {
    pub fn load(track_dir: &Path, track: &str) -> Option<Self> {
        let path = track_dir.join(format!("{track}_track_boundary.spdt"));
        let file = SpdtFile::parse(&std::fs::read(path).ok()?).ok()?;
        matches!(file.cells, SpdtCells::Boundary(_)).then_some(Self(file))
    }

    pub fn cell(&self, p: Vec3) -> Option<u16> {
        let f = &self.0;
        let [cx, cz] = f.cell_size();
        let max_x = f.bbox_min[0] + cx * f32::from(f.width) - 0.01;
        let max_z = f.bbox_min[2] + cz * f32::from(f.height) - 0.01;
        let (x, z) = (p.x.clamp(f.bbox_min[0], max_x), p.z.clamp(f.bbox_min[2], max_z));
        let ix = (((x - f.bbox_min[0]) / cx).floor() as usize).min(usize::from(f.width) - 1);
        let iz = (((z - f.bbox_min[2]) / cz).floor() as usize).min(usize::from(f.height) - 1);
        let SpdtCells::Boundary(cells) = &f.cells else { return None };
        cells.get(iz * f.width as usize + ix).copied()
    }

    pub fn in_bounds(&self, p: Vec3) -> bool {
        self.cell(p).is_some_and(|c| c & 0xC000 == 0)
    }

    pub fn push_direction(&self, p: Vec3) -> Option<Vec3> {
        let c = self.cell(p)?;
        let a = f32::from((c >> 8) & 0x3F) * std::f32::consts::TAU / 63.0;
        Some(Vec3::new(a.sin(), 0.0, a.cos()))
    }
}

pub fn drop_start(world: &World, drop: usize) -> Option<(Vec3, Vec3)> {
    let (at, q) = world.starts.get(drop)?.slot(0)?;
    let heading = Vec3::new((q * Vec3::Z).x, 0.0, (q * Vec3::Z).z).try_normalize()?;
    Some((at, heading))
}

pub fn spawn_pose(
    world: &World,
    _bounds: Option<&Boundary>,
    position: Option<Vec3>,
    heading: Option<Vec3>,
) -> Result<(Vec3, Vec3)> {
    spawn_pose_for_drop(world, 0, position, heading)
}

pub fn spawn_pose_for_drop(
    world: &World,
    drop: usize,
    position: Option<Vec3>,
    heading: Option<Vec3>,
) -> Result<(Vec3, Vec3)> {
    let start = drop_start(world, drop);
    let at = match (position, start) {
        (Some(p), _) => p,
        (None, Some((p, _))) => p,
        (None, None) => highest_terrain_vertex(&world.collision).context("no terrain")?,
    };
    let fall_line = || {
        world
            .collision
            .ray_cast(at + Vec3::Y * 1.0, -Vec3::Y, 101.0)
            .and_then(|h| Vec3::new(h.normal.x, 0.0, h.normal.z).try_normalize())
            .unwrap_or(Vec3::Z)
    };
    let heading = heading.or(start.filter(|_| position.is_none()).map(|s| s.1)).unwrap_or_else(fall_line);
    Ok((at, heading))
}

pub fn level_start_pose(
    world: &World,
    drop: usize,
    position: Option<Vec3>,
    heading: Option<Vec3>,
) -> Result<(Vec3, Quat)> {
    let (at, h) = spawn_pose_for_drop(world, drop, position, heading)?;
    Ok((at, level_start_orientation(world, at, h)))
}

pub fn level_start_orientation(world: &World, at: Vec3, heading: Vec3) -> Quat {
    let flat = Vec3::new(heading.x, 0.0, heading.z).try_normalize().unwrap_or(Vec3::Z);
    for d in &world.starts {
        if let Some((p, q)) = d.slot(0)
            && p.distance(at) < 1e-3
            && Vec3::new((q * Vec3::Z).x, 0.0, (q * Vec3::Z).z).normalize_or_zero().dot(flat) > 0.9999
        {
            return q;
        }
    }
    Quat::from_rotation_y(flat.x.atan2(flat.z))
}

pub fn highest_terrain_vertex(world: &CollisionWorld) -> Option<Vec3> {
    world
        .clusters
        .iter()
        .filter(|c| c.is_terrain())
        .flat_map(|c| &world.vertices[c.vert_start as usize..c.vert_start as usize + c.vert_count as usize])
        .copied()
        .max_by(|a, b| a.y.total_cmp(&b.y))
}

const FIELD_DROP_OBJECTIVE_SCRIPTS: u32 = 0xB3E0_D079;
const FIELD_DROP_SCENE_SCRIPTS: u32 = 0x085A_9D8A;
pub const SCRIPTS_DIR: &str = "memresident/data/common/scripts";

pub fn drop_objective_scripts(big_root: &Path, track: &str, drop_key: u32) -> Result<Vec<String>> {
    drop_script_list(big_root, track, drop_key, FIELD_DROP_OBJECTIVE_SCRIPTS)
}

pub fn drop_scene_scripts(big_root: &Path, track: &str, drop_key: u32) -> Result<Vec<String>> {
    drop_script_list(big_root, track, drop_key, FIELD_DROP_SCENE_SCRIPTS)
}

fn drop_script_list(big_root: &Path, track: &str, drop_key: u32, field_hash: u32) -> Result<Vec<String>> {
    with_track_db(big_root, track, |db| {
        let field = db.field(CLASS_DROP, drop_key, field_hash)?;
        let values = match field.map(|f| &f.data) {
            Some(FieldData::Array(vs)) => vs.iter().collect::<Vec<_>>(),
            Some(FieldData::Single(v)) => vec![v],
            None => Vec::new(),
        };
        Ok(values
            .into_iter()
            .filter_map(|v| if let Value::Text(Some(t)) = v { Some(t.clone()) } else { None })
            .collect())
    })
}

pub fn load_drop_survival(big_root: &Path, track: &str, drop_key: u32) -> Result<crate::survival::DropSurvival> {
    use crate::objectives::{DropScripts, GsValue, Probe};
    let mut scripts = DropScripts::load(big_root, drop_objective_scripts(big_root, track, drop_key)?);
    scripts.add_scene(big_root, drop_scene_scripts(big_root, track, drop_key)?);
    let p = Probe::run(&scripts.level_batch(None), None, crate::character::GearStats::default_gear().slots);
    let objective_tuning_set = match p.value("Player1", "INT_AvalancheTuningSet") {
        Some(GsValue::Int(v)) => Some(v),
        _ => None,
    };
    let mut d = crate::survival::DropSurvival {
        cold: p.flag("ColdDrop"),
        low_oxygen: p.flag("LowOxygenDrop"),
        rock_damage: p.set_true("Player1", "BLN_EnableRockDamage"),
        tripping: p.set_true("Player1", "BLN_EnableTripping"),
        avalanche: crate::avalanche::DropAvalanche {
            objective_tuning_set,
            avalanche_drop: p.flag("AvalancheDrop"),
            reverse_cam: p.value("TheCamera", "INT_CameraType") == Some(GsValue::Int(2)),
        },
        objectives: Some(std::sync::Arc::new(scripts)),
        ..Default::default()
    };
    let drops = ssx_assets::track_lighting::load_track_drops(big_root, track)?;
    let preset = drops.iter().find(|x| x.key == drop_key).map(|x| &x.lighting);
    if let Some(p) = preset
        && let Some(v) = p.vector("dirLight_Direction")
    {
        let sun = Vec3::new(v[0], v[1], v[2]).normalize_or_zero();
        d.sun = (sun + Vec3::Y * p.float("dirLight_Offset").unwrap_or(0.0)).try_normalize();
    }
    // TODO(guess): the drop's preset is the lighting service's current one when the objectives test it and `HeadLampGearTrack` reads `headLampScaleFactor` (an override or blend could change it); settle: the service's +0x20.
    crate::guess_hit!();
    d.night = preset.and_then(|p| p.float("isNightCondition")).is_some_and(|v| v > 0.0);
    d.head_lamp_scale = preset.and_then(|p| p.float("headLampScaleFactor")).unwrap_or(0.0);
    Ok(d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssx_formats::spdt::SpdtKind;

    fn grid(cells: Vec<u16>) -> Boundary {
        Boundary(SpdtFile {
            kind: SpdtKind::Boundary,
            type_name: "snowboun".into(),
            version: 1,
            bbox_min: [-15.0, 0.0, 0.0],
            bbox_max: [15.0, 100.0, 20.0],
            width: 3,
            height: 2,
            cells: SpdtCells::Boundary(cells),
        })
    }

    #[test]
    fn boundary_bits_clamping_and_push_direction() {
        let b = grid(vec![0x0012, 0x4000, 0x8000, 0x3F00 & (21 << 8), 0x00FF, 0xC000]);
        assert!(b.in_bounds(Vec3::new(-10.0, 0.0, 5.0)));
        assert!(!b.in_bounds(Vec3::new(0.0, 0.0, 5.0)) && !b.in_bounds(Vec3::new(10.0, 0.0, 5.0)));
        assert!(b.in_bounds(Vec3::new(-10.0, 0.0, 15.0)) && b.in_bounds(Vec3::new(0.0, 0.0, 15.0)));
        assert!(!b.in_bounds(Vec3::new(10.0, 0.0, 15.0)));
        assert_eq!(b.cell(Vec3::new(-500.0, 0.0, -500.0)), Some(0x0012));
        assert_eq!(b.cell(Vec3::new(500.0, 0.0, 500.0)), Some(0xC000));
        let d = b.push_direction(Vec3::new(-10.0, 0.0, 15.0)).expect("cell");
        let a = 21.0 * std::f32::consts::TAU / 63.0;
        assert!(d.distance(Vec3::new(a.sin(), 0.0, a.cos())) < 1e-6);
        assert!(b.push_direction(Vec3::new(-10.0, 0.0, 5.0)).is_some_and(|d| d.distance(Vec3::Z) < 1e-6));
    }

    #[test]
    fn the_start_locator_slot_gives_position_and_heading() {
        let mut w = crate::testutil::world(crate::testutil::plane(100.0, 0.2));
        let q = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let slot = Placement { scale: [1.0; 3], rotation: q.to_array(), position: [3.0, 1.0, 4.0], unk: [0.0; 6] };
        w.starts = vec![DropStart { drop_key: 1, slots: vec![slot], terrain_files: Vec::new() }];
        let (at, heading) = spawn_pose_for_drop(&w, 0, None, None).expect("start");
        assert!(at.distance(Vec3::new(3.0, 1.0, 4.0)) < 1e-6 && heading.distance(Vec3::X) < 1e-5);
        let (_, h) = spawn_pose_for_drop(&w, 0, Some(Vec3::new(0.0, 5.0, 0.0)), None).expect("pose");
        assert!(h.distance(Vec3::Z) < 1e-5, "{h:?}");
        w.starts.clear();
        assert!(spawn_pose(&w, None, None, None).is_err());
    }
}
