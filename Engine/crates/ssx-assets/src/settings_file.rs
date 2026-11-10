use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{Map, Value as Json};

pub const MAX_FILE: u64 = 1024 * 1024;
pub const MAX_ASIDE: u32 = 999;

#[derive(Debug)]
pub enum UpdateError {
    Unreadable(String),
    Failed(anyhow::Error),
}

impl std::fmt::Display for UpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UpdateError::Unreadable(why) => write!(f, "the settings file cannot be read: {why}"),
            UpdateError::Failed(e) => write!(f, "{e:#}"),
        }
    }
}

fn object_of(path: &Path, bytes: Option<&[u8]>) -> Result<Map<String, Json>, String> {
    let Some(bytes) = bytes else { return Ok(Map::new()) };
    let shown = ssx_core::text::shown(&path.to_string_lossy()).to_string();
    if bytes.len() as u64 > MAX_FILE {
        return Err(format!("{shown}: larger than {MAX_FILE} bytes"));
    }
    match serde_json::from_slice::<Json>(bytes) {
        Ok(Json::Object(m)) => Ok(m),
        Ok(_) => Err(format!("{shown}: not a JSON object")),
        Err(e) => Err(format!("{shown}: {e}")),
    }
}

pub fn read_object(path: &Path) -> Result<Map<String, Json>> {
    let shown = || ssx_core::text::shown(&path.to_string_lossy()).to_string();
    let bytes = match ssx_files::open_plain(path).map_err(ssx_files::Error::into_io) {
        Ok(Some(f)) => {
            use std::io::Read;
            let mut bytes = Vec::new();
            f.take(MAX_FILE + 1).read_to_end(&mut bytes).with_context(|| format!("reading {}", shown()))?;
            Some(bytes)
        }
        Ok(None) => anyhow::bail!("{}: not a plain file (a link or a folder is there)", shown()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e).with_context(|| format!("reading {}", shown())),
    };
    object_of(path, bytes.as_deref()).map_err(anyhow::Error::msg)
}

pub fn merge_changes(theirs: &mut Map<String, Json>, base: &Map<String, Json>, ours: &Map<String, Json>, levels: u32) {
    for k in base.keys().filter(|k| !ours.contains_key(*k)) {
        theirs.remove(k);
    }
    for (k, o) in ours {
        let b = base.get(k);
        if b == Some(o) {
            continue;
        }
        match (o, b, theirs.get_mut(k)) {
            (Json::Object(o), Some(Json::Object(b)), Some(Json::Object(t))) if levels > 1 => {
                merge_changes(t, b, o, levels - 1)
            }
            _ => {
                theirs.insert(k.clone(), o.clone());
            }
        }
    }
}

pub fn update_object(
    path: &Path,
    change: impl FnOnce(&mut Map<String, Json>) -> Result<(), String>,
) -> Result<Map<String, Json>, UpdateError> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        ssx_files::create_dir_all(dir).map_err(|e| UpdateError::Failed(e.into()))?;
    }
    let mut written = None;
    let mut read = false;
    let result = ssx_files::update(path, &ssx_files::Options::new(), MAX_FILE, |old| {
        read = true;
        let mut m = object_of(path, old.as_deref()).map_err(Step::Unreadable)?;
        change(&mut m).map_err(|e| Step::Failed(anyhow::anyhow!(e)))?;
        let text = serde_json::to_vec_pretty(&Json::Object(m.clone())).map_err(|e| Step::Failed(e.into()))?;
        written = Some(m);
        Ok::<_, Step>(text)
    });
    match result {
        Ok(()) => written.ok_or_else(|| UpdateError::Failed(anyhow::anyhow!("nothing was written"))),
        Err(Step::Unreadable(why)) => Err(UpdateError::Unreadable(why)),
        Err(Step::Files(e)) if !read && e.path().file_name() == path.file_name() => {
            Err(UpdateError::Unreadable(e.to_string()))
        }
        Err(Step::Files(e)) => Err(UpdateError::Failed(e.into())),
        Err(Step::Failed(e)) => Err(UpdateError::Failed(e)),
    }
}

enum Step {
    Unreadable(String),
    Files(ssx_files::Error),
    Failed(anyhow::Error),
}

impl From<ssx_files::Error> for Step {
    fn from(e: ssx_files::Error) -> Self {
        Step::Files(e)
    }
}

pub fn replace_unreadable(path: &Path, members: Map<String, Json>) -> Result<PathBuf> {
    let name = path.file_name().context("the settings file has no name")?.to_os_string();
    let names = (1..=MAX_ASIDE).map(|n| {
        let mut aside = name.clone();
        aside.push(format!(".broken-{n}"));
        path.with_file_name(aside)
    });
    let shown = ssx_core::text::shown(&path.to_string_lossy()).to_string();
    let aside = ssx_files::move_aside(path, names)
        .with_context(|| format!("moving {shown} aside"))?
        .with_context(|| format!("{shown}: every name up to .broken-{MAX_ASIDE} is taken"))?;
    update_object(path, |m| {
        m.extend(members);
        Ok(())
    })
    .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(aside)
}

#[cfg(test)]
mod tests;
