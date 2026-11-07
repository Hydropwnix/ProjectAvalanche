use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use ssx_content::hash::ContentIndex;
use ssx_content::limits::MAX_FILE_BYTES;
use ssx_content::zip::{INDEX_FILE, PackFile, ZipArchive, ZipError, ZipLimits, write_zip};
use ssx_content::{ContentHash, hash::hash_file};

use super::check::{CheckOptions, Report, check};

pub struct Packed {
    pub zip: PathBuf,
    pub files: usize,
    pub bytes: u64,
    pub size: u64,
    pub hash: ContentHash,
}

pub fn pack(dir: &Path, out: Option<&Path>, opts: &CheckOptions<'_>) -> Result<(Report, Option<Packed>)> {
    let report = check(dir, opts)?;
    if report.checked.errors() > 0 {
        return Ok((report, None));
    }
    let (Some(m), Some(tree)) = (&report.checked.manifest, &report.checked.tree) else {
        bail!("{}: checked without a manifest or a tree", dir.display());
    };
    let r = &m.resource;
    let zip = match out {
        Some(p) => p.to_path_buf(),
        None => {
            let parent = std::fs::canonicalize(dir)?.parent().map(Path::to_path_buf).unwrap_or_default();
            parent.join(format!("{}-{}.zip", r.name, r.version))
        }
    };
    let files: Vec<String> = tree.files().map(str::to_string).collect();
    let mut index = ContentIndex::default();
    for f in &files {
        index.entries.insert(f.clone(), tree.copy_to(f, MAX_FILE_BYTES, &mut std::io::sink())?);
    }
    let mut names = files.clone();
    names.push(INDEX_FILE.to_string());
    names.sort();
    let index_text = index.to_tsv();
    let packed = ssx_files::replace_with(&zip, &ssx_files::Options::new(), |file| -> Result<()> {
        let it = names.iter().map(|n| -> Result<PackFile, ZipError> {
            let bytes = if n == INDEX_FILE {
                index_text.clone().into_bytes()
            } else {
                tree.read(n, MAX_FILE_BYTES).map_err(|e| ZipError::Refused(e.to_string()))?
            };
            Ok(PackFile { name: n.clone(), bytes })
        });
        write_zip(std::io::BufWriter::new(&mut *file), it, &ZipLimits::default())?;
        std::io::Seek::rewind(file)?;
        verify(file, &index)
    });
    packed.with_context(|| format!("packing {}", dir.display()))?;
    let (size, hash) = hash_file(&zip)?;
    let bytes = index.entries.values().map(|(s, _)| s).sum();
    Ok((report, Some(Packed { zip, files: files.len(), bytes, size, hash })))
}

fn verify(zip: &mut std::fs::File, index: &ContentIndex) -> Result<()> {
    let mut a = ZipArchive::open(std::io::BufReader::new(zip), &ZipLimits::default())?;
    let mut got = BTreeMap::new();
    for i in 0..a.entries().len() {
        let (name, size) = (a.entries()[i].name.clone(), a.entries()[i].size);
        let hash = a.extract(i, &mut std::io::sink())?;
        if name != INDEX_FILE {
            got.insert(name, (size, hash));
        }
    }
    if got != index.entries {
        bail!("the archive read back differs from the files (did a file change while it was packed?)");
    }
    Ok(())
}
