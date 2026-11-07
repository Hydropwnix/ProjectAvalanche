use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context, Result};
use ssx_content::data::simid::record_hash;
use ssx_content::data::{SimContentId, SimRow};
use ssx_content::map::Map;
use ssx_content::{ContentIndex, Layer};

use super::super::data::simid::{SessionId, file_id, session_files};
use super::super::resolve::Found;
use super::super::store::GameContent;
use super::{LayoutBase, MapObject};

impl GameContent {
    pub fn map_sim_content_id(
        self: &std::sync::Arc<Self>,
        data: &Path,
        map: &Found<Map>,
        base: Option<&LayoutBase>,
        objects: &[MapObject],
        mode: Option<&super::super::gamemodes::ResolvedMode>,
    ) -> Result<SessionId> {
        let text = std::fs::read_to_string(data.join(super::super::hash::INDEX_FILE))
            .with_context(|| format!("{}: the data folder's content index", data.display()))?;
        let index = ContentIndex::from_tsv(&text).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        self.map_sim_content_id_with(&index, map, base, objects, mode)
    }

    pub fn map_sim_content_id_with(
        self: &std::sync::Arc<Self>,
        index: &ContentIndex,
        map: &Found<Map>,
        base: Option<&LayoutBase>,
        objects: &[MapObject],
        mode: Option<&super::super::gamemodes::ResolvedMode>,
    ) -> Result<SessionId> {
        let file_row = |path: &str| -> Result<SimRow> {
            let (_, hash) = index.entries.get(path).with_context(|| format!("the data folder lacks {path}"))?;
            Ok(SimRow { id: file_id(path), class: "file".into(), hash: *hash, layer: "L0".into() })
        };
        let mut rows = match base {
            Some(b) => self.sim_content_id_with(index, &b.track, &b.ui_drop)?.rows,
            None => {
                let mut rows = Vec::new();
                for path in session_files("", &BTreeSet::new()) {
                    rows.push(file_row(&path)?);
                }
                rows.extend(self.record_rows(None)?);
                rows
            }
        };
        rows.push(SimRow {
            id: map.id.to_string(),
            class: "simulation".into(),
            hash: ssx_content::map::sim_hash(&map.value),
            layer: self.layer_of(&map.id),
        });
        if let Some((g, rules)) = mode.and_then(|m| Some((m.gamemode.as_ref()?, &m.rules))) {
            rows.push(SimRow {
                id: g.id.to_string(),
                class: "simulation".into(),
                hash: ssx_content::gamemode::sim_hash(rules),
                layer: self.layer_of(&g.id),
            });
        }
        for o in objects {
            match (&o.pob, &o.model) {
                (Some(pob), _) => {
                    let file = crate::track_objects::asset_file(self.big_root(), pob)
                        .with_context(|| format!("{}: its collision {pob} is not in the data folder", o.id))?;
                    let rel = data_member(self.big_root(), &file)
                        .with_context(|| format!("{}: outside the data folder", o.id))?;
                    rows.push(file_row(&rel)?);
                }
                (None, model) => {
                    let mut t = toml::Table::new();
                    t.insert("source".into(), toml::Value::String(o.source.clone()));
                    if let Some((m, hash)) = model {
                        t.insert("collision".into(), toml::Value::String(m.to_string()));
                        let h = hash.map_or_else(String::new, |h| h.to_string());
                        t.insert("collision_hash".into(), toml::Value::String(h));
                    }
                    let layer = if o.layer == Layer::Base { "L0".to_string() } else { self.layer_of(&o.id) };
                    rows.push(SimRow { id: o.id.to_string(), class: "collision".into(), hash: record_hash(&t), layer });
                }
            }
        }
        if let Some(id) = &map.value.toml.lighting
            && self.packages_provide(ssx_content::Kind::Lighting)
        {
            let routed = self.map_lighting(id)?;
            let names = self.disc_records().context("the disc's records")?.names().map_err(anyhow::Error::msg)?;
            let &(_, key) = names.by_id.get(id).with_context(|| format!("{id}: no lighting preset of the disc"))?;
            let disc =
                crate::vault_db::with_attribdb(self.big_root(), |db| crate::track_lighting::preset(db, key, None))?;
            let (now, was) = (lighting_sim(&routed), lighting_sim(&disc));
            if now != was {
                rows.push(SimRow {
                    id: id.to_string(),
                    class: "simulation".into(),
                    hash: record_hash(&now),
                    layer: self.layer_of(id),
                });
            }
        }
        let id = SimContentId::of(&rows).map_err(anyhow::Error::msg)?;
        Ok(SessionId { id, rows })
    }
}

fn data_member(big_root: &Path, file: &Path) -> Option<String> {
    let names = file.strip_prefix(big_root).ok()?.components().map(|c| match c {
        std::path::Component::Normal(name) => name.to_str(),
        _ => None,
    });
    let names = names.collect::<Option<Vec<&str>>>()?;
    (!names.is_empty()).then(|| format!("big/{}", names.join("/")))
}

fn lighting_sim(p: &crate::track_lighting::LightingPreset) -> toml::Table {
    let mut t = toml::Table::new();
    for k in super::super::data::LIGHTING_SIM_FIELDS {
        if let Some(v) = p.vector(k) {
            let a = v.iter().map(|x| toml::Value::Float(ssx_content::data::ant::float(*x))).collect();
            t.insert(k.to_string(), toml::Value::Array(a));
        } else if let Some(v) = p.float(k) {
            t.insert(k.to_string(), toml::Value::Float(ssx_content::data::ant::float(v)));
        }
    }
    t
}

#[cfg(test)]
mod data_member_tests {
    use std::path::Path;

    use super::data_member;

    #[test]
    fn a_disc_files_row_is_named_below_the_content_folder() {
        let big = Path::new("/srv/extracted/big");
        let pob = big.join("data0/data/xenon/gameexplorer/worlds/geometry/rock_boulder_a.pob");
        let named = data_member(big, &pob);
        assert_eq!(named.as_deref(), Some("big/data0/data/xenon/gameexplorer/worlds/geometry/rock_boulder_a.pob"));
        assert_eq!(data_member(Path::new("big"), Path::new("big/data0/a.pob")).as_deref(), Some("big/data0/a.pob"));
        assert_eq!(data_member(big, Path::new("/elsewhere/data0/a.pob")), None);
        assert_eq!(data_member(big, &big.join("data0/../../a.pob")), None);
        assert_eq!(data_member(big, big), None);
    }
}
