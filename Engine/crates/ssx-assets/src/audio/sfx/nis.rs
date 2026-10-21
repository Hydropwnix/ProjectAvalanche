use std::path::Path;

use anyhow::{Context, Result};

use crate::anim_db::{Asset, AssetDb};

pub const ANT_TAGS_SYSTEM: i64 = 4139;

#[derive(Clone, Debug, PartialEq)]
pub struct NisAudioTag {
    pub role: i64,
    pub time: f32,
    pub duration: f32,
    pub system: i64,
    pub event: i64,
    pub name: String,
}

fn num(a: &Asset, field: &str) -> f32 {
    a.f32(field).or_else(|| a.int(field).map(|v| v as f32)).unwrap_or(0.0)
}

pub fn nis_audio_tags(big_root: &Path, bundles: &[String], sequence: &str) -> Result<Vec<NisAudioTag>> {
    let dir = big_root.join(crate::rig_anim::ANIM_DIR);
    let db = AssetDb::load(&dir, &bundles.iter().map(String::as_str).collect::<Vec<_>>())?;
    let seq = db.by_name(sequence).with_context(|| format!("no sequence {sequence}"))?;
    let mut out = Vec::new();
    for actor in seq.guids("ActorAssets").into_iter().filter_map(|g| db.get(g)) {
        let role = actor.guid("TrackSelectionEnumValue").and_then(|g| db.get(g)).and_then(|v| v.int("Value"));
        let Some(set) = actor.guid("TagCollectionSet").and_then(|g| db.get(g)) else { continue };
        for c in set.guids("TagCollectionKeys").into_iter().filter_map(|g| db.get(g)) {
            for t in c.guids("TagKeys").into_iter().filter_map(|g| db.get(g)) {
                if !t.is("AudioEventTag") {
                    continue;
                }
                out.push(NisAudioTag {
                    role: role.unwrap_or(-1),
                    time: num(&t, "Time"),
                    duration: num(&t, "Duration"),
                    system: t.int("SystemID").unwrap_or(0),
                    event: t.int("EventID").unwrap_or(0),
                    name: t.value("EventName").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                });
            }
        }
    }
    out.sort_by(|a, b| a.time.total_cmp(&b.time));
    Ok(out)
}

pub fn nis_node_signals(big_root: &Path, bundles: &[String], node: &str) -> Result<Vec<String>> {
    let dir = big_root.join(crate::rig_anim::ANIM_DIR);
    let db = AssetDb::load(&dir, &bundles.iter().map(String::as_str).collect::<Vec<_>>())?;
    let n = db.by_name(node).with_context(|| format!("no node {node}"))?;
    Ok(n.guids("ActiveSignals").into_iter().filter_map(|g| db.get(g)).map(|a| a.name().to_string()).collect())
}
