use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use ssx_content::map::{Map, PropLook};
use ssx_content::model::Collision;
use ssx_content::object::{ObjectCollision, ObjectDef};
use ssx_content::{API_LEVEL, BASE_NAMESPACE, ContentHash, ContentId, Kind, Layer, LoadError, PackageSource, Version};

use super::resolve::{Found, ModelUse};
use super::store::GameContent;
use crate::track_lighting::LightingPreset;
use crate::track_objects::AudioName;

#[derive(Clone, Debug)]
pub struct MapObject {
    pub id: ContentId,
    pub source: String,
    pub layer: Layer,
    pub collision: Option<Arc<Collision>>,
    pub pob: Option<String>,
    pub model: Option<(ContentId, Option<ContentHash>)>,
    pub audio: Option<AudioName>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutBase {
    pub drop: ContentId,
    pub ui_drop: String,
    pub track: String,
    pub ge_drop: u32,
    pub index: usize,
}

impl GameContent {
    pub fn map(&self, id: &ContentId) -> Result<Found<Map>, LoadError> {
        if id.kind() != Kind::Map {
            return Err(LoadError::Failed { id: id.clone(), message: "not a map id".into() });
        }
        self.strongest_in(id, self.session_skip(), |_: &Map| None)
    }

    pub(in crate::content) fn session_skip(&self) -> impl Fn(Layer) -> Option<&'static str> + use<> {
        let in_session = self.session_policy().is_some();
        move |layer| {
            (in_session && layer == Layer::ClientMods)
                .then_some("a client mod's, and a network session rides the server's maps and gamemodes only")
        }
    }

    pub fn map_object(&self, id: &ContentId) -> Result<MapObject, LoadError> {
        let found = self.map_object_def(id)?;
        let o = &found.value;
        let (collision, pob, model) = match &o.collision {
            ObjectCollision::None => (None, None, None),
            ObjectCollision::Decoded { path, collision } => (Some(collision.clone()), Some(path.clone()), None),
            ObjectCollision::Model(m) => {
                let model = self.model(m, ModelUse::Static)?;
                let hash = model.value.source;
                (model.value.collision.clone(), None, Some((m.clone(), hash)))
            }
        };
        Ok(MapObject {
            id: id.clone(),
            source: found.source.clone(),
            layer: found.layer,
            collision,
            pob,
            model,
            audio: o.audio.clone(),
        })
    }

    pub fn map_object_def(&self, id: &ContentId) -> Result<Found<ObjectDef>, LoadError> {
        if id.kind() != Kind::Object {
            return Err(LoadError::Failed { id: id.clone(), message: "not an object id".into() });
        }
        let fits = |o: &ObjectDef| match (&o.controller, &o.rigid_body) {
            (Some(c), _) => Some(format!(
                "its controller script {c} makes it an entity with a controller, which a map does not place yet (M8c)"
            )),
            (None, Some(_)) => Some("a rigid body (a controlled entity's) is not placed by a map yet (M8c)".into()),
            (None, None) => None,
        };
        self.strongest(id, fits)
    }

    pub fn map_objects(&self, map: &Map) -> Result<BTreeMap<ContentId, MapObject>, String> {
        let mut out = BTreeMap::new();
        for p in &map.definition.props {
            let PropLook::Object(id) = &p.look else { continue };
            if out.contains_key(id) {
                continue;
            }
            match self.map_object(id) {
                Ok(o) => {
                    out.insert(id.clone(), o);
                }
                Err(e) => return Err(format!("{}: node \"{}\" ava_object: {e}", map.toml.scene, p.node)),
            }
        }
        for m in &map.definition.markers {
            if let Err(e) = self.marker(&m.marker) {
                return Err(format!("{}: node \"{}\" ava_marker: {e}", map.toml.scene, m.node));
            }
        }
        Ok(out)
    }

    pub fn map_lighting(&self, id: &ContentId) -> Result<LightingPreset> {
        if id.kind() != Kind::Lighting || id.namespace() != BASE_NAMESPACE {
            bail!(
                "{id}: a map's lighting is a preset of the disc by id (a package changes it by its override or patch)"
            );
        }
        let records = self.disc_records().context("the disc's records are not indexed")?;
        let names = records.names().map_err(anyhow::Error::msg)?;
        let &(_, key) = names.by_id.get(id).with_context(|| format!("{id}: no lighting preset of the disc"))?;
        let disc = crate::vault_db::with_attribdb(self.big_root(), |db| crate::track_lighting::preset(db, key, None))?;
        Ok(self.route_lighting(key, disc))
    }

    pub fn layout_base(&self, id: &ContentId) -> Result<LayoutBase> {
        if id.kind() != Kind::Drop || id.namespace() != BASE_NAMESPACE {
            bail!("{id}: a layout sits on a drop of the disc (ssx:drop/<ui_drop name>)");
        }
        let ui_drop = id.path().to_string();
        let ge = super::data::events::ge_drop_of(self.big_root(), &ui_drop)?
            .filter(|&k| k != 0)
            .with_context(|| format!("{id}: no ui_drop of that name with a ge_drop"))?;
        for track in super::data::simid::member_tracks() {
            let starts = ssx_rules::locators::load_drop_starts(self.big_root(), &track).unwrap_or_default();
            if let Some(index) = starts.iter().position(|d| d.drop_key == ge) {
                return Ok(LayoutBase { drop: id.clone(), ui_drop, track, ge_drop: ge, index });
            }
        }
        bail!("{id}: its ge_drop 0x{ge:08x} has no start locator on any track")
    }

    pub fn mount_folder(&self, dir: &Path, mounted: &[(String, Version)]) -> Result<(String, Version, Vec<ContentId>)> {
        let source = PackageSource::open(dir).map_err(|e| anyhow!("{}: {e}", dir.display()))?;
        if let Some(why) = super::mods::package_rules(&source) {
            bail!("{}: {why}", dir.display());
        }
        let m = &source.manifest;
        if m.resource.api > API_LEVEL {
            bail!("{}: needs API level {}; this game offers {API_LEVEL}", m.resource.name, m.resource.api);
        }
        for dep in &m.dependencies.require {
            if !mounted.iter().any(|(name, v)| *name == dep.name && dep.req.matches(*v)) {
                bail!("{}: requires {} {}, which is not mounted (--with it first)", m.resource.name, dep.name, dep.req);
            }
        }
        let (name, version) = (m.resource.name.clone(), m.resource.version);
        let maps = m.add.keys().chain(m.overrides.keys()).filter(|id| id.kind() == Kind::Map).cloned().collect();
        self.swap_dev_package(None, source, Default::default());
        Ok((name, version, maps))
    }

    pub(in crate::content) fn strongest<T: Send + Sync + 'static>(
        &self,
        id: &ContentId,
        fits: impl Fn(&T) -> Option<String>,
    ) -> Result<Found<T>, LoadError> {
        self.strongest_in(id, |_| None, fits)
    }

    pub(in crate::content) fn strongest_in<T: Send + Sync + 'static>(
        &self,
        id: &ContentId,
        skip: impl Fn(Layer) -> Option<&'static str>,
        fits: impl Fn(&T) -> Option<String>,
    ) -> Result<Found<T>, LoadError> {
        let mut last = LoadError::Missing(id.clone());
        let mut i = 0;
        loop {
            let loaded = {
                let registry = self.registry.read().unwrap_or_else(std::sync::PoisonError::into_inner);
                let mut providers = registry.providers(id).skip(i);
                let Some((info, source)) = providers.next() else { break };
                let next = providers.next().map(|(n, _)| n.name.to_string());
                let asset = match skip(info.layer) {
                    Some(why) => {
                        Err(LoadError::Failed { id: id.clone(), message: format!("from {}: {why}", info.name) })
                    }
                    None => self.cached_load(id, info.name, info.layer, source),
                };
                (info.name.to_string(), info.layer, next, asset)
            };
            i += 1;
            let (source, layer, next, asset) = loaded;
            let value = asset.and_then(|a| {
                a.into_value::<T>().ok_or_else(|| LoadError::Failed {
                    id: id.clone(),
                    message: format!("not a decoded {}", id.kind().as_str()),
                })
            });
            match value {
                Ok(v) => match fits(&v) {
                    None => return Ok(Found { id: id.clone(), value: v, source, layer }),
                    Some(why) => {
                        self.passed_over(id, &source, layer, &why, next.as_deref());
                        last = LoadError::Failed { id: id.clone(), message: why };
                    }
                },
                Err(e) => {
                    self.passed_over(id, &source, layer, &e.to_string(), next.as_deref());
                    last = e;
                }
            }
        }
        Err(last)
    }
}

mod drawn;
mod simid;

pub use drawn::{ListedMap, MAX_LISTED_MAPS, TERRAIN_EFFECT};

#[cfg(test)]
mod tests;
