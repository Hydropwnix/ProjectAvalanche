use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ssx_assets::vault_db::{fnv1, single_text};
use ssx_formats::attrib::{Schema, Vault, collection};

pub const TRACKS_DIR: &str = "data0/data/xenon/gameexplorer";

const CLASS_CHARACTER: u32 = fnv1("character");
const FIELD_CHARACTER_ID: u32 = 0x8097_DCBE;
const FIELD_CHARACTER_LABEL: u32 = 0xEAA0_A6F7;

pub fn tracks(big_root: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(big_root.join(TRACKS_DIR))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            std::fs::read_dir(e.path())
                .into_iter()
                .flatten()
                .flatten()
                .any(|f| f.file_name().to_string_lossy().to_ascii_lowercase().ends_with("_mesh_subd.subd"))
        })
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    out.sort();
    out
}

pub fn track_dir(big_root: &Path, track: &str) -> PathBuf {
    big_root.join(TRACKS_DIR).join(track)
}

#[derive(Clone, Debug)]
pub struct DropInfo {
    pub text_id: String,
    pub peak: String,
    pub lighting: String,
}

#[derive(Default)]
pub struct DropCache {
    tracks: HashMap<String, Result<Vec<DropInfo>, String>>,
}

impl DropCache {
    pub fn get(&mut self, big_root: &Path, track: &str) -> &Result<Vec<DropInfo>, String> {
        self.tracks.entry(track.to_string()).or_insert_with(|| {
            ssx_assets::track_lighting::load_track_drops(big_root, track)
                .map(|drops| {
                    drops
                        .into_iter()
                        .map(|d| DropInfo {
                            text_id: d.text_id.unwrap_or_else(|| format!("{:08x}", d.key)),
                            peak: d.peak.unwrap_or_default(),
                            lighting: d.lighting.name,
                        })
                        .collect()
                })
                .map_err(|e| format!("{e:#}"))
        })
    }

    pub fn clear(&mut self) {
        self.tracks.clear();
    }
}

pub fn presets(drops: &[DropInfo]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for d in drops {
        if !out.contains(&d.lighting) {
            out.push(d.lighting.clone());
        }
    }
    out
}

#[derive(Clone, Debug)]
pub struct Rider {
    pub name: String,
    pub label: String,
}

pub fn riders(big_root: &Path) -> Result<Vec<Rider>> {
    let dir = big_root.join("bootcache/data/xenon/attribdb");
    let read = |stem: &str| -> Result<(Vec<u8>, Vec<u8>)> {
        let path = |ext: &str| dir.join(format!("{stem}.{ext}"));
        let vlt = std::fs::read(path("vlt")).with_context(|| format!("reading {}", path("vlt").display()))?;
        let bin = std::fs::read(path("bin")).with_context(|| format!("reading {}", path("bin").display()))?;
        Ok((vlt, bin))
    };
    let (schema_vlt, schema_bin) = read("attribdb_schema")?;
    let schema = Schema::from_vault(&Vault::parse(&schema_vlt, &schema_bin)?).context("reading the schema")?;
    let mut out = Vec::new();
    for stem in ["ge_attribdb", "attribdb"] {
        let (vlt, bin) = read(stem)?;
        let vault = Vault::parse(&vlt, &bin).with_context(|| format!("parsing {stem}.vlt"))?;
        for c in collection::collections(&vault)?.iter().filter(|c| c.class == CLASS_CHARACTER) {
            let decoded = c.decode(&vault, &schema)?;
            let text = |k: u32| decoded.fields.iter().find(|f| f.key == k).and_then(|f| single_text(&f.data));
            let Some(name) = text(FIELD_CHARACTER_ID).filter(|n| fnv1(n) == c.key) else { continue };
            let label = text(FIELD_CHARACTER_LABEL).unwrap_or_else(|| name.clone());
            out.push(Rider { name, label });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}
