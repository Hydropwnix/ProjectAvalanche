use std::sync::{Arc, OnceLock, PoisonError};

use super::{PackageSource, is_toml, problems_text};
use crate::audio::music::{MusicDef, TrackAudio};
use crate::audio::sounds::{SoundClass, SoundDef};
use crate::audio::{AudioInfo, Limits, probe_head};
use crate::id::{ContentId, Kind};
use crate::limits::MAX_RECORD_BYTES;
use crate::manifest::Entry;
use crate::registry::{Asset, LoadError};

pub(super) type MusicOnce = OnceLock<Result<Arc<MusicDef>, String>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum MusicClass {
    Added,
    Licensed,
    Pathfinder,
    Other,
}

pub fn audio_limits(kind: Kind) -> Limits {
    if kind == Kind::Music { Limits::music() } else { Limits::sound() }
}

impl PackageSource {
    fn audio_entry<'a>(&'a self, id: &ContentId) -> Result<&'a str, LoadError> {
        let failed = |message: String| LoadError::Failed { id: id.clone(), message };
        match self.manifest.entry(id) {
            Some(Entry::File(rel)) => Ok(rel),
            Some(Entry::Files(_)) => Err(failed(format!("a {} is one file, not files by role", id.kind().as_str()))),
            None => Err(LoadError::Missing(id.clone())),
        }
    }

    pub fn sound_def(&self, id: &ContentId) -> Result<SoundDef, LoadError> {
        let rel = self.audio_entry(id)?;
        if !is_toml(rel) {
            return Ok(SoundDef::file(rel));
        }
        let bytes = self.read(id, rel, MAX_RECORD_BYTES)?;
        crate::audio::sounds::parse_toml(&bytes, SoundClass::of(id))
            .map_err(|ps| LoadError::Failed { id: id.clone(), message: problems_text(rel, ps) })
    }

    pub fn music_def(&self, id: &ContentId) -> Result<MusicDef, LoadError> {
        self.music_def_shared(id).map(|d| (*d).clone())
    }

    fn music_def_shared(&self, id: &ContentId) -> Result<Arc<MusicDef>, LoadError> {
        let rel = self.audio_entry(id)?;
        let over = (id.namespace() != self.manifest.resource.name).then_some(id);
        if !is_toml(rel) {
            if over.is_some_and(|o| o.path().starts_with("pathfinder/")) {
                let audio = TrackAudio::Looped { file: rel.to_string(), region: None };
                return Ok(Arc::new(MusicDef::Track { audio, gain: 1.0 }));
            }
            return Ok(Arc::new(MusicDef::song_file(rel)));
        }
        let class = match over {
            None => MusicClass::Added,
            Some(o) if o.path().starts_with("licensed/") => MusicClass::Licensed,
            Some(o) if o.path().starts_with("pathfinder/") => MusicClass::Pathfinder,
            Some(_) => MusicClass::Other,
        };
        let once = self
            .music_defs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry((rel.to_string(), class))
            .or_default()
            .clone();
        let parsed = once.get_or_init(|| {
            let bytes = self.tree.read(rel, MAX_RECORD_BYTES).map_err(|e| e.to_string())?;
            crate::audio::music::parse_toml(&bytes, over).map(Arc::new).map_err(|ps| problems_text(rel, ps))
        });
        parsed.clone().map_err(|message| LoadError::Failed { id: id.clone(), message })
    }

    pub(super) fn load_sound(&self, id: &ContentId) -> Result<Asset, LoadError> {
        Ok(Asset::Value(Arc::new(self.sound_def(id)?)))
    }

    pub(super) fn load_music(&self, id: &ContentId) -> Result<Asset, LoadError> {
        Ok(Asset::Value(self.music_def_shared(id)?))
    }

    pub(super) fn probe_audio(&self, id: &ContentId) -> Result<Vec<(String, AudioInfo)>, LoadError> {
        let files: Vec<String> = match id.kind() {
            Kind::Sound => self.sound_def(id)?.files,
            Kind::Music => self.music_def_shared(id)?.files().into_iter().map(str::to_string).collect(),
            _ => return Ok(Vec::new()),
        };
        let mut out = Vec::with_capacity(files.len());
        for rel in files {
            out.push((rel.clone(), self.probe_audio_file(id, &rel)?));
        }
        Ok(out)
    }

    pub fn probe_audio_file(&self, id: &ContentId, rel: &str) -> Result<AudioInfo, LoadError> {
        let key = (rel.to_string(), id.kind());
        let limits = &audio_limits(id.kind());
        let cached = self.audio_heads.lock().unwrap_or_else(PoisonError::into_inner).get(&key).cloned();
        let measured = cached.unwrap_or_else(|| {
            let size = self.tree.file_size(rel).map_err(|e| e.to_string());
            let mut read = |offset: u64, len: u64| self.tree.read_range(rel, offset, len).map_err(|e| e.to_string());
            let m = size.and_then(|size| probe_head(rel, size, &mut read, limits).map_err(|e| e.to_string()));
            self.audio_heads.lock().unwrap_or_else(PoisonError::into_inner).insert(key, m.clone());
            m
        });
        measured.map_err(|message| LoadError::Failed { id: id.clone(), message })
    }

    pub fn read_audio_file(&self, id: &ContentId, rel: &str, limits: &Limits) -> Result<Arc<[u8]>, LoadError> {
        Ok(self.read(id, rel, limits.max_file_bytes)?.into())
    }
}
