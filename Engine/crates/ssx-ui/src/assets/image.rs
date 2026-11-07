use anyhow::{Context, Result};

use crate::fs::{FileSource, normalise};

#[derive(Clone, Debug)]
pub struct ImageData {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub source: String,
}

pub fn resolve_external(fs: &dyn FileSource, dir: &str, name: &str) -> Option<String> {
    let full = normalise(&format!("{dir}{name}"));
    let stem = full.rsplit_once('.').map_or(full.as_str(), |(s, _)| s);
    let xpr = format!("{stem}.xpr");
    if fs.exists(&xpr) {
        return Some(xpr);
    }
    fs.exists(&full).then_some(full)
}

pub fn load(fs: &dyn FileSource, path: &str) -> Result<ImageData> {
    fs.image(path).with_context(|| format!("{path} not loaded"))
}
