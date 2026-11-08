use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use ssx_content::hash::{ContentIndex, hash_files};

pub const INDEX_FILE: &str = "content.tsv";

pub struct HashRun {
    pub index: ContentIndex,
    pub bytes: u64,
    pub seconds: f64,
    pub skipped: Vec<String>,
}

fn regular_files(root: &Path, skipped: &mut Vec<PathBuf>) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(&d).with_context(|| format!("listing {}", d.display()))? {
            let e = e.with_context(|| format!("listing {}", d.display()))?;
            let t = e.file_type().with_context(|| format!("reading {}", e.path().display()))?;
            if t.is_dir() {
                dirs.push(e.path());
            } else if t.is_file() && e.file_name().to_str().is_none_or(|n| ssx_files::partial_target(n).is_none()) {
                out.push(e.path());
            } else {
                skipped.push(e.path());
            }
        }
    }
    out.sort();
    Ok(out)
}

pub fn hash_data(data: &Path, threads: usize) -> Result<HashRun> {
    let big = data.join("big");
    if !big.is_dir() {
        bail!("{} is not a data folder: no big/ in it (the game data setup makes it)", data.display());
    }
    let mut skipped = Vec::new();
    let mut files = regular_files(&big, &mut skipped)?;
    let movies = data.join("disc").join(crate::setup::MOVIE_DIR);
    files.extend(super::vaults::files_with_ext(&movies, "vp6")?);
    let start = Instant::now();
    let results = hash_files(&files, threads);
    let seconds = start.elapsed().as_secs_f64();
    let mut index = ContentIndex::default();
    let mut bytes = 0u64;
    for (p, r) in files.iter().zip(results) {
        let (size, hash) = r.map_err(|e| anyhow::anyhow!("hashing {}: {e}", p.display()))?;
        bytes += size;
        index.entries.insert(super::vaults::rel(data, p), (size, hash));
    }
    let skipped = skipped.iter().map(|p| super::vaults::rel(data, p)).collect();
    Ok(HashRun { index, bytes, seconds, skipped })
}

pub fn write_index(data: &Path, index: &ContentIndex) -> Result<PathBuf> {
    let out = data.join(INDEX_FILE);
    ssx_files::replace(&out, index.to_tsv().as_bytes(), &ssx_files::Options::new())
        .with_context(|| format!("writing {}", out.display()))?;
    Ok(out)
}

pub fn ensure_index(data: &Path, threads: usize) -> Result<ContentIndex> {
    if let Ok(text) = std::fs::read_to_string(data.join(INDEX_FILE))
        && let Ok(index) = ContentIndex::from_tsv(&text)
    {
        return Ok(index);
    }
    let run = hash_data(data, threads)?;
    write_index(data, &run.index)?;
    Ok(run.index)
}
