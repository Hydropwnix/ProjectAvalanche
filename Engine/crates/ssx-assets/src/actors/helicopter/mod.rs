mod animation;

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use ssx_formats::attrib::collection::FieldData;
use ssx_formats::attrib::{Value, fnv1};
use ssx_formats::crsf::{CrsfFile, SceneGraph};

use crate::character::{Skin, Srt};
use crate::ghosts::{CLASS_GAMEOBJECT, FIELD_LOD_THRESHOLDS, FIELD_MODELS, bind_pose, part_refs, skin_of};
use crate::object_mesh::ObjectMesh;
use crate::track_objects::asset_file;
use crate::vault_db::{Material, refs, with_level_db};

pub use animation::*;

const CLASS_UI_DROP: u32 = fnv1(b"ui_drop");
const CLASS_GE_DROP: u32 = fnv1(b"ge_drop");
const CLASS_PEAK: u32 = fnv1(b"peak");
const CLASS_RANGE: u32 = fnv1(b"range");
const CLASS_HELICOPTER: u32 = fnv1(b"helicopter");
const FIELD_UI_DROP_GE_DROP: u32 = 0xB9C2_CFC5;
const FIELD_UI_DROP_HELICOPTER: u32 = 0x2101_12EA;
const FIELD_UI_DROP_PEAK: u32 = 0x8296_DA5A;
const FIELD_GE_DROP_PEAK: u32 = 0x17E8_41D0;
const FIELD_PEAK_RANGE: u32 = 0xAB6B_2547;
const FIELD_RANGE_HELICOPTER: u32 = fnv1(b"helicopter");
const FIELD_HELI_GAMEOBJECT: u32 = 0x58F5_AE0E;
const FIELD_HELI_NAME: u32 = 0x8097_DCBE;
const FIELD_HELI_DISPLAY: u32 = 0x5A0F_251D;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeliSource {
    Drop,
    Range,
    Default,
}

pub const DEFAULT_HELICOPTER: u32 = 0xB48A_C47B;

#[derive(Clone, Debug, PartialEq)]
pub struct HeliChoice {
    pub key: u32,
    pub source: HeliSource,
    pub peak: u32,
    pub range: u32,
    pub name: String,
    pub display: String,
    pub gameobject: u32,
}

pub(crate) struct Vaults(crate::career::data::Vaults);

impl Vaults {
    pub(crate) fn load(big_root: &Path) -> Result<Self> {
        Ok(Self(crate::career::data::Vaults::load(big_root)?))
    }

    pub(crate) fn reference(&self, class: u32, coll: u32, key: u32) -> Result<u32> {
        Ok(self.0.rec(class, coll)?.reference(key))
    }

    fn text(&self, class: u32, coll: u32, key: u32) -> Result<Option<String>> {
        Ok(self.0.rec(class, coll)?.text(key))
    }
}

pub fn drop_helicopter(big_root: &Path, ui_drop: &str) -> Result<HeliChoice> {
    choice(&Vaults::load(big_root)?, fnv1(ui_drop.as_bytes())).with_context(|| format!("ui_drop {ui_drop}"))
}

pub fn default_helicopter(big_root: &Path) -> Result<HeliChoice> {
    helicopter(&Vaults::load(big_root)?, DEFAULT_HELICOPTER, HeliSource::Default, 0, 0)
}

pub fn survey(big_root: &Path) -> Result<Vec<(u32, Result<HeliChoice>)>> {
    let v = Vaults::load(big_root)?;
    let keys = v.0.keys_of(CLASS_UI_DROP);
    Ok(keys.into_iter().map(|k| (k, choice(&v, k))).collect())
}

pub fn drop_range(big_root: &Path, ui_drop: &str) -> Result<(u32, u32)> {
    peak_range(&Vaults::load(big_root)?, fnv1(ui_drop.as_bytes())).with_context(|| format!("ui_drop {ui_drop}"))
}

fn peak_range(v: &Vaults, drop: u32) -> Result<(u32, u32)> {
    let mut peak = v.reference(CLASS_UI_DROP, drop, FIELD_UI_DROP_PEAK)?;
    if peak == 0 {
        let ge_drop = v.reference(CLASS_UI_DROP, drop, FIELD_UI_DROP_GE_DROP)?;
        let name = v
            .text(CLASS_GE_DROP, ge_drop, FIELD_GE_DROP_PEAK)?
            .with_context(|| format!("ge_drop {ge_drop:08x} names no peak"))?;
        peak = fnv1(name.as_bytes());
    }
    Ok((peak, v.reference(CLASS_PEAK, peak, FIELD_PEAK_RANGE)?))
}

fn choice(v: &Vaults, drop: u32) -> Result<HeliChoice> {
    let own = v.reference(CLASS_UI_DROP, drop, FIELD_UI_DROP_HELICOPTER)?;
    let (peak, range) = peak_range(v, drop)?;
    let (key, source) = if own != 0 {
        (own, HeliSource::Drop)
    } else {
        let r = v.reference(CLASS_RANGE, range, FIELD_RANGE_HELICOPTER)?;
        anyhow::ensure!(r != 0, "neither the drop nor its range {range:08x} names a helicopter");
        (r, HeliSource::Range)
    };
    helicopter(v, key, source, peak, range)
}

fn helicopter(v: &Vaults, key: u32, source: HeliSource, peak: u32, range: u32) -> Result<HeliChoice> {
    let gameobject = v.reference(CLASS_HELICOPTER, key, FIELD_HELI_GAMEOBJECT)?;
    anyhow::ensure!(gameobject != 0, "helicopter {key:08x} has no game object");
    Ok(HeliChoice {
        key,
        source,
        peak,
        range,
        name: v.text(CLASS_HELICOPTER, key, FIELD_HELI_NAME)?.unwrap_or_default(),
        display: v.text(CLASS_HELICOPTER, key, FIELD_HELI_DISPLAY)?.unwrap_or_default(),
        gameobject,
    })
}

#[derive(Clone, Debug)]
pub struct HeliPart {
    pub mesh_name: String,
    pub mesh: ObjectMesh,
    pub skin: Option<Skin>,
    pub material: Material,
}

#[derive(Clone, Debug)]
pub struct HeliModel {
    pub lod_thresholds: Vec<f32>,
    pub rig: SceneGraph,
    pub bind: Vec<Srt>,
    pub parts: Vec<HeliPart>,
    pub problems: Vec<String>,
}

pub fn load_model(big_root: &Path, track: &str, key: u32) -> Result<HeliModel> {
    load_model_in(big_root, Some(track), key)
}

pub fn load_model_in(big_root: &Path, track: Option<&str>, key: u32) -> Result<HeliModel> {
    let (lod_thresholds, lod0) = with_level_db(big_root, track, |db| {
        let mut thresholds = Vec::new();
        let mut models = Vec::new();
        for f in db.fields(CLASS_GAMEOBJECT, key)? {
            match f.key {
                FIELD_LOD_THRESHOLDS => {
                    let vs: Vec<&Value> = match &f.data {
                        FieldData::Array(vs) => vs.iter().collect(),
                        FieldData::Single(v) => vec![v],
                    };
                    thresholds =
                        vs.into_iter().filter_map(|v| if let Value::Float(x) = v { Some(*x) } else { None }).collect();
                }
                FIELD_MODELS => {
                    for m in refs(&f.data).into_iter().filter(|r| r.collection != 0) {
                        models.push(part_refs(db, m.class, m.collection)?);
                    }
                }
                _ => {}
            }
        }
        Ok((thresholds, models.into_iter().next().with_context(|| format!("game object {key:08x} has no model"))?))
    })?;
    let mut problems = Vec::new();
    let mut loaded = Vec::new();
    let mut rig: Option<SceneGraph> = None;
    for p in lod0 {
        let read = || -> Result<(ObjectMesh, CrsfFile)> {
            let crsf_rel = p.crsf.as_deref().context("no FileRef (.crsf)")?;
            let mesh = crate::content::models::rider_mesh(big_root, &p.geom)?;
            let crsf = asset_file(big_root, crsf_rel).with_context(|| format!("{crsf_rel} not found"))?;
            let file = CrsfFile::parse(&std::fs::read(&crsf).with_context(|| format!("reading {}", crsf.display()))?)
                .with_context(|| format!("parsing {}", crsf.display()))?;
            Ok((mesh, file))
        };
        match read() {
            Ok((mesh, file)) => {
                if let Some(scene) = &file.scene {
                    let joints = |g: &SceneGraph| -> Vec<(String, Option<String>)> {
                        g.nodes
                            .iter()
                            .filter(|n| matches!(n.kind, ssx_formats::crsf::NodeKind::Joint(_)))
                            .map(|n| (n.name.clone(), n.parent.map(|q| g.nodes[q].name.clone())))
                            .collect()
                    };
                    match &rig {
                        None => rig = Some(scene.clone()),
                        Some(r) if joints(r) != joints(scene) => {
                            problems.push(format!("{}: its skeleton differs from the first part's", p.name));
                        }
                        Some(_) => {}
                    }
                }
                loaded.push((p, mesh, file));
            }
            Err(e) => problems.push(format!("{}: {e:#}", p.name)),
        }
    }
    let rig = rig.context("no helicopter part has a scene graph")?;
    let mut face_offsets = HashMap::new();
    for (_, _, file) in &loaded {
        for o in file.face_offsets.iter().flatten() {
            face_offsets.insert(o.joint.clone(), o.translate);
        }
    }
    let bind = bind_pose(&rig, &face_offsets);
    let mut parts = Vec::new();
    for (p, mesh, file) in loaded {
        let skin = if file.weights.is_some() {
            match skin_of(&file, &rig, mesh.positions.len()) {
                Ok(s) => Some(s),
                Err(e) => {
                    problems.push(format!("{}: {e:#}", p.name));
                    continue;
                }
            }
        } else {
            None
        };
        parts.push(HeliPart { mesh_name: p.name, mesh, skin, material: p.material });
    }
    if parts.is_empty() {
        bail!("game object {key:08x}: no part loaded ({})", problems.join("; "));
    }
    Ok(HeliModel { lod_thresholds, rig, bind, parts, problems })
}
