use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result, bail};
use ssx_content::data::simid::record_hash;
use ssx_content::data::{Class, SimContentId, SimRow};
use ssx_content::{ContentHash, ContentId, ContentIndex, Kind, Layer};

use super::super::store::GameContent;

const SERVER_FILES: &str = include_str!("../../../../../assets/server/server-files.tsv");

pub const RULES: &str = "rules";
pub const COLLISION: &str = "optional-collision";
pub const VERIFY: &str = "optional-verify";
pub const GHOSTS: &str = "optional-ghosts";

fn ui_drop_key(drop: &str) -> u32 {
    ssx_formats::attrib::fnv1(drop.as_bytes())
}

pub fn file_id(path: &str) -> String {
    format!("file:{path}")
}

fn tracks(rows: &[(&str, &str)]) -> BTreeSet<String> {
    rows.iter()
        .filter_map(|(p, _)| p.rsplit('/').next()?.strip_suffix("_track.vlt").map(str::to_string))
        .filter(|t| !t.is_empty())
        .collect()
}

fn track_of(path: &str, tracks: &BTreeSet<String>) -> Option<String> {
    path.split('/').find_map(|seg| {
        let stem = seg.split('.').next().unwrap_or(seg);
        let t = stem.strip_suffix("_track").unwrap_or(stem);
        tracks.contains(t).then(|| t.to_string())
    })
}

fn member_rows() -> Vec<(&'static str, &'static str)> {
    SERVER_FILES
        .lines()
        .skip(1)
        .filter_map(|l| {
            let mut f = l.split('\t');
            Some((f.next()?, f.next()?))
        })
        .collect()
}

pub fn member_index() -> Result<ContentIndex> {
    let mut index = ContentIndex::default();
    for (i, l) in SERVER_FILES.lines().enumerate().skip(1) {
        let f: Vec<&str> = l.split('\t').collect();
        let [path, _, size, hash] = f.as_slice() else { bail!("server-files.tsv:{}: four columns", i + 1) };
        let size = size.parse().with_context(|| format!("server-files.tsv:{}: the size", i + 1))?;
        let hash = hash.parse().map_err(|e| anyhow::anyhow!("server-files.tsv:{}: {e}", i + 1))?;
        index.entries.insert(path.to_string(), (size, hash));
    }
    Ok(index)
}

pub fn member_tracks() -> BTreeSet<String> {
    tracks(&member_rows())
}

pub fn session_files(track: &str, ghosts: &BTreeSet<u32>) -> Vec<String> {
    let rows = member_rows();
    let all = tracks(&rows);
    let mut out = Vec::new();
    for (path, class) in &rows {
        let keep = match *class {
            RULES | COLLISION | VERIFY => track_of(path, &all).is_none_or(|t| t == track),
            GHOSTS => {
                let stem =
                    path.rsplit('/').next().and_then(|n| n.strip_suffix(".bin")).and_then(|s| s.parse::<u32>().ok());
                stem.is_some_and(|g| ghosts.contains(&g))
            }
            _ => false,
        };
        if keep {
            out.push(path.to_string());
        }
    }
    out
}

pub struct SessionId {
    pub id: SimContentId,
    pub rows: Vec<SimRow>,
}

impl GameContent {
    pub fn sim_content_id(self: &std::sync::Arc<Self>, data: &Path, track: &str, drop: &str) -> Result<SessionId> {
        if !member_tracks().contains(track) {
            bail!("{}: no track of the member list", ssx_core::text::quoted(track));
        }
        if super::super::segment(drop).is_none() {
            bail!("{}: no ui_drop name", ssx_core::text::quoted(drop));
        }
        let index = crate::folder::content_index(data)?;
        self.sim_content_id_with(&index, track, drop)
    }

    pub fn sim_content_id_with(
        self: &std::sync::Arc<Self>,
        index: &ContentIndex,
        track: &str,
        drop: &str,
    ) -> Result<SessionId> {
        if !member_tracks().contains(track) {
            bail!("{}: no track of the member list", ssx_core::text::quoted(track));
        }
        if super::super::segment(drop).is_none() {
            bail!("{}: no ui_drop name", ssx_core::text::quoted(drop));
        }
        let mut rows = Vec::new();
        let ghosts = self.rival_ghosts(drop)?;
        let mut missing = Vec::new();
        for path in session_files(track, &ghosts) {
            match index.entries.get(&path) {
                Some((_, hash)) => rows.push(SimRow {
                    id: file_id(&path),
                    class: "file".into(),
                    hash: *hash,
                    layer: Layer::Base.source_text("ssx"),
                }),
                None => missing.push(path),
            }
        }
        if !missing.is_empty() {
            bail!("the data folder lacks {} of the session's files (first: {})", missing.len(), missing[0]);
        }
        rows.extend(self.record_rows(Some(drop))?);
        rows.extend(self.level_rows(track, drop)?);
        let id = SimContentId::of(&rows).map_err(anyhow::Error::msg)?;
        Ok(SessionId { id, rows })
    }

    fn rival_ghosts(&self, drop: &str) -> Result<BTreeSet<u32>> {
        let text = super::events::ghost_db_text(self.big_root()).context("reading ghostdb.csv")?;
        let key = ui_drop_key(drop);
        let rows = ssx_formats::csvdb::parse_ghosts(&text)?;
        Ok(rows.iter().filter(|r| r.ui_drop == key).filter_map(|r| u32::try_from(r.ghost_id).ok()).collect())
    }

    pub(crate) fn record_rows(&self, drop: Option<&str>) -> Result<Vec<SimRow>> {
        let mut rows = Vec::new();
        if super::tuning::KINDS.iter().any(|&k| self.packages_provide(k))
            && let Some(disc) = self.disc_tuning()
        {
            for c in self.tuning_candidates().iter() {
                let class = super::tuning::class_of(&c.object.type_desc(&disc.stream.schema).name);
                if class != Class::Simulation {
                    continue;
                }
                let table =
                    ssx_content::data::ant::to_table(&disc.stream.schema, &c.object).map_err(anyhow::Error::msg)?.0;
                rows.push(SimRow {
                    id: c.id.to_string(),
                    class: "simulation".into(),
                    hash: record_hash(&table),
                    layer: self.layer_of(&c.id),
                });
            }
        }
        if let Some(drop) = drop
            && super::events::KINDS.iter().any(|&k| self.packages_provide(k))
            && let Some(disc) = self.disc_events()
        {
            let routed = self.routed_events();
            let key = ui_drop_key(drop);
            if let Some((d, e)) = routed.tables.get(&key) {
                for (kind, table) in [(Kind::Drop, d), (Kind::Event, e)] {
                    let Some(t) = table else { continue };
                    let path = super::super::segment(drop).with_context(|| format!("{drop:?}: no id path"))?;
                    let id =
                        ContentId::new(ssx_content::BASE_NAMESPACE, kind, &path).map_err(|e| anyhow::anyhow!("{e}"))?;
                    let Some(schema) = disc.schema_of(&id) else { continue };
                    let sim: toml::Table = t
                        .iter()
                        .filter(|(k, _)| schema.fields.get(*k).is_some_and(|f| f.class == Class::Simulation))
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect();
                    let base: toml::Table = disc
                        .record(&id)
                        .map_err(anyhow::Error::msg)?
                        .into_iter()
                        .filter(|(k, _)| schema.fields.get(k).is_some_and(|f| f.class == Class::Simulation))
                        .collect();
                    if sim != base {
                        let layer = self.layer_of(&id);
                        rows.push(SimRow {
                            id: id.to_string(),
                            class: "simulation".into(),
                            hash: record_hash(&sim),
                            layer,
                        });
                    }
                }
            }
        }
        rows.extend(self.rider_stat_rows()?);
        if self.packages_provide(Kind::Anim) {
            rows.extend(self.anim_rows());
        }
        if self.packages_provide(Kind::Trick) {
            rows.extend(self.trick_rows());
        }
        Ok(rows)
    }

    fn level_rows(self: &std::sync::Arc<Self>, track: &str, drop: &str) -> Result<Vec<SimRow>> {
        let mut rows = Vec::new();
        if self.packages_provide(Kind::Lighting)
            && let Some(ge) = super::events::ge_drop_of(self.big_root(), drop)?
        {
            let routed = crate::track_lighting::load_track_drops(self.big_root(), track)?;
            let disc = crate::track_lighting::disc_track_drops(self.big_root(), track)?;
            let sim = |d: &crate::track_lighting::Drop| -> toml::Table {
                let mut t = toml::Table::new();
                for k in super::LIGHTING_SIM_FIELDS {
                    if let Some(v) = d.lighting.vector(k) {
                        let a = v.iter().map(|x| toml::Value::Float(ssx_content::data::ant::float(*x))).collect();
                        t.insert(k.to_string(), toml::Value::Array(a));
                    } else if let Some(v) = d.lighting.float(k) {
                        t.insert(k.to_string(), toml::Value::Float(ssx_content::data::ant::float(v)));
                    }
                }
                t
            };
            let pick = |list: &[crate::track_lighting::Drop]| list.iter().find(|d| d.key == ge).map(sim);
            let (now, was) = (pick(&routed), pick(&disc));
            if now != was {
                let key = routed.iter().find(|d| d.key == ge).and_then(|d| d.lighting_key).unwrap_or(0);
                let named = self.disc_records().and_then(|r| r.names().ok()).and_then(|n| n.lighting(key).cloned());
                let layer = named.as_ref().map_or_else(|| Layer::Base.source_text("ssx"), |id| self.layer_of(id));
                let id = named.map_or_else(|| format!("lighting:0x{key:08x}"), |id| id.to_string());
                let t = now.unwrap_or_default();
                rows.push(SimRow { id, class: "simulation".into(), hash: record_hash(&t), layer });
            }
        }
        if let Some((hash, layer)) = self.track_packages(track).sim_input() {
            rows.push(SimRow { id: format!("track:{track}"), class: "collision".into(), hash, layer });
        }
        Ok(rows)
    }

    fn rider_stat_rows(&self) -> Result<Vec<SimRow>> {
        let kinds = super::super::characters::RIDER_KINDS;
        if !kinds.iter().any(|&k| self.packages_provide(k)) {
            return Ok(Vec::new());
        }
        let mut rows = Vec::new();
        for id in self.changed_ids(&kinds) {
            let field = if id.kind() == Kind::Character { "stat_bonuses" } else { "attributes" };
            let stats = |t: &toml::Table| match id.kind() {
                Kind::Character => t.get("career").and_then(toml::Value::as_table).and_then(|c| c.get(field)).cloned(),
                _ => t.get(field).cloned(),
            };
            let disc = self.with_registry(|r| {
                r.providers(&id).find(|(m, _)| m.layer == Layer::Base).and_then(|(_, s)| s.load(&id).ok())
            });
            let disc_value = disc.as_ref().and_then(|a| a.record()).and_then(stats);
            let base = disc.as_ref().and_then(|a| a.record()).cloned().unwrap_or_default();
            let Ok((t, _, _)) = self.sim_record(&id, base) else { continue };
            let value = stats(&t);
            if value != disc_value {
                let mut one = toml::Table::new();
                if let Some(v) = value {
                    one.insert(field.into(), v);
                }
                rows.push(SimRow {
                    id: id.to_string(),
                    class: field.into(),
                    hash: record_hash(&one),
                    layer: self.layer_of(&id),
                });
            }
        }
        Ok(rows)
    }
}

impl GameContent {
    pub(crate) fn layer_of(&self, id: &ContentId) -> String {
        let below = self.sim_policy() != ssx_content::policy::ClientMods::Any;
        let keep = |l: Layer| !(below && l == Layer::ClientMods);
        self.with_registry(|r| {
            let winner = r.providers(id).find(|(m, _)| keep(m.layer)).map(|(m, _)| m.layer.source_text(m.name));
            let patches: Vec<String> = r
                .patches_in(id, keep)
                .map(|p| p.into_iter().map(|(n, l, _)| l.source_text(&n)).collect())
                .unwrap_or_default();
            let mut out = winner.unwrap_or_else(|| "none".into());
            if !patches.is_empty() {
                out.push_str(&format!(", patched by {}", patches.join(", ")));
            }
            ssx_core::text::shown_max(&out, ssx_core::text::MESSAGE_CHARS).to_string()
        })
    }
}

pub fn rows_text(rows: &[SimRow]) -> String {
    let mut sorted: Vec<&SimRow> = rows.iter().collect();
    sorted.sort_by(|a, b| (&a.id, &a.class).cmp(&(&b.id, &b.class)));
    let mut out = String::new();
    for r in sorted {
        out.push_str(&format!("{}\t{}\t{}\t{}\n", r.id, r.class, r.hash, r.layer));
    }
    out
}

pub fn file_hashes(rows: &[SimRow]) -> BTreeMap<&str, ContentHash> {
    rows.iter().filter(|r| r.class == "file").map(|r| (r.id.as_str(), r.hash)).collect()
}

pub fn result_sim_id(big_root: &Path, track: &str, drop: &str) -> Option<String> {
    let content = GameContent::of(big_root);
    if !content.sim_changed() {
        return None;
    }
    let data = big_root.parent().unwrap_or(big_root);
    match content.sim_content_id(data, track, drop) {
        Ok(s) if s.rows.iter().any(|r| r.class != "file") => {
            content.info(format!("content: the session's sim content id {} ({} rows)", s.id, s.rows.len()));
            Some(s.id.to_string())
        }
        Ok(_) => None,
        Err(e) => {
            content.notice(format!("content: the session's sim content id: {e:#}"));
            Some("unknown".into())
        }
    }
}

pub fn best_not_kept(big_root: &Path, event: &str, why: crate::career::profile::BestNotKept) {
    use crate::career::profile::BestNotKept;
    let reason = match why {
        BestNotKept::NoEventId => "the event has no content id to record its sim content id under",
        BestNotKept::ModStateFull => "the save's mod state is at its caps (no room for its sim content id)",
    };
    let event = ssx_core::text::quoted(event);
    GameContent::of(big_root).notice(format!(
        "content: a better result on {event} ridden with simulation mods is not kept as the best: {reason}"
    ));
}

pub fn run_sim_id(
    data: &Path,
    track: &str,
    drop: &str,
    policy: ssx_content::policy::ClientMods,
    mods: bool,
    rows: bool,
) -> Result<()> {
    let big = crate::setup::big_root(data);
    let content = GameContent::of(&big);
    if mods {
        let report = content.mount_client_mods(&crate::setup::app_dir());
        if let Some(e) = &report.list_error {
            bail!("the mods list: {e}");
        }
        for m in &report.mounted {
            ssx_core::eprintln!("mounted {}", ssx_core::text::shown(&m.name));
        }
    }
    content.set_sim_policy(policy);
    let tracks: Vec<String> = if track == "all" {
        member_tracks().into_iter().collect()
    } else if member_tracks().contains(track) {
        vec![track.to_string()]
    } else {
        bail!("{}: no track of the member list (`all`: every track)", ssx_core::text::quoted(track));
    };
    let ge_drops = super::events::ge_drops(&big)?;
    let mut found = 0;
    for track in &tracks {
        let keys: BTreeSet<u32> = crate::track_lighting::disc_track_drops(&big, track)?.iter().map(|d| d.key).collect();
        let names: Vec<&str> = if drop == "all" {
            ge_drops.iter().filter(|(_, g)| keys.contains(g)).map(|(n, _)| n.as_str()).collect()
        } else {
            let ge = ge_drops.iter().find(|(n, _)| n == drop).map(|(_, g)| *g);
            let ge = ge.with_context(|| format!("{drop:?}: no ui_drop of that name"))?;
            if !keys.contains(&ge) {
                bail!("{drop:?} is not ridden on {track:?} (its ge_drop 0x{ge:08x} is not one of the track's drops)");
            }
            vec![drop]
        };
        for name in names {
            let session = content.sim_content_id(data, track, name)?;
            let (policy, n) = (format!("policy {policy:?}"), format!("{} rows", session.rows.len()));
            ssx_core::print::row(&[&session.id, &track, &name, &policy, &n]);
            if rows {
                print!("{}", rows_text(&session.rows));
            }
            found += 1;
        }
    }
    ssx_core::eprintln!("{found} drops");
    for n in content.take_notices() {
        ssx_core::eprintln!("{}", n.text);
    }
    Ok(())
}
