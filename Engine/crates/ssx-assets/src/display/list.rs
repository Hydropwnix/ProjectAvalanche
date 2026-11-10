use serde_json::{Map, Value as Json};

use super::{REFRESH_MHZ, VideoMode, json_u32, monitor_name_valid};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Windows,
    X11,
    Wayland,
    MacOs,
    Other,
}

impl Backend {
    const ALL: [Backend; 5] = [Backend::Windows, Backend::X11, Backend::Wayland, Backend::MacOs, Backend::Other];

    pub fn name(self) -> &'static str {
        match self {
            Backend::Windows => "windows",
            Backend::X11 => "x11",
            Backend::Wayland => "wayland",
            Backend::MacOs => "macos",
            Backend::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Option<Backend> {
        Backend::ALL.into_iter().find(|b| b.name() == s)
    }

    pub fn exclusive(self) -> bool {
        matches!(self, Backend::Windows | Backend::X11 | Backend::MacOs)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MonitorInfo {
    pub name: Option<String>,
    pub position: (i32, i32),
    pub size: (u32, u32),
    pub scale_permille: u32,
    pub refresh_mhz: Option<u32>,
    pub primary: bool,
    pub modes: Vec<VideoMode>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DisplayList {
    pub backend: Backend,
    pub monitors: Vec<MonitorInfo>,
}

pub const MAX_LIST_BYTES: usize = 1024 * 1024;
pub const MAX_MONITORS: usize = 16;
pub const MAX_MODES: usize = 1024;
pub const SCALE_PERMILLE: std::ops::RangeInclusive<u32> = 250..=8_000;

pub fn scale_permille(factor: f64) -> u32 {
    let p = (factor * 1000.0).round();
    if p.is_nan() {
        return 1000;
    }
    p.clamp(f64::from(*SCALE_PERMILLE.start()), f64::from(*SCALE_PERMILLE.end())) as u32
}

impl DisplayList {
    pub fn to_json(&self) -> String {
        let monitors: Vec<Json> = self
            .monitors
            .iter()
            .map(|m| {
                let mut o = Map::new();
                o.insert("name".into(), m.name.clone().map_or(Json::Null, Json::String));
                o.insert("x".into(), m.position.0.into());
                o.insert("y".into(), m.position.1.into());
                o.insert("width".into(), m.size.0.into());
                o.insert("height".into(), m.size.1.into());
                o.insert("scale_permille".into(), m.scale_permille.into());
                o.insert("refresh_mhz".into(), m.refresh_mhz.map_or(Json::Null, Json::from));
                o.insert("primary".into(), m.primary.into());
                o.insert("modes".into(), Json::Array(m.modes.iter().map(|v| v.to_json()).collect()));
                Json::Object(o)
            })
            .collect();
        let mut o = Map::new();
        o.insert("backend".into(), self.backend.name().into());
        o.insert("monitors".into(), Json::Array(monitors));
        printable_ascii(&Json::Object(o).to_string())
    }

    pub fn parse(text: &str) -> Result<DisplayList, String> {
        if text.len() > MAX_LIST_BYTES {
            return Err(format!("{} bytes, at most {MAX_LIST_BYTES}", text.len()));
        }
        let v: Json = serde_json::from_str(text.trim()).map_err(|e| format!("not JSON: {e}"))?;
        let o = v.as_object().ok_or("not an object")?;
        let backend = o.get("backend").and_then(Json::as_str).and_then(Backend::parse).ok_or("no backend")?;
        let list = o.get("monitors").and_then(Json::as_array).ok_or("no monitors")?;
        if list.len() > MAX_MONITORS {
            return Err(format!("{} monitors, at most {MAX_MONITORS}", list.len()));
        }
        let mut monitors = Vec::new();
        for (i, m) in list.iter().enumerate() {
            monitors.push(MonitorInfo::from_json(m).ok_or_else(|| format!("monitor {i} does not read"))?);
        }
        Ok(DisplayList { backend, monitors })
    }

    pub fn find(&self, name: Option<&str>) -> Option<&MonitorInfo> {
        match name {
            Some(n) => self.monitors.iter().find(|m| m.name.as_deref() == Some(n)),
            None => self.monitors.iter().find(|m| m.primary).or_else(|| self.monitors.first()),
        }
    }
}

fn printable_ascii(json: &str) -> String {
    let mut out = String::with_capacity(json.len());
    for c in json.chars() {
        if (' '..='~').contains(&c) {
            out.push(c);
        } else {
            for unit in c.encode_utf16(&mut [0; 2]) {
                out.push_str(&format!("\\u{unit:04x}"));
            }
        }
    }
    out
}

impl MonitorInfo {
    fn from_json(v: &Json) -> Option<MonitorInfo> {
        let o = v.as_object()?;
        let name = match o.get("name") {
            None | Some(Json::Null) => None,
            Some(n) => Some(n.as_str().filter(|n| monitor_name_valid(n))?.to_string()),
        };
        let int = |k: &str| o.get(k)?.as_i64().and_then(|n| i32::try_from(n).ok());
        let side = |k: &str| o.get(k).and_then(json_u32).filter(|n| (1..=65_535).contains(n));
        let scale_permille = o.get("scale_permille").and_then(json_u32).filter(|s| SCALE_PERMILLE.contains(s))?;
        let refresh_mhz = match o.get("refresh_mhz") {
            None | Some(Json::Null) => None,
            Some(r) => Some(json_u32(r).filter(|r| REFRESH_MHZ.contains(r))?),
        };
        let list = o.get("modes")?.as_array()?;
        if list.len() > MAX_MODES {
            return None;
        }
        let mut modes: Vec<VideoMode> = list.iter().filter_map(VideoMode::from_json).collect();
        modes.sort_by(|a, b| b.cmp(a));
        modes.dedup();
        Some(MonitorInfo {
            name,
            position: (int("x")?, int("y")?),
            size: (side("width")?, side("height")?),
            scale_permille,
            refresh_mhz,
            primary: o.get("primary")?.as_bool()?,
            modes,
        })
    }

    pub fn label(&self) -> String {
        let hz = self.refresh_mhz.map(|r| format!(", {} Hz", VideoMode { width: 1, height: 1, refresh_mhz: r }.hz()));
        format!(
            "{} — {} × {}{}{}",
            self.name.as_deref().unwrap_or("(unnamed)"),
            self.size.0,
            self.size.1,
            hz.unwrap_or_default(),
            if self.primary { " (primary)" } else { "" }
        )
    }
}
