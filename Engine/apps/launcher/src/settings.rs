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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub player_name: Option<String>,
    #[serde(deserialize_with = "masters_leniently")]
    pub masters: Vec<String>,
    #[serde(deserialize_with = "host_leniently", skip_serializing_if = "host_is_default")]
    pub host: crate::host::HostSettings,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<serde_json::Value>,
}

fn host_is_default(h: &crate::host::HostSettings) -> bool {
    *h == crate::host::HostSettings::default()
}

fn host_leniently<'de, D: serde::Deserializer<'de>>(d: D) -> Result<crate::host::HostSettings, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}

fn masters_leniently<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(match value {
        serde_json::Value::Array(list) => {
            list.into_iter().filter_map(|v| v.as_str().map(str::to_string)).take(crate::masters::MAX_MASTERS).collect()
        }
        _ => Vec::new(),
    })
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
            player_name: None,
            masters: vec![ssx_core::OFFICIAL_MASTER.into()],
            resources: None,
            host: crate::host::HostSettings::default(),
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
        write_synced(&Self::path(), serde_json::to_string_pretty(self)?.as_bytes())
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

fn write_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    ssx_files::create_dir_all(dir).map_err(ssx_files::Error::into_io)?;
    ssx_files::replace(path, bytes, &ssx_files::Options::new()).map_err(ssx_files::Error::into_io)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_settings_are_written_whole_or_not_at_all() {
        let dir = std::env::temp_dir().join(format!("launcher-settings-write-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("settings.json");
        write_synced(&path, b"{\"vsync\": true}").unwrap();
        write_synced(&path, b"{\"vsync\": false}").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"{\"vsync\": false}");
        assert!(!dir.join("settings.json.partial").exists());
        let blocked = dir.join("blocked.json");
        std::fs::create_dir_all(blocked.join("inside")).unwrap();
        assert!(write_synced(&blocked, b"{}").is_err());
        assert!(blocked.join("inside").is_dir(), "the folder in the way is untouched");
        assert!(!dir.join("blocked.json.partial").exists(), "the partial file is removed");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_games_player_name_survives_the_launchers_save() {
        let s: Settings = serde_json::from_str(r#"{"vsync": false, "player_name": "Elise", "language": "x"}"#).unwrap();
        assert_eq!(s.player_name.as_deref(), Some("Elise"));
        let back = serde_json::to_string(&s).unwrap();
        assert!(back.contains(r#""player_name":"Elise""#), "{back}");
        let none = serde_json::to_string(&Settings::default()).unwrap();
        assert!(!none.contains("player_name"), "no member written when the game set none");
    }

    #[test]
    fn the_games_resource_limits_survive_the_launchers_save() {
        let text = r#"{"vsync": false, "resources": {"download_limit_mib": 512, "server_scripts": false}}"#;
        let s: Settings = serde_json::from_str(text).unwrap();
        let back = serde_json::to_string(&s).unwrap();
        assert!(back.contains(r#""resources":{"download_limit_mib":512,"server_scripts":false}"#), "{back}");
        assert!(!serde_json::to_string(&Settings::default()).unwrap().contains("resources"));
    }

    #[test]
    fn the_master_list_survives_and_a_wrong_one_spoils_nothing_else() {
        let s: Settings = serde_json::from_str(r#"{"vsync": false, "masters": ["https://m.example.org"]}"#).unwrap();
        assert_eq!(s.masters, ["https://m.example.org"]);
        assert!(serde_json::to_string(&s).unwrap().contains(r#""masters":["https://m.example.org"]"#));
        let wrong: Settings = serde_json::from_str(r#"{"vsync": false, "masters": "https://m.example.org"}"#).unwrap();
        assert!(!wrong.vsync && wrong.masters.is_empty(), "the rest of the file kept");
        let mixed: Settings = serde_json::from_str(r#"{"masters": [1, "https://a.example.org", null]}"#).unwrap();
        assert_eq!(mixed.masters, ["https://a.example.org"]);
        let preset = Settings::default();
        assert_eq!(preset.masters, [ssx_core::OFFICIAL_MASTER]);
        assert_eq!(crate::masters::canonical(ssx_core::OFFICIAL_MASTER).as_deref(), Ok(ssx_core::OFFICIAL_MASTER));
        let old: Settings = serde_json::from_str(r#"{"vsync": false}"#).unwrap();
        assert_eq!(old.masters, [ssx_core::OFFICIAL_MASTER], "a file without the member gets the preset");
        let emptied = Settings { masters: Vec::new(), ..Settings::default() };
        let text = serde_json::to_string(&emptied).unwrap();
        assert!(text.contains(r#""masters":[]"#), "{text}");
        assert!(serde_json::from_str::<Settings>(&text).unwrap().masters.is_empty(), "an emptied list stays empty");
    }
}
