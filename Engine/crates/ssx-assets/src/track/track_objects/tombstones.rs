use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use ssx_formats::attrib::{FieldData, Value};
use ssx_lua::game::controllers::StateSets;
use ssx_lua::game::controllers::tracks::{Switch, Target, Track};

use crate::particles::{CLASS_PARTICLE_NODE, ParticleNode, particle_node};
use crate::vault_db::{fnv1, refs, single, with_track_db};

const SCRIPTS_DIR: &str = "memresident/data/common/scripts";
const CONTROLLERS_SCRIPT: &str = "world_entity_controllers_default.lua";
pub const SCHEMA: &str = "Tombstone";
const CLASS_GAMEOBJECT: u32 = fnv1("ge_scene_gameobject");
const FIELD_MODELS: u32 = 0xB43A_3598;
const FIELD_PARTICLE_NODES: u32 = fnv1("ParticleNodes");
const FIELD_MESHES: u32 = fnv1("Meshes");
const FIELD_LOD_THRESHOLDS: u32 = fnv1("LODThresholds");

#[derive(Clone, Debug)]
pub struct TombstoneAssets {
    pub game_object: String,
    pub lod_thresholds: Vec<f32>,
    pub node: ParticleNode,
    pub has_mesh: bool,
    pub state: String,
    pub on_start: Switch,
    pub on_end: Switch,
}

pub fn load(big_root: &Path, track: &str) -> Result<TombstoneAssets> {
    let (schemas, _) = crate::avalanche_entities::batches(big_root, &[]).context("the schemas batch")?;
    let schema = schemas.schemas.get(SCHEMA).with_context(|| format!("no schema {SCHEMA}"))?;
    anyhow::ensure!(schema.controller.as_deref() == Some(SCHEMA), "{SCHEMA}: controller {:?}", schema.controller);
    let game_object = schema
        .factories
        .iter()
        .filter(|f| f.kind == "GameObject")
        .flat_map(|f| &f.calls)
        .find(|(m, _)| m == "SetGameObjectName")
        .and_then(|(_, a)| a.first().and_then(|a| a.str()).map(str::to_string))
        .context("Tombstone: no game object")?;
    let path = big_root.join(SCRIPTS_DIR).join(CONTROLLERS_SCRIPT);
    let src = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let graph = ssx_lua::game::graph::run_batch(
        ssx_lua::game::Binder::StateSets,
        &[(CONTROLLERS_SCRIPT.to_string(), src)],
        None,
    );
    let sets = Arc::new(StateSets::from_graph(&graph));
    let set = sets.set(SCHEMA).context("no Tombstone state set")?;
    let first = sets.sets[set].states.first().context("Tombstone: no state")?;
    let tracks: Vec<&Track> = first.update.iter().chain(&first.enter).filter_map(|&k| sets.kinds.get(k)).collect();
    let (on_start, on_end) = match tracks.as_slice() {
        [Track::Particle { target: Target::Node(n), on_start, on_end }] if n.is_empty() => (*on_start, *on_end),
        other => bail!("Tombstone's state: tracks {other:?}"),
    };
    let state = sets.sets[set].states[0].name.text.to_string();
    let (node, lod_thresholds, has_mesh) = with_track_db(big_root, track, |db| {
        let key = fnv1(&game_object);
        let field = |k: u32| db.field(CLASS_GAMEOBJECT, key, k);
        let lod_thresholds = match field(FIELD_LOD_THRESHOLDS)?.map(|f| &f.data) {
            Some(FieldData::Array(vs)) => {
                vs.iter().filter_map(|v| if let Value::Float(x) = v { Some(*x) } else { None }).collect()
            }
            Some(d) => {
                single(d).and_then(|v| if let Value::Float(x) = v { Some(vec![*x]) } else { None }).unwrap_or_default()
            }
            None => Vec::new(),
        };
        let models = field(FIELD_MODELS)?.context("no models")?;
        let model = refs(&models.data).into_iter().find(|r| r.collection != 0).context("no model")?;
        let nodes = db.field(model.class, model.collection, FIELD_PARTICLE_NODES)?.context("no particle node")?;
        let node = refs(&nodes.data)
            .into_iter()
            .find(|r| r.collection != 0 && r.class == CLASS_PARTICLE_NODE)
            .context("no particle node")?;
        let has_mesh =
            db.field(model.class, model.collection, FIELD_MESHES)?.is_some_and(|m| !refs(&m.data).is_empty());
        Ok((particle_node(db, node.collection)?, lod_thresholds, has_mesh))
    })?;
    Ok(TombstoneAssets { game_object, lod_thresholds, node, has_mesh, state, on_start, on_end })
}
