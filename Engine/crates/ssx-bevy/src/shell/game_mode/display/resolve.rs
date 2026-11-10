use bevy::prelude::{Entity, UVec2};
use bevy::window::{MonitorSelection, VideoModeSelection, WindowMode as BevyMode, WindowPosition};
use ssx_assets::display::{Backend, DisplaySettings, VideoMode, WindowMode};

pub struct Screen<'a> {
    pub entity: Entity,
    pub name: Option<&'a str>,
    pub primary: bool,
    pub size: UVec2,
    pub modes: &'a [bevy::window::VideoMode],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    pub mode: BevyMode,
    pub position: Option<WindowPosition>,
    pub monitor: Option<Entity>,
    pub effective: WindowMode,
    pub video: Option<VideoMode>,
    pub notes: Vec<String>,
}

pub fn shown(name: &str) -> String {
    let mut s: String = name.chars().take(ssx_assets::display::MAX_NAME).flat_map(char::escape_debug).collect();
    if name.chars().count() > ssx_assets::display::MAX_NAME {
        s.push('…');
    }
    format!("\"{s}\"")
}

pub fn resolve(s: &DisplaySettings, screens: &[Screen], backend: Option<Backend>) -> Resolved {
    let mut notes = Vec::new();
    let primary = || screens.iter().find(|m| m.primary).or_else(|| screens.first());
    let screen = match s.monitor.as_deref() {
        Some(name) => match screens.iter().find(|m| m.name == Some(name)) {
            Some(m) => Some(m),
            None => {
                let instead = primary();
                if !screens.is_empty() {
                    notes.push(format!(
                        "the monitor {} is not connected: the primary monitor{} instead",
                        shown(name),
                        instead.and_then(|m| m.name).map(|n| format!(" {}", shown(n))).unwrap_or_default()
                    ));
                }
                instead
            }
        },
        None => primary(),
    };
    let selection = screen.map_or(MonitorSelection::Primary, |m| MonitorSelection::Entity(m.entity));
    let mut effective = s.mode;
    if effective == WindowMode::Exclusive && backend.is_some_and(|b| !b.exclusive()) {
        notes.push(format!(
            "exclusive fullscreen is not available on {}: borderless fullscreen instead (the desktop's resolution)",
            backend.map_or("this system", |b| b.name())
        ));
        effective = WindowMode::Borderless;
    }
    let mut video = None;
    let (mode, position) = match effective {
        WindowMode::Windowed => {
            let at = s.monitor.as_ref().map(|_| WindowPosition::Centered(selection));
            (BevyMode::Windowed, at)
        }
        WindowMode::Borderless => (BevyMode::BorderlessFullscreen(selection), None),
        WindowMode::Exclusive => {
            let wanted = s.video_mode;
            let found = wanted.and_then(|v| {
                let modes = screen?.modes.iter().filter(|m| {
                    m.physical_size == UVec2::new(v.width, v.height) && m.refresh_rate_millihertz == v.refresh_mhz
                });
                modes.max_by_key(|m| m.bit_depth).copied()
            });
            let pick = match (wanted, found) {
                (Some(_), Some(m)) => {
                    video = wanted;
                    VideoModeSelection::Specific(m)
                }
                (Some(v), None) => {
                    notes.push(format!(
                        "the video mode {v} is not one of the monitor's{}: its current mode instead",
                        screen.and_then(|m| m.name).map(|n| format!(" ({})", shown(n))).unwrap_or_default()
                    ));
                    VideoModeSelection::Current
                }
                (None, _) => VideoModeSelection::Current,
            };
            (BevyMode::Fullscreen(selection, pick), None)
        }
    };
    Resolved { mode, position, monitor: screen.map(|m| m.entity), effective, video, notes }
}
