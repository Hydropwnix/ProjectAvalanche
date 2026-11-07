use anyhow::{Context, Result, bail};
use ssx_content::map::{Map, MapToml, Mode};
use ssx_content::{BASE_NAMESPACE, ContentId, Kind, Layer};

use super::super::store::GameContent;
use crate::landscape::terrain_material::TerrainMaterial;
use crate::track_objects::TrackMidground;

pub const TERRAIN_EFFECT: &str = "terrain_default.fx";
pub const MAX_LISTED_MAPS: usize = 256;

#[derive(Clone, Debug, PartialEq)]
pub struct ListedMap {
    pub id: ContentId,
    pub package: String,
    pub layer: Layer,
    pub toml: Result<MapToml, String>,
}

impl ListedMap {
    pub fn title(&self) -> Option<&str> {
        self.toml.as_ref().ok().map(|t| t.title.as_str())
    }

    pub fn modes(&self) -> &[Mode] {
        self.toml.as_ref().map(|t| t.modes.as_slice()).unwrap_or_default()
    }

    pub fn base(&self) -> Option<&ContentId> {
        self.toml.as_ref().ok().and_then(|t| t.base.as_ref())
    }
}

impl GameContent {
    pub fn map_terrain_material(&self, id: &ContentId) -> Result<TerrainMaterial> {
        if id.kind() != Kind::Material || id.namespace() != BASE_NAMESPACE {
            bail!("{id}: a map's terrain is drawn with a terrain material of the disc (ssx:material/<name>)");
        }
        let records = self.disc_records().context("the disc's records are not indexed")?;
        let names = records.names().map_err(anyhow::Error::msg)?;
        let &(class, key) = names.by_id.get(id).with_context(|| format!("{id}: no material of the disc"))?;
        let m = crate::vault_db::with_attribdb(self.big_root(), |db| db.material(class, key))?;
        if m.effect.as_deref() != Some(TERRAIN_EFFECT) {
            bail!(
                "{id}: its effect is {}, not the terrain family's {TERRAIN_EFFECT}",
                m.effect.as_deref().unwrap_or("none")
            );
        }
        if m.textures.is_empty() {
            bail!("{id}: a terrain material without textures");
        }
        Ok(m)
    }

    pub fn disc_game_object(&self, id: &ContentId) -> Result<crate::track_objects::GameObject> {
        if id.kind() != Kind::Object || id.namespace() != BASE_NAMESPACE {
            bail!("{id}: not an object of the disc");
        }
        let records = self.disc_records().context("the disc's records are not indexed")?;
        let names = records.names().map_err(anyhow::Error::msg)?;
        let &(class, key) = names.by_id.get(id).with_context(|| format!("{id}: no object of the disc"))?;
        crate::vault_db::with_attribdb(self.big_root(), |db| crate::track_objects::game_object(db, class, key))
    }

    pub fn map_midground(&self, id: &ContentId) -> Result<TrackMidground> {
        if id.kind() != Kind::Track || id.namespace() != BASE_NAMESPACE {
            bail!("{id}: a map shows the mid-ground of a disc track (ssx:track/<name>)");
        }
        if !self.has(id) {
            bail!("{id}: no track of the disc");
        }
        crate::track_objects::load_track_midground(self.big_root(), id.path())?
            .with_context(|| format!("{id}: its ge_track names no mid-ground object"))
    }

    pub fn paired_postfx(&self, lighting: &ContentId) -> Result<Option<String>> {
        let records = self.disc_records().context("the disc's records are not indexed")?;
        let names = records.names().map_err(anyhow::Error::msg)?;
        let &(_, key) =
            names.by_id.get(lighting).with_context(|| format!("{lighting}: no lighting preset of the disc"))?;
        for track in super::super::modtool::tracks::tracks(self.big_root()) {
            let drops = crate::track_lighting::load_track_drops(self.big_root(), &track)?;
            if let Some(d) = drops.iter().find(|d| d.lighting_key == Some(key)) {
                return Ok(d.postfx.clone());
            }
        }
        Ok(None)
    }

    pub fn map_toml(&self, id: &ContentId) -> Option<Result<MapToml, String>> {
        self.with_registry(|r| {
            let (info, source) = r.providers(id).next()?;
            let held = self.held_map(id, info.name).and_then(|a| a.into_value::<Map>()).map(|m| m.toml.clone());
            Some(match held.map(Ok).or_else(|| source.map_record(id)) {
                Some(Ok(t)) => Ok(t),
                Some(Err(e)) => Err(e.to_string()),
                None => Err(format!("{id}: its provider {} holds no map.toml", info.name)),
            })
        })
    }

    pub fn listed_maps(&self) -> (Vec<ListedMap>, usize) {
        self.with_registry(|r| {
            let ids = r.list_above(Layer::Base, Kind::Map);
            let more = ids.len().saturating_sub(MAX_LISTED_MAPS);
            let mut out = Vec::new();
            for id in ids.into_iter().take(MAX_LISTED_MAPS) {
                let Some((info, source)) = r.providers(&id).next() else { continue };
                let held = self.held_map(&id, info.name).and_then(|a| a.into_value::<Map>()).map(|m| m.toml.clone());
                let toml = match held.map(Ok).or_else(|| source.map_record(&id)) {
                    Some(Ok(t)) => Ok(t),
                    Some(Err(e)) => Err(e.to_string()),
                    None => Err(format!("{id}: its provider {} holds no map.toml", info.name)),
                };
                out.push(ListedMap { id, package: info.name.to_string(), layer: info.layer, toml });
            }
            (out, more)
        })
    }
}
