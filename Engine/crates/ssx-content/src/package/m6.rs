use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock, PoisonError};

use super::PackageSource;
use crate::font::{self, Converted};
use crate::id::{ContentId, Kind};
use crate::manifest::Entry;
use crate::path::EntryKind;
use crate::registry::{Asset, LoadError};
use crate::strings::{self, Lang};

pub(super) type FontOnce = OnceLock<Result<Arc<Converted>, String>>;

#[derive(Clone, Debug, Default)]
pub struct PackageStrings {
    pub texts: BTreeMap<ContentId, BTreeMap<Lang, String>>,
    pub problems: Vec<String>,
}

impl PackageStrings {
    pub fn read(source: &PackageSource) -> PackageStrings {
        let mut out = PackageStrings::default();
        let ns = source.manifest.resource.name.clone();
        let files: Vec<String> = source
            .tree
            .entries()
            .filter(|(p, k)| *k == EntryKind::File && strings::table_lang(p).is_some())
            .map(|(p, _)| p.to_string())
            .collect();
        for file in files {
            let lang = match strings::table_lang(&file) {
                Some(Ok(l)) => l,
                Some(Err(e)) => {
                    out.problems.push(e);
                    continue;
                }
                None => continue,
            };
            let table = source
                .tree
                .read(&file, crate::limits::MAX_RECORD_BYTES)
                .map_err(|e| vec![(0, 0, e.to_string())])
                .and_then(|b| strings::parse_table(&b, &file, lang, &ns));
            match table {
                Ok(t) => {
                    for (id, e) in t.entries {
                        out.texts.entry(id).or_default().insert(lang, e.text);
                    }
                }
                Err(ps) => out.problems.push(super::problems_text(&file, ps)),
            }
        }
        out
    }

    pub fn text(&self, id: &ContentId, lang: Lang) -> Option<&str> {
        self.texts.get(id)?.get(&lang).map(String::as_str)
    }
}

impl PackageSource {
    pub fn strings(&self) -> Arc<PackageStrings> {
        self.strings.get_or_init(|| Arc::new(PackageStrings::read(self))).clone()
    }

    pub fn declares_movie(&self, file: &str) -> Result<(), String> {
        let mut budget = crate::glob::Budget(1_000_000);
        let hit = self.actionscript_patterns.iter().any(|p| p.matches(file, &mut budget).unwrap_or(false));
        if hit { Ok(()) } else { Err(format!("{file}: a movie the manifest does not declare in [actionscript] files")) }
    }

    pub(super) fn load_movie(&self, id: &ContentId, entry: &Entry) -> Result<Asset, LoadError> {
        let failed = |message: String| LoadError::Failed { id: id.clone(), message };
        let Entry::File(rel) = entry else { return Err(failed("a movie is one file, not files by role".into())) };
        let ext = rel.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
        if !super::MOVIE_EXTENSIONS.contains(&ext.as_str()) {
            return Err(failed(format!(
                "{rel}: a screen is a movie (.swf / .gfx); layouts (.json) are not read from packages"
            )));
        }
        self.declares_movie(rel).map_err(failed)?;
        let cap = ssx_formats::gfx::Limits::default().max_file_bytes as u64;
        let bytes = self.read(id, rel, cap)?;
        crate::ui::Container::of(&bytes).map_err(|e| failed(format!("{rel}: {e}")))?;
        Ok(Asset::Bytes(Arc::from(bytes)))
    }

    pub(super) fn load_font(&self, id: &ContentId, entry: &Entry) -> Result<Asset, LoadError> {
        let failed = |message: String| LoadError::Failed { id: id.clone(), message };
        let Entry::File(rel) = entry else { return Err(failed("a font is one file, not files by role".into())) };
        let cell = {
            let mut fonts = self.fonts.lock().unwrap_or_else(PoisonError::into_inner);
            fonts.entry(rel.clone()).or_default().clone()
        };
        let converted = cell.get_or_init(|| {
            let limits = font::Limits::default();
            let bytes = self.tree.read(rel, limits.max_file_bytes as u64).map_err(|e| e.to_string())?;
            font::convert(&bytes, id.as_str(), &limits).map(Arc::new).map_err(|e| format!("{rel}: {e}"))
        });
        match converted {
            Ok(c) => Ok(Asset::Value(c.clone())),
            Err(e) => Err(failed(e.clone())),
        }
    }

    pub(super) fn load_string(&self, id: &ContentId) -> Result<Asset, LoadError> {
        match self.strings().texts.get(id) {
            Some(t) => Ok(Asset::Value(Arc::new(t.clone()))),
            None => Err(LoadError::Missing(id.clone())),
        }
    }

    pub(super) fn has_string(&self, id: &ContentId) -> bool {
        id.kind() == Kind::String && self.strings().texts.contains_key(id)
    }
}
