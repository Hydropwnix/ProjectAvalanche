use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub fn app_dir() -> PathBuf {
    ssx_assets::setup::app_dir()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    #[default]
    Viewer,
    Ride,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Shading {
    #[default]
    Auto,
    Textured,
    Height,
    Material,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Music {
    #[default]
    Auto,
    DeadlyDescent,
    Licensed,
    Off,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub iso: Option<PathBuf>,
    pub game_data: Option<PathBuf>,
    pub mode: Mode,
    pub track: String,
    pub drop: usize,
    pub lighting: Option<String>,
    pub character: String,
    pub subdivide: u32,
    pub shading: Shading,
    pub objects: bool,
    pub hud: bool,
    pub vsync: bool,
    pub wireframe: bool,
    pub music: Music,
    pub music_volume: f32,
    pub sfx: bool,
    pub sfx_volume: f32,
    pub particles: bool,
    pub snow_tracks: bool,
    pub local_lights: bool,
    pub game_post: bool,
    pub game_sky: bool,
    pub boost_meter: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            iso: None,
            game_data: None,
            mode: Mode::Viewer,
            track: "alps0101".into(),
            drop: 0,
            lighting: None,
            character: ssx_assets::character::DEFAULT_CHARACTER.into(),
            subdivide: ssx_assets::subdiv::GAME_LEVELS as u32,
            shading: Shading::Auto,
            objects: true,
            hud: true,
            vsync: true,
            wireframe: false,
            music: Music::Auto,
            music_volume: 1.0,
            sfx: true,
            sfx_volume: 1.0,
            particles: true,
            snow_tracks: true,
            local_lights: true,
            game_post: true,
            game_sky: true,
            boost_meter: 0.0,
        }
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        app_dir().join("settings.json")
    }

    pub fn load() -> Self {
        std::fs::read_to_string(Self::path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::path();
        std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))?;
        let tmp = path.with_extension("json.partial");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        std::fs::rename(tmp, path)
    }

    pub fn data_ready(&self) -> bool {
        let gd = self.game_data();
        let tree = gd.join("big/cache1/data/xenon/attribdb").is_dir() && gd.join("big/data0").is_dir();
        ssx_assets::setup::is_complete(&gd) || (tree && !gd.join(ssx_assets::setup::MARKER).exists())
    }

    pub fn game_data(&self) -> PathBuf {
        self.game_data.clone().unwrap_or_else(ssx_assets::setup::default_data_dir)
    }

    pub fn big_root(&self) -> PathBuf {
        ssx_assets::setup::big_root(&self.game_data())
    }
}
