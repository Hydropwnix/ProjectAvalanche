use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use ssx_content::strings::Lang;
use ssx_content::{Asset, ContentId, Kind, LoadError, Source};

use super::super::files::members;
use super::super::{path_segments, segment};

pub struct DiscUi {
    pub files: BTreeMap<ContentId, PathBuf>,
}

impl DiscUi {
    pub fn index(big_root: &Path) -> Result<Self> {
        let mut files = BTreeMap::new();
        for (stem, holders) in members(big_root, "data/common/fe", "gfx")? {
            if let Some(id) = path_segments(&stem).and_then(|p| ContentId::new("ssx", Kind::Ui, &p).ok()) {
                files.insert(id, holders[0].clone());
            }
        }
        Ok(DiscUi { files })
    }
}

impl Source for DiscUi {
    fn name(&self) -> &str {
        "ssx"
    }

    fn has(&self, id: &ContentId) -> bool {
        self.files.contains_key(id)
    }

    fn load(&self, id: &ContentId) -> Result<Asset, LoadError> {
        let f = self.files.get(id).ok_or_else(|| LoadError::Missing(id.clone()))?;
        let bytes = std::fs::read(f).map_err(|e| LoadError::Failed { id: id.clone(), message: e.to_string() })?;
        Ok(Asset::Bytes(Arc::from(bytes)))
    }

    fn list(&self, kind: Kind) -> Vec<ContentId> {
        if kind == Kind::Ui { self.files.keys().cloned().collect() } else { Vec::new() }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibraryFont {
    pub region: String,
    pub name: String,
}

pub struct DiscFonts {
    pub fonts: BTreeMap<ContentId, LibraryFont>,
}

impl DiscFonts {
    pub fn index(big_root: &Path) -> Result<Self> {
        let mut fonts = BTreeMap::new();
        for (stem, holders) in members(big_root, "data/common/fe/gfxfontlib", "gfx")? {
            let Some(region) = stem.strip_suffix("/gfxfontlib") else { continue };
            let path = &holders[0];
            let bytes = std::fs::read(path).with_context(|| format!("{}", path.display()))?;
            let movie = ssx_formats::gfx::Movie::parse(&bytes).with_context(|| format!("{}", path.display()))?;
            for (_, t) in movie.all_tags() {
                if let ssx_formats::gfx::Tag::Font(f) = &t.tag
                    && !f.glyphs.is_empty()
                    && let (Some(r), Some(n)) = (segment(region), segment(&f.name))
                    && let Ok(id) = ContentId::new("ssx", Kind::Font, &format!("{r}/{n}"))
                {
                    fonts.insert(id, LibraryFont { region: region.to_string(), name: f.name.clone() });
                }
            }
        }
        Ok(DiscFonts { fonts })
    }
}

impl Source for DiscFonts {
    fn name(&self) -> &str {
        "ssx"
    }

    fn has(&self, id: &ContentId) -> bool {
        self.fonts.contains_key(id)
    }

    fn load(&self, id: &ContentId) -> Result<Asset, LoadError> {
        let f = self.fonts.get(id).ok_or_else(|| LoadError::Missing(id.clone()))?;
        Ok(Asset::Value(Arc::new(f.clone())))
    }

    fn list(&self, kind: Kind) -> Vec<ContentId> {
        if kind == Kind::Font { self.fonts.keys().cloned().collect() } else { Vec::new() }
    }
}

pub struct DiscStrings {
    pub texts: BTreeMap<ContentId, KeyTexts>,
}

pub type KeyTexts = (String, BTreeMap<Lang, String>);

impl DiscStrings {
    pub fn index(big_root: &Path) -> Result<Self> {
        let tables = members(big_root, "data/xenon/loc", "db")?;
        let mut texts: BTreeMap<ContentId, (String, BTreeMap<Lang, String>)> = BTreeMap::new();
        for (stem, holders) in &tables {
            let Some(lang) = Lang::from_game_file(stem) else { continue };
            let l = ssx_formats::t3db::LocStrings::parse(&std::fs::read(&holders[0])?)
                .with_context(|| format!("{}", holders[0].display()))?;
            for (key, text) in l.by_hash.values() {
                let Some(id) = segment(key).and_then(|s| ContentId::new("ssx", Kind::String, &s).ok()) else {
                    continue;
                };
                let e = texts.entry(id).or_insert_with(|| (key.clone(), BTreeMap::new()));
                e.1.insert(lang, text.clone().unwrap_or_default());
            }
        }
        Ok(DiscStrings { texts })
    }

    pub fn text(&self, id: &ContentId, lang: Lang) -> Option<&str> {
        self.texts.get(id)?.1.get(&lang).map(String::as_str)
    }
}

impl Source for DiscStrings {
    fn name(&self) -> &str {
        "ssx"
    }

    fn has(&self, id: &ContentId) -> bool {
        self.texts.contains_key(id)
    }

    fn load(&self, id: &ContentId) -> Result<Asset, LoadError> {
        let (_, t) = self.texts.get(id).ok_or_else(|| LoadError::Missing(id.clone()))?;
        Ok(Asset::Value(Arc::new(t.clone())))
    }

    fn list(&self, kind: Kind) -> Vec<ContentId> {
        if kind == Kind::String { self.texts.keys().cloned().collect() } else { Vec::new() }
    }
}

pub const PORT_STRINGS: &str = include_str!("../../../../../assets/fe/strings/eng_us.json");

pub struct PortStrings {
    pub texts: BTreeMap<ContentId, (String, String)>,
}

impl PortStrings {
    pub fn parse() -> Result<Self> {
        let map: BTreeMap<String, String> =
            serde_json::from_str(PORT_STRINGS).context("the port's strings (strings/eng_us.json)")?;
        let mut texts = BTreeMap::new();
        for (key, text) in map {
            let id = ContentId::new("ava", Kind::String, &key.to_ascii_lowercase())
                .with_context(|| format!("the port's string key {key:?}"))?;
            texts.insert(id, (key, text));
        }
        Ok(PortStrings { texts })
    }
}

impl Source for PortStrings {
    fn name(&self) -> &str {
        "ava"
    }

    fn has(&self, id: &ContentId) -> bool {
        self.texts.contains_key(id)
    }

    fn load(&self, id: &ContentId) -> Result<Asset, LoadError> {
        let (_, t) = self.texts.get(id).ok_or_else(|| LoadError::Missing(id.clone()))?;
        Ok(Asset::Value(Arc::new(Lang::ALL.into_iter().map(|l| (l, t.clone())).collect::<BTreeMap<_, _>>())))
    }

    fn list(&self, kind: Kind) -> Vec<ContentId> {
        if kind == Kind::String { self.texts.keys().cloned().collect() } else { Vec::new() }
    }
}
