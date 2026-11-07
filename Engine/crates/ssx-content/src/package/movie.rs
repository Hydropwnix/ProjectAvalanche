use std::sync::Arc;

use super::PackageSource;
use crate::id::ContentId;
use crate::manifest::Entry;
use crate::movie::{Demuxer, Limits, MovieInfo};
use crate::registry::{Asset, LoadError};

#[derive(Clone, Debug)]
pub struct PackageMovieHead {
    pub rel: String,
    pub len: u64,
    pub info: Arc<MovieInfo>,
}

impl PackageSource {
    fn movie_rel<'a>(&self, id: &ContentId, entry: &'a Entry) -> Result<&'a str, LoadError> {
        let failed = |message: String| LoadError::Failed { id: id.clone(), message };
        let Entry::File(rel) = entry else { return Err(failed("a movie is one file, not files by role".into())) };
        if !rel.to_ascii_lowercase().ends_with(".webm") {
            return Err(failed(format!("{rel}: a movie is WebM (.webm: AV1 video, Vorbis audio)")));
        }
        Ok(rel)
    }

    pub(super) fn load_movie_file(&self, id: &ContentId, entry: &Entry) -> Result<Asset, LoadError> {
        let rel = self.movie_rel(id, entry)?.to_string();
        let (file, len) = self.open_movie(id)?;
        let demux = Demuxer::open(file, len, &Limits::default())
            .map_err(|e| LoadError::Failed { id: id.clone(), message: format!("{rel}: {e}") })?;
        Ok(Asset::Value(Arc::new(PackageMovieHead { rel, len, info: Arc::new(demux.info().clone()) })))
    }

    pub(super) fn open_movie(&self, id: &ContentId) -> Result<(std::fs::File, u64), LoadError> {
        let entry = self.manifest.entry(id).ok_or_else(|| LoadError::Missing(id.clone()))?;
        let rel = self.movie_rel(id, entry)?;
        self.tree
            .open_read(rel, Limits::default().max_file_bytes)
            .map_err(|e| LoadError::Failed { id: id.clone(), message: e.to_string() })
    }
}
