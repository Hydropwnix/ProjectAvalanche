use std::path::Path;

use anyhow::{Context, Result};
use ssx_content::hash::{ContentHash, ContentIndex};

use super::folder::{INDEX_FILE, verify_index};

pub const ROWS_FILE: &str = "setup-rows.tsv";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part<'a> {
    Archive(&'a str),
    Movies,
}

impl Part<'_> {
    pub fn prefix(&self) -> String {
        match self {
            Part::Archive(name) => format!("big/{name}/"),
            Part::Movies => "disc/".to_string(),
        }
    }
}

pub fn of_part<'a>(
    index: &'a ContentIndex,
    part: Part<'_>,
) -> impl Iterator<Item = (&'a String, &'a (u64, ContentHash))> {
    let prefix = part.prefix();
    index.entries.range(prefix.clone()..).take_while(move |(p, _)| p.starts_with(&prefix))
}

pub fn digest(index: &ContentIndex, part: Part<'_>) -> ContentHash {
    let mut h = blake3::Hasher::new();
    for (path, (size, hash)) in of_part(index, part) {
        h.update(format!("{path}\t{size}\t{hash}\n").as_bytes());
    }
    ContentHash(*h.finalize().as_bytes())
}

pub fn take_part(from: &ContentIndex, part: Part<'_>, to: &mut ContentIndex) {
    for (path, row) in of_part(from, part) {
        to.entries.insert(path.clone(), *row);
    }
}

pub fn earlier(out: &Path) -> ContentIndex {
    for name in [ROWS_FILE, INDEX_FILE] {
        if let Ok(Some(text)) = super::folder::read_text(out, name)
            && let Ok(index) = verify_index(&text, None)
        {
            return index;
        }
    }
    ContentIndex::default()
}

pub fn write(out: &Path, rows: &ContentIndex) -> Result<()> {
    let path = out.join(ROWS_FILE);
    ssx_files::replace(&path, rows.to_tsv().as_bytes(), &ssx_files::Options::new())
        .with_context(|| format!("writing {}", path.display()))
}

pub fn remove(out: &Path) -> Result<()> {
    let path = out.join(ROWS_FILE);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(ssx_files::sync_folder(out)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
    }
}
