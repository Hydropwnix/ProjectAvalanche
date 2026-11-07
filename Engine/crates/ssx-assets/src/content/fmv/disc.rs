use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use ssx_content::{Asset, ContentId, Kind, LoadError, Source};

use super::super::segment;

pub const MOVIE_DIR: &str = "data/common/fmv";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscMovie {
    pub path: PathBuf,
}

pub struct DiscMovies {
    pub files: BTreeMap<ContentId, PathBuf>,
}

impl DiscMovies {
    pub fn index(big_root: &Path) -> Result<Self> {
        let mut files = BTreeMap::new();
        for dir in [big_root.join("../disc").join(MOVIE_DIR), big_root.join("bootcache").join(MOVIE_DIR)] {
            let Ok(list) = std::fs::read_dir(&dir) else { continue };
            let mut found: Vec<PathBuf> = list.filter_map(|e| e.ok().map(|e| e.path())).collect();
            found.sort();
            for p in found.into_iter().filter(|p| p.is_file()) {
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
                let Some(stem) = name.strip_suffix(".vp6") else { continue };
                if let Some(id) = segment(stem).and_then(|s| ContentId::new("ssx", Kind::Movie, &s).ok()) {
                    files.entry(id).or_insert(p);
                }
            }
        }
        Ok(DiscMovies { files })
    }
}

impl Source for DiscMovies {
    fn name(&self) -> &str {
        "ssx"
    }

    fn has(&self, id: &ContentId) -> bool {
        self.files.contains_key(id)
    }

    fn load(&self, id: &ContentId) -> Result<Asset, LoadError> {
        let path = self.files.get(id).ok_or_else(|| LoadError::Missing(id.clone()))?;
        Ok(Asset::Value(Arc::new(DiscMovie { path: path.clone() })))
    }

    fn list(&self, kind: Kind) -> Vec<ContentId> {
        if kind == Kind::Movie { self.files.keys().cloned().collect() } else { Vec::new() }
    }
}
