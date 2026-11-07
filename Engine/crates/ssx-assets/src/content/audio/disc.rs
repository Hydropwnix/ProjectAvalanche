use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ssx_content::{Asset, BASE_NAMESPACE, ContentId, Kind, LoadError, Source};
use ssx_formats::csi::{CsiFile, Kind as CsiKind};

use super::super::files::members;
use super::super::{path_segments, segment};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscSound {
    pub file: String,
    pub name: String,
    pub function: bool,
    pub csis: u32,
}

pub struct DiscSounds {
    pub by_id: BTreeMap<ContentId, DiscSound>,
}

impl DiscSounds {
    pub fn index(big_root: &Path) -> Result<Self> {
        let mut by_id = BTreeMap::new();
        for (stem, holders) in members(big_root, "data/xenon/audio", "csi")? {
            let path = &holders[0];
            let f = CsiFile::parse(&std::fs::read(path)?).with_context(|| format!("{}", path.display()))?;
            let Some(m) = path_segments(&stem) else { continue };
            for e in f.entries.iter().filter(|e| e.kind != CsiKind::Global) {
                let Some(n) = segment(&e.name) else { continue };
                let Ok(id) = ContentId::new(BASE_NAMESPACE, Kind::Sound, &format!("{m}/{n}")) else { continue };
                by_id.entry(id).or_insert(DiscSound {
                    file: stem.clone(),
                    name: e.name.clone(),
                    function: e.kind == CsiKind::Function,
                    csis: e.id,
                });
            }
        }
        Ok(Self { by_id })
    }

    pub fn get(&self, id: &ContentId) -> Option<&DiscSound> {
        self.by_id.get(id)
    }
}

impl Source for DiscSounds {
    fn name(&self) -> &str {
        BASE_NAMESPACE
    }

    fn has(&self, id: &ContentId) -> bool {
        self.by_id.contains_key(id)
    }

    fn load(&self, id: &ContentId) -> Result<Asset, LoadError> {
        let s = self.by_id.get(id).ok_or_else(|| LoadError::Missing(id.clone()))?;
        let mut t = toml::Table::new();
        t.insert("interface".into(), s.file.clone().into());
        t.insert("name".into(), s.name.clone().into());
        t.insert("kind".into(), (if s.function { "function" } else { "event" }).into());
        t.insert("csis_id".into(), i64::from(s.csis).into());
        Ok(Asset::Record(t))
    }

    fn list(&self, kind: Kind) -> Vec<ContentId> {
        if kind == Kind::Sound { self.by_id.keys().cloned().collect() } else { Vec::new() }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiscTrack {
    Licensed(u32),
    Pathfinder { project: String, track: usize, mpf: PathBuf },
}

pub struct DiscMusic {
    pub by_id: BTreeMap<ContentId, DiscTrack>,
}

impl DiscMusic {
    pub fn index(big_root: &Path) -> Result<Self> {
        let mut by_id = BTreeMap::new();
        let info = members(big_root, "data/xenon/audio/music", "bin")?;
        if let Some(holders) = info.get("music_musicboxsonginfo") {
            for s in crate::musicbox::parse_song_info(&std::fs::read(&holders[0])?)? {
                if let Ok(id) = ContentId::new(BASE_NAMESPACE, Kind::Music, &format!("licensed/{}", s.id)) {
                    by_id.insert(id, DiscTrack::Licensed(s.id));
                }
            }
        }
        for (stem, holders) in members(big_root, "data/xenon/audio/pathfinder", "mpf")? {
            let Some(project) = stem.split('/').next_back().and_then(segment) else { continue };
            let mpf = ssx_formats::mpf::MpfFile::parse(&std::fs::read(&holders[0])?)
                .with_context(|| format!("{}", holders[0].display()))?;
            for track in 0..mpf.tracks.len() {
                let path = format!("pathfinder/{project}/{track}");
                if let Ok(id) = ContentId::new(BASE_NAMESPACE, Kind::Music, &path) {
                    by_id
                        .insert(id, DiscTrack::Pathfinder { project: project.clone(), track, mpf: holders[0].clone() });
                }
            }
        }
        Ok(Self { by_id })
    }

    pub fn get(&self, id: &ContentId) -> Option<&DiscTrack> {
        self.by_id.get(id)
    }
}

impl Source for DiscMusic {
    fn name(&self) -> &str {
        BASE_NAMESPACE
    }

    fn has(&self, id: &ContentId) -> bool {
        self.by_id.contains_key(id)
    }

    fn load(&self, id: &ContentId) -> Result<Asset, LoadError> {
        let mut t = toml::Table::new();
        match self.by_id.get(id).ok_or_else(|| LoadError::Missing(id.clone()))? {
            DiscTrack::Licensed(s) => {
                t.insert("kind".into(), "licensed".into());
                t.insert("sample_id".into(), i64::from(*s).into());
            }
            DiscTrack::Pathfinder { project, track, .. } => {
                t.insert("kind".into(), "pathfinder".into());
                t.insert("project".into(), project.clone().into());
                t.insert("track".into(), (*track as i64).into());
            }
        }
        Ok(Asset::Record(t))
    }

    fn list(&self, kind: Kind) -> Vec<ContentId> {
        if kind == Kind::Music { self.by_id.keys().cloned().collect() } else { Vec::new() }
    }
}
