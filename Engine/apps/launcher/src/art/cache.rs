use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::Slot;
use super::image::Picture;

const FORMAT: u32 = 1;

#[derive(Debug, Default, Serialize, Deserialize)]
struct Index {
    format: u32,
    key: String,
    slots: BTreeMap<String, String>,
}

pub struct Cache {
    dir: PathBuf,
    index: Index,
}

pub fn data_key(game_data: &Path) -> String {
    let marker = ssx_assets::setup::read_marker(game_data);
    let print = marker.as_ref().and_then(|m| m["fingerprint"].as_str()).map(str::to_owned).unwrap_or_else(|| {
        let file = ssx_assets::setup::big_root(game_data).join("bootcache/data/common/fe/views/bootlogo_i6.xpr");
        let meta = std::fs::metadata(file).ok();
        let time = meta.as_ref().and_then(|m| m.modified().ok());
        let secs = time.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
        format!("tree-{}-{secs}", meta.map_or(0, |m| m.len()))
    });
    format!("{}|{print}", game_data.display())
}

impl Cache {
    pub fn open(app_dir: &Path, key: &str) -> Self {
        let dir = app_dir.join("LauncherCache");
        let stored: Option<Index> =
            std::fs::read_to_string(dir.join("art.json")).ok().and_then(|s| serde_json::from_str(&s).ok());
        let index = stored.filter(|i| i.format == FORMAT && i.key == key).unwrap_or_else(|| Index {
            format: FORMAT,
            key: key.to_owned(),
            slots: BTreeMap::new(),
        });
        Self { dir, index }
    }

    fn file(&self, slot: Slot) -> PathBuf {
        self.dir.join(format!("{}.png", slot.name()))
    }

    pub fn load(&self, slot: Slot) -> Option<(Picture, String)> {
        let note = self.index.slots.get(slot.name())?;
        Some((Picture::read_png(&self.file(slot)).ok()?, note.clone()))
    }

    pub fn store(&mut self, slot: Slot, picture: &Picture, note: &str) -> Result<()> {
        ssx_files::create_dir_all(&self.dir)?;
        picture.write_png(&self.file(slot))?;
        self.index.slots.insert(slot.name().to_owned(), note.to_owned());
        let index = serde_json::to_string_pretty(&self.index)?;
        Ok(ssx_files::replace(&self.dir.join("art.json"), index.as_bytes(), &ssx_files::Options::new())?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cache_belongs_to_one_data_folder() {
        let dir = std::env::temp_dir().join(format!("avalanche-cache-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let picture = Picture::new(2, 1, vec![1, 2, 3, 255, 4, 5, 6, 128]).unwrap();
        let mut cache = Cache::open(&dir, "a");
        assert!(cache.load(Slot::Hero).is_none());
        cache.store(Slot::Hero, &picture, "from a").unwrap();
        let again = Cache::open(&dir, "a");
        assert_eq!(again.load(Slot::Hero), Some((picture, "from a".to_owned())));
        assert!(again.load(Slot::Logo).is_none());
        assert!(Cache::open(&dir, "b").load(Slot::Hero).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
