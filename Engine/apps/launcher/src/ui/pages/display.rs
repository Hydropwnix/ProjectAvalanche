use eframe::egui::{self, Ui};
use ssx_assets::display::{Backend, DEFAULT_WINDOW, DisplayList, MonitorInfo, VideoMode, WindowMode};

use crate::app::Launcher;
use crate::ui::{theme, widgets};

fn mode_label(m: WindowMode) -> &'static str {
    match m {
        WindowMode::Windowed => "Window",
        WindowMode::Borderless => "Borderless fullscreen",
        WindowMode::Exclusive => "Exclusive fullscreen",
    }
}

fn video_label(v: &VideoMode) -> String {
    format!("{} × {} at {} Hz", v.width, v.height, v.hz())
}

fn no_exclusive(backend: Backend) -> String {
    format!(
        "Exclusive fullscreen is not available on {} (the compositor keeps the display's mode): it starts as \
         borderless fullscreen, at the desktop's resolution.",
        backend.name()
    )
}

impl Launcher {
    pub fn display_section(&mut self, ui: &mut Ui) {
        self.displays.ask(ui.ctx(), false);
        let list = self.displays.list.as_ref().and_then(|l| l.as_ref().ok()).cloned();
        let exclusive = list.as_ref().is_none_or(|l| l.backend.exclusive());
        let s = &mut self.settings.display;
        widgets::heading(ui, "Window");
        egui::Grid::new("display").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
            widgets::field(ui, "Mode");
            let shown = match s.mode {
                WindowMode::Exclusive if !exclusive => "Exclusive fullscreen (borderless here)",
                m => mode_label(m),
            };
            let combo = egui::ComboBox::from_id_salt("window-mode").selected_text(shown).width(330.0);
            widgets::combo(ui, combo, |ui| {
                for m in WindowMode::ALL {
                    let enabled = m != WindowMode::Exclusive || exclusive;
                    if ui.add_enabled(enabled, egui::Button::selectable(s.mode == m, mode_label(m))).clicked() {
                        *s = s.with_mode(m);
                    }
                }
            });
            ui.end_row();
            widgets::field(ui, "Monitor");
            monitor_combo(ui, list.as_ref(), &mut s.monitor);
            ui.end_row();
            widgets::field(ui, "Resolution");
            let monitor = list.as_ref().and_then(|l| l.find(s.monitor.as_deref()));
            ui.add_enabled_ui(s.fullscreen == WindowMode::Exclusive && exclusive, |ui| {
                video_combo(ui, monitor, &mut s.video_mode);
            });
            ui.end_row();
            widgets::field(ui, "Window size");
            ui.horizontal(|ui| {
                window_size_combo(ui, &mut s.window);
                if widgets::button(ui, "Reset", widgets::Kind::Plain, s.window.is_some()).clicked() {
                    s.window = None;
                }
            });
            ui.end_row();
            widgets::field(ui, "Render resolution");
            render_resolution_combo(ui, &mut s.render_height);
            ui.end_row();
        });
        ui.add_space(8.0);
        widgets::note(
            ui,
            "Alt+Enter in the game switches between the window and the fullscreen mode chosen here. The game keeps the \
             mode you leave it in, the window's size and the render resolution.",
        );
        widgets::note(
            ui,
            "Render resolution is the height the game draws every frame at, in all modes, scaled to the window: a 4K \
             monitor can run at 1080p. The window's own resolution is the sharpest.",
        );
        if !exclusive && let Some(l) = &list {
            widgets::note(ui, &no_exclusive(l.backend));
        }
        if s.fullscreen == WindowMode::Exclusive && exclusive {
            widgets::note(
                ui,
                "Exclusive fullscreen sets the monitor's mode while the game has the focus; switching away (Alt+Tab) \
                 gives the desktop its mode back and minimises the game.",
            );
        }
        for note in &self.settings.display_notes {
            widgets::note(ui, &widgets::shown(note));
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let asking = self.displays.asking();
            if widgets::button(ui, "Find monitors again", widgets::Kind::Plain, !asking).clicked() {
                self.displays.ask(ui.ctx(), true);
            }
            match (&self.displays.list, asking) {
                (_, true) => widgets::status(ui, theme::CYAN_SOFT, "Asking the game for the monitors…"),
                (Some(Err(e)), false) => widgets::status(
                    ui,
                    theme::ERROR,
                    format!("Monitors not listed: {} (the game uses the primary one)", widgets::shown(e)),
                ),
                (Some(Ok(l)), false) => widgets::status(
                    ui,
                    theme::TEXT_SOFT,
                    format!("{} monitor(s) on {}", l.monitors.len(), l.backend.name()),
                ),
                (None, false) => {}
            }
        });
    }
}

const WINDOW_SIZES: [(u32, u32); 7] =
    [(3840, 2160), (2560, 1440), (1920, 1080), (1600, 900), (1280, 720), (1024, 768), (960, 540)];

const RENDER_HEIGHTS: [u32; 6] = [2160, 1440, 1080, 900, 720, 540];

fn window_size_combo(ui: &mut Ui, size: &mut Option<(u32, u32)>) {
    let default = format!("The game's ({} × {})", DEFAULT_WINDOW.0, DEFAULT_WINDOW.1);
    let shown = match *size {
        None => default.clone(),
        Some((w, h)) => format!("{w} × {h}"),
    };
    let combo = egui::ComboBox::from_id_salt("window-size").selected_text(shown).width(330.0);
    widgets::combo(ui, combo, |ui| {
        ui.selectable_value(size, None, default);
        for (w, h) in WINDOW_SIZES {
            ui.selectable_value(size, Some((w, h)), format!("{w} × {h}"));
        }
        if let Some((w, h)) = *size
            && !WINDOW_SIZES.contains(&(w, h))
        {
            ui.selectable_value(size, Some((w, h)), format!("{w} × {h} (as you left it)"));
        }
    });
}

fn render_resolution_combo(ui: &mut Ui, height: &mut Option<u32>) {
    let shown = match *height {
        None => "The window's".to_string(),
        Some(h) => format!("{h}p"),
    };
    let combo = egui::ComboBox::from_id_salt("render-height").selected_text(shown).width(330.0);
    widgets::combo(ui, combo, |ui| {
        ui.selectable_value(height, None, "The window's");
        for h in RENDER_HEIGHTS {
            ui.selectable_value(height, Some(h), format!("{h}p"));
        }
        if let Some(h) = *height
            && !RENDER_HEIGHTS.contains(&h)
        {
            ui.selectable_value(height, Some(h), format!("{h}p (as you left it)"));
        }
    });
}

fn monitor_combo(ui: &mut Ui, list: Option<&DisplayList>, monitor: &mut Option<String>) {
    let listed = |name: &str| list.is_some_and(|l| l.monitors.iter().any(|m| m.name.as_deref() == Some(name)));
    let shown = match monitor.as_deref() {
        None => "The primary monitor".to_string(),
        Some(name) => match list.and_then(|l| l.monitors.iter().find(|m| m.name.as_deref() == Some(name))) {
            Some(m) => widgets::shown(&m.label()),
            None => format!("{} (not connected)", widgets::shown(name)),
        },
    };
    let combo = egui::ComboBox::from_id_salt("monitor").selected_text(shown).width(330.0);
    widgets::combo(ui, combo, |ui| {
        ui.selectable_value(monitor, None, "The primary monitor");
        for m in list.map(|l| l.monitors.as_slice()).unwrap_or_default() {
            let Some(name) = &m.name else { continue };
            ui.selectable_value(monitor, Some(name.clone()), widgets::shown(&m.label()));
        }
        if let Some(name) = monitor.clone().filter(|n| !listed(n)) {
            ui.selectable_value(monitor, Some(name.clone()), format!("{} (not connected)", widgets::shown(&name)));
        }
    });
}

fn video_combo(ui: &mut Ui, monitor: Option<&MonitorInfo>, video: &mut Option<VideoMode>) {
    let listed = |v: &VideoMode| monitor.is_some_and(|m| m.modes.contains(v));
    let shown = match video {
        None => "The monitor's current mode".to_string(),
        Some(v) if listed(v) => video_label(v),
        Some(v) => format!("{} (not listed: the current one)", video_label(v)),
    };
    let combo = egui::ComboBox::from_id_salt("video-mode").selected_text(shown).width(330.0);
    widgets::combo(ui, combo, |ui| {
        ui.selectable_value(video, None, "The monitor's current mode");
        for v in monitor.map(|m| m.modes.as_slice()).unwrap_or_default() {
            ui.selectable_value(video, Some(*v), video_label(v));
        }
    });
}
