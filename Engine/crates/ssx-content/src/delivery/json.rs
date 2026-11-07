use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::{
    ClientManifest, FORMAT, File, FileRole, LevelId, MAX_BODY, MAX_CLASS, MAX_FILES, MAX_LEVELS, MAX_NAME,
    MAX_RESOURCES, MAX_ROW_ID, MAX_ROWS, MAX_SERVER_BYTES, MAX_TITLE, Resource, Row,
};
use crate::data::SimContentId;
use crate::hash::ContentHash;
use crate::id::valid_namespace;
use crate::limits::{MAX_FILE_BYTES, MAX_PACKAGE_BYTES};
use crate::manifest::MANIFEST_FILE;
use crate::path::check_package_path;
use crate::policy::ClientMods;
use crate::version::Version;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the server's resource manifest: {0}")]
pub struct ManifestError(pub String);

fn bad(m: impl Into<String>) -> ManifestError {
    ManifestError(m.into())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    format: u32,
    client_mods: String,
    client_scripts: bool,
    mirrors: Vec<String>,
    content_id: Option<String>,
    resources: Vec<RawResource>,
    levels: Vec<RawLevel>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawResource {
    name: String,
    version: String,
    title: String,
    group: u32,
    scripts: bool,
    files: Vec<RawFile>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    path: String,
    hash: String,
    size: u64,
    role: String,
    lazy: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLevel {
    level: String,
    id: String,
    files: String,
    rows: Vec<RawRow>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRow {
    id: String,
    class: String,
    hash: String,
}

fn q(s: &str) -> String {
    ssx_core::text::quoted(s).to_string()
}

fn hash(s: &str, what: &str) -> Result<ContentHash, ManifestError> {
    s.parse().map_err(|_| bad(format!("{what}: {} is not 64 lower-case hex digits", q(s))))
}

fn line(s: &str, max: usize, what: &str) -> Result<(), ManifestError> {
    if s.len() > max || s.chars().any(char::is_control) {
        return Err(bad(format!("{what}: at most {max} bytes on one line")));
    }
    Ok(())
}

fn level_name(s: &str) -> Result<(), ManifestError> {
    super::LevelName::parse(s).map(|_| ()).map_err(|e| bad(format!("levels: {}: {e}", q(s))))
}

pub fn from_json(body: &[u8]) -> Result<ClientManifest, ManifestError> {
    if body.len() > MAX_BODY {
        return Err(bad(format!("{} bytes, at most {MAX_BODY}", body.len())));
    }
    let raw: RawManifest =
        serde_json::from_slice(body).map_err(|e| bad(format!("does not read: {}", q(&e.to_string()))))?;
    if raw.format != FORMAT {
        return Err(bad(format!("format {}, this game reads {FORMAT}", raw.format)));
    }
    let client_mods =
        ClientMods::parse(&raw.client_mods).ok_or_else(|| bad(format!("client_mods: {}", q(&raw.client_mods))))?;
    let content_id: Option<SimContentId> = match &raw.content_id {
        Some(c) => Some(c.parse().map_err(|_| bad(format!("content_id: {}", q(c))))?),
        None => None,
    };
    if raw.mirrors.len() > super::mirror::MAX_MIRRORS {
        return Err(bad(format!("{} mirrors, at most {}", raw.mirrors.len(), super::mirror::MAX_MIRRORS)));
    }
    let mut mirrors = Vec::with_capacity(raw.mirrors.len());
    for m in &raw.mirrors {
        let mirror = super::Mirror::parse(m).map_err(|e| bad(format!("mirrors: {}: {}", q(m), q(&e))))?;
        if mirrors.contains(&mirror) {
            return Err(bad(format!("mirrors: {} twice", q(m))));
        }
        mirrors.push(mirror);
    }
    if raw.resources.len() > MAX_RESOURCES {
        return Err(bad(format!("{} resources, at most {MAX_RESOURCES}", raw.resources.len())));
    }
    if raw.levels.len() > MAX_LEVELS {
        return Err(bad(format!("{} levels, at most {MAX_LEVELS}", raw.levels.len())));
    }
    let mut names = BTreeSet::new();
    let (mut files, mut bytes) = (0usize, 0u64);
    let mut resources = Vec::with_capacity(raw.resources.len());
    for r in raw.resources {
        let what = format!("resource {}", q(&r.name));
        if r.name.len() > MAX_NAME || !valid_namespace(&r.name) || r.name == "ssx" || r.name == "ava" {
            return Err(bad(format!("{what}: not a package name of at most {MAX_NAME} bytes")));
        }
        if !names.insert(r.name.clone()) {
            return Err(bad(format!("{what} twice")));
        }
        let version: Version = r.version.parse().map_err(|_| bad(format!("{what}: version {}", q(&r.version))))?;
        line(&r.title, MAX_TITLE, &format!("{what}: title"))?;
        if r.group as usize >= MAX_RESOURCES {
            return Err(bad(format!("{what}: group {} (below {MAX_RESOURCES})", r.group)));
        }
        files += r.files.len();
        if files > MAX_FILES {
            return Err(bad(format!("more than {MAX_FILES} files")));
        }
        let mut paths = BTreeSet::new();
        let mut folded = BTreeSet::new();
        let mut size = 0u64;
        let mut out = Vec::with_capacity(r.files.len());
        for f in r.files {
            check_package_path(&f.path, false).map_err(|e| bad(format!("{what}: {}: {}", q(&f.path), q(&e))))?;
            if !paths.insert(f.path.clone()) || !folded.insert(f.path.to_lowercase()) {
                return Err(bad(format!("{what}: {} twice (or twice in another case)", q(&f.path))));
            }
            let hash = hash(&f.hash, &format!("{what}: {}", q(&f.path)))?;
            if f.size > MAX_FILE_BYTES {
                return Err(bad(format!("{what}: {}: {} bytes, at most {MAX_FILE_BYTES}", q(&f.path), f.size)));
            }
            size += f.size;
            let role =
                FileRole::parse(&f.role).ok_or_else(|| bad(format!("{what}: {}: role {}", q(&f.path), q(&f.role))))?;
            if (f.path == MANIFEST_FILE) != (role == FileRole::Manifest) || (role == FileRole::Manifest && f.lazy) {
                return Err(bad(format!("{what}: {}: the manifest is {MANIFEST_FILE}, never lazy", q(&f.path))));
            }
            out.push(File { path: f.path, hash, size: f.size, role, lazy: f.lazy });
        }
        if !paths.contains(MANIFEST_FILE) {
            return Err(bad(format!("{what}: no {MANIFEST_FILE}")));
        }
        for p in &folded {
            let folder = format!("{p}/");
            if folded.range(folder.clone()..).next().is_some_and(|n| n.starts_with(&folder)) {
                return Err(bad(format!("{what}: {} is a file and a folder", q(p))));
            }
        }
        if size > MAX_PACKAGE_BYTES {
            return Err(bad(format!("{what}: {size} bytes, at most {MAX_PACKAGE_BYTES}")));
        }
        bytes += size;
        if bytes > MAX_SERVER_BYTES {
            return Err(bad(format!("more than {MAX_SERVER_BYTES} bytes of files")));
        }
        resources.push(Resource {
            name: r.name,
            version,
            title: r.title,
            group: r.group,
            scripts: r.scripts,
            files: out,
        });
    }
    let mut levels = Vec::with_capacity(raw.levels.len());
    let mut seen = BTreeSet::new();
    for l in raw.levels {
        level_name(&l.level)?;
        if !seen.insert(l.level.clone()) {
            return Err(bad(format!("levels: {} twice", q(&l.level))));
        }
        let what = format!("level {}", q(&l.level));
        let id: SimContentId = l.id.parse().map_err(|_| bad(format!("{what}: id {}", q(&l.id))))?;
        let files = hash(&l.files, &format!("{what}: files"))?;
        if l.rows.len() > MAX_ROWS {
            return Err(bad(format!("{what}: {} rows, at most {MAX_ROWS}", l.rows.len())));
        }
        let mut rows = Vec::with_capacity(l.rows.len());
        for r in l.rows {
            line(&r.id, MAX_ROW_ID, &format!("{what}: a row's id"))?;
            line(&r.class, MAX_CLASS, &format!("{what}: a row's class"))?;
            if r.id.is_empty() || r.class.is_empty() || r.id.starts_with("file:") {
                return Err(bad(format!("{what}: row {}: an id and a class, no file row", q(&r.id))));
            }
            let hash = hash(&r.hash, &format!("{what}: row {}", q(&r.id)))?;
            rows.push(Row { id: r.id, class: r.class, hash });
        }
        levels.push(LevelId { level: l.level, id, files, rows });
    }
    let pairs: Vec<(String, SimContentId)> = levels.iter().map(|l| (l.level.clone(), l.id)).collect();
    match content_id {
        Some(c) if super::content_id(&pairs) != Some(c) => {
            return Err(bad("content_id is not the levels' ids combined"));
        }
        None if !levels.is_empty() => return Err(bad("levels without a content_id")),
        _ => {}
    }
    Ok(ClientManifest { client_mods, client_scripts: raw.client_scripts, mirrors, content_id, resources, levels })
}

impl ClientManifest {
    pub fn to_json(&self) -> String {
        let raw = RawManifest {
            format: FORMAT,
            client_mods: self.client_mods.name().into(),
            client_scripts: self.client_scripts,
            mirrors: self.mirrors.iter().map(ToString::to_string).collect(),
            content_id: self.content_id.map(|c| c.to_string()),
            resources: self
                .resources
                .iter()
                .map(|r| RawResource {
                    name: r.name.clone(),
                    version: r.version.to_string(),
                    title: r.title.clone(),
                    group: r.group,
                    scripts: r.scripts,
                    files: r
                        .files
                        .iter()
                        .map(|f| RawFile {
                            path: f.path.clone(),
                            hash: f.hash.to_string(),
                            size: f.size,
                            role: f.role.name().into(),
                            lazy: f.lazy,
                        })
                        .collect(),
                })
                .collect(),
            levels: self
                .levels
                .iter()
                .map(|l| RawLevel {
                    level: l.level.clone(),
                    id: l.id.to_string(),
                    files: l.files.to_string(),
                    rows: l
                        .rows
                        .iter()
                        .map(|r| RawRow { id: r.id.clone(), class: r.class.clone(), hash: r.hash.to_string() })
                        .collect(),
                })
                .collect(),
        };
        serde_json::to_string(&raw).unwrap_or_default()
    }
}
