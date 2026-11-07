use crate::glob::{Budget, Pattern};
use crate::hash::ContentHash;
use crate::limits::{MAX_FILE_BYTES, MAX_PACKAGE_BYTES};
use crate::manifest::{MANIFEST_FILE, Manifest};
use crate::path::{EntryKind, PackageTree};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum FileRole {
    Manifest,
    Content,
    Patch,
    Script,
    Data,
}

impl FileRole {
    pub const fn name(self) -> &'static str {
        match self {
            FileRole::Manifest => "manifest",
            FileRole::Content => "content",
            FileRole::Patch => "patch",
            FileRole::Script => "script",
            FileRole::Data => "data",
        }
    }

    pub fn parse(s: &str) -> Option<FileRole> {
        Some(match s {
            "manifest" => FileRole::Manifest,
            "content" => FileRole::Content,
            "patch" => FileRole::Patch,
            "script" => FileRole::Script,
            "data" => FileRole::Data,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceFiles {
    pub files: Vec<super::File>,
    pub kept_back: usize,
    pub server_scripts: bool,
    pub client_scripts: bool,
    pub package: ContentHash,
}

const MATCH_BUDGET: u64 = 1 << 16;

fn patterns(list: &[String]) -> Result<Vec<Pattern>, String> {
    list.iter().map(|p| Pattern::parse(p)).collect()
}

fn any(ps: &[Pattern], path: &str) -> Result<bool, String> {
    let mut budget = Budget(MATCH_BUDGET);
    for p in ps {
        if p.matches(path, &mut budget).map_err(|e| format!("{path}: {e}"))? {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn files_of(tree: &PackageTree, m: &Manifest) -> Result<ResourceFiles, String> {
    let server = patterns(&m.scripts.server)?;
    let client = patterns(&[m.scripts.client.clone(), m.scripts.shared.clone()].concat())?;
    let server_only = patterns(&m.files.server_only)?;
    let lazy = patterns(&m.files.lazy)?;
    let mut named = std::collections::BTreeMap::new();
    for e in m.add.values().chain(m.overrides.values()) {
        for f in e.files() {
            named.insert(f.to_string(), FileRole::Content);
        }
    }
    for f in m.patches.values() {
        named.insert(f.clone(), FileRole::Patch);
    }
    let server_scripts = !m.scripts.server.is_empty() || !m.scripts.shared.is_empty();
    let mut out = ResourceFiles {
        files: Vec::new(),
        kept_back: 0,
        server_scripts,
        client_scripts: false,
        package: ContentHash([0; 32]),
    };
    let mut every: Vec<(String, ContentHash)> = Vec::new();
    let mut total = 0u64;
    let paths: Vec<String> =
        tree.entries().filter(|(_, k)| *k == EntryKind::File).map(|(p, _)| p.to_string()).collect();
    for path in paths {
        let is_server = any(&server, &path)?;
        if is_server || any(&server_only, &path)? {
            if named.contains_key(&path) || path == MANIFEST_FILE {
                return Err(format!("{path}: named by the package's content, so it cannot be kept on the server"));
            }
            out.kept_back += 1;
            let (size, hash) = hash_file(tree, &path)?;
            total += size;
            if total > MAX_PACKAGE_BYTES {
                return Err(format!("more than {MAX_PACKAGE_BYTES} bytes of files"));
            }
            every.push((path, hash));
            continue;
        }
        let is_lazy = any(&lazy, &path)?;
        if is_lazy && (named.contains_key(&path) || path == MANIFEST_FILE) {
            return Err(format!("{path}: named by the package's content, so it cannot be lazy"));
        }
        let role = if path == MANIFEST_FILE {
            FileRole::Manifest
        } else if let Some(r) = named.get(&path) {
            *r
        } else if any(&client, &path)? {
            out.client_scripts = true;
            FileRole::Script
        } else {
            FileRole::Data
        };
        let (size, hash) = hash_file(tree, &path)?;
        total += size;
        if total > MAX_PACKAGE_BYTES {
            return Err(format!("more than {MAX_PACKAGE_BYTES} bytes of files"));
        }
        every.push((path.clone(), hash));
        out.files.push(super::File { path, hash, size, role, lazy: is_lazy });
    }
    every.sort();
    let mut rows = Vec::with_capacity(every.iter().map(|(p, _)| p.len() + 33).sum());
    for (path, hash) in &every {
        rows.extend_from_slice(path.as_bytes());
        rows.push(0);
        rows.extend_from_slice(&hash.0);
    }
    out.package = ContentHash::of(&rows);
    for f in named.keys() {
        if tree.kind(f) != Some(EntryKind::File) {
            return Err(format!("{f}: named by the package's content and not in it"));
        }
    }
    Ok(out)
}

fn hash_file(tree: &PackageTree, path: &str) -> Result<(u64, ContentHash), String> {
    tree.copy_to(path, MAX_FILE_BYTES, &mut std::io::sink()).map_err(|e| e.to_string())
}
