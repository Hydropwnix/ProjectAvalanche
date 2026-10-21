mod cache;
mod disc;
mod gallery;
pub mod image;
mod loader;
mod noise;
mod procedural;
mod render;

use std::path::{Path, PathBuf};

pub use gallery::{Art, Gallery};
use image::Picture;
pub use loader::load;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Background,
    Hero,
    TileMods,
    TileSettings,
    TileExtras,
    Logo,
}

impl Slot {
    pub const ALL: [Self; 6] =
        [Self::Background, Self::Hero, Self::TileMods, Self::TileSettings, Self::TileExtras, Self::Logo];

    pub fn name(self) -> &'static str {
        match self {
            Self::Background => "background",
            Self::Hero => "hero",
            Self::TileMods => "tile_mods",
            Self::TileSettings => "tile_settings",
            Self::TileExtras => "tile_extras",
            Self::Logo => "logo",
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }

    pub fn stored(self) -> (usize, usize) {
        match self {
            Self::Background => (1920, 1080),
            Self::Hero => (1792, 576),
            Self::TileMods | Self::TileSettings | Self::TileExtras => (576, 368),
            Self::Logo => (300, 192),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    User,
    Disc,
    BuiltIn,
}

pub fn resolve(slot: Slot, user: bool, disc: bool) -> Option<Source> {
    match (user, disc, slot) {
        (true, _, _) => Some(Source::User),
        (false, true, _) => Some(Source::Disc),
        (false, false, Slot::Logo) => None,
        (false, false, _) => Some(Source::BuiltIn),
    }
}

pub struct Loaded {
    pub slot: Slot,
    pub source: Source,
    pub picture: Picture,
    pub note: String,
}

#[derive(Clone, Debug, Default)]
pub struct Request {
    pub app_dir: PathBuf,
    pub game_data: Option<PathBuf>,
}

fn fitted(slot: Slot, picture: Picture) -> Picture {
    let (max_w, max_h) = slot.stored();
    let scale = (max_w as f64 / picture.width as f64).min(max_h as f64 / picture.height as f64);
    if scale >= 1.0 {
        return picture;
    }
    picture.resize((picture.width as f64 * scale).round() as usize, (picture.height as f64 * scale).round() as usize)
}

fn user_file(app_dir: &Path, slot: Slot) -> PathBuf {
    app_dir.join("LauncherArt").join(format!("{}.png", slot.name()))
}

fn built_in(slot: Slot) -> Option<Picture> {
    let scene = |s: &procedural::Scene, w, h| Some(render::render(s, w, h));
    match slot {
        Slot::Background => scene(&procedural::BACKDROP, 1280, 720),
        Slot::Hero => scene(&procedural::HERO, 1344, 432),
        Slot::TileMods => scene(&procedural::TILES[0], 576, 368),
        Slot::TileSettings => scene(&procedural::TILES[1], 576, 368),
        Slot::TileExtras => scene(&procedural::TILES[2], 576, 368),
        Slot::Logo => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slot_takes_the_first_source_that_has_a_picture() {
        for slot in Slot::ALL {
            assert_eq!(resolve(slot, true, true), Some(Source::User));
            assert_eq!(resolve(slot, true, false), Some(Source::User));
            assert_eq!(resolve(slot, false, true), Some(Source::Disc));
            let last = if slot == Slot::Logo { None } else { Some(Source::BuiltIn) };
            assert_eq!(resolve(slot, false, false), last);
            assert_eq!(built_in(slot).is_some(), last.is_some(), "{}", slot.name());
        }
        let mut names: Vec<&str> = Slot::ALL.iter().map(|s| s.name()).collect();
        names.dedup();
        assert_eq!(names.len(), 6);
        assert!(Slot::ALL.iter().enumerate().all(|(i, s)| s.index() == i));
    }

    #[test]
    fn the_loader_prefers_the_players_files() {
        let dir = std::env::temp_dir().join(format!("avalanche-art-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("LauncherArt")).unwrap();
        let request = Request { app_dir: dir.clone(), game_data: None };
        let run = || {
            let mut got = Vec::new();
            let problems = load(&request, &mut |l| got.push((l.slot, l.source, l.picture.width, l.picture.height)));
            (got, problems)
        };
        let (got, problems) = run();
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(got.len(), 5);
        assert!(got.iter().all(|g| g.1 == Source::BuiltIn && g.0 != Slot::Logo));
        let big = Picture::new(4000, 1000, vec![200; 4000 * 1000 * 4]).unwrap();
        big.write_png(&user_file(&dir, Slot::Hero)).unwrap();
        std::fs::write(user_file(&dir, Slot::Logo), b"not a picture").unwrap();
        let (got, problems) = run();
        assert_eq!(got.iter().find(|g| g.0 == Slot::Hero), Some(&(Slot::Hero, Source::User, 1792, 448)));
        assert!(got.iter().all(|g| g.0 != Slot::Logo));
        assert_eq!(got.iter().filter(|g| g.1 == Source::BuiltIn).count(), 4);
        assert_eq!(problems.len(), 1, "{problems:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pictures_scale_by_area() {
        let mut rgba = Vec::new();
        for _ in 0..2 {
            rgba.extend([0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255, 255, 255, 255, 255, 255]);
        }
        let small = Picture::new(4, 2, rgba).unwrap().resize(2, 1);
        assert_eq!(small.rgba, [0, 0, 0, 255, 255, 255, 255, 255]);
        let mixed = Picture::new(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 0]).unwrap().resize(1, 1);
        assert_eq!(mixed.rgba, [255, 0, 0, 128]);
        assert_eq!(Picture::new(3, 3, vec![7; 36]).unwrap().crop(1, 1, 5, 5).rgba.len(), 2 * 2 * 4);
    }
}
