use eframe::egui::{self, Ui};

use crate::app::Launcher;
use crate::catalog;
use crate::command;
use crate::settings::Mode;
use crate::ui::theme;
use crate::ui::widgets::{self, Kind};

const MAX_BOOST: f32 = 150.0;
const WIDE: f32 = 380.0;

impl Launcher {
    pub fn tools_section(&mut self, ui: &mut Ui) {
        if !self.data_ready || self.tracks.is_empty() {
            widgets::status(ui, theme::WARN, "The tools need the game data: set it up under Settings, Game data.");
            if widgets::button(ui, "Open the setup", Kind::Plain, true).clicked() {
                self.go_to_setup();
            }
            return;
        }
        widgets::paragraph(
            ui,
            "Not the game: one track without the front end, to look at the data or to test the ride.",
        );
        ui.add_space(4.0);
        egui::Grid::new("tools").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
            widgets::field(ui, "Run");
            ui.horizontal(|ui| {
                ui.radio_value(&mut self.settings.mode, Mode::Viewer, "Asset viewer (fly camera)");
                ui.radio_value(&mut self.settings.mode, Mode::Ride, "Test ride (rider and physics)");
            });
            ui.end_row();
            self.track_rows(ui);
            widgets::field(ui, "Rider");
            match &self.riders {
                Ok(riders) if !riders.is_empty() => {
                    let current = riders.iter().find(|r| r.name == self.settings.character);
                    let shown = current.map_or(self.settings.character.clone(), |r| r.label.clone());
                    widgets::combo(ui, egui::ComboBox::from_id_salt("rider").selected_text(shown), |ui| {
                        for r in riders {
                            ui.selectable_value(&mut self.settings.character, r.name.clone(), &r.label);
                        }
                    });
                }
                Ok(_) => _ = ui.text_edit_singleline(&mut self.settings.character),
                Err(e) => widgets::status(ui, theme::ERROR, format!("riders unavailable: {e}")),
            }
            ui.end_row();
            widgets::field(ui, "Debug text");
            ui.checkbox(&mut self.settings.hud, "HUD text of the viewer");
            ui.end_row();
            widgets::field(ui, "Test ride");
            ui.add_enabled_ui(self.settings.mode == Mode::Ride, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Boost meter at the start");
                    ui.add(egui::Slider::new(&mut self.settings.boost_meter, 0.0..=MAX_BOOST)).on_hover_text(
                        "Test option (--ride-boost-meter): the game starts with an empty meter and fills it from \
                         landed tricks.",
                    );
                });
            });
            ui.end_row();
        });
        ui.add_space(8.0);
        let what = if self.settings.mode == Mode::Ride { "the test ride" } else { "the asset viewer" };
        ui.horizontal(|ui| {
            let running = self.running();
            let launch = widgets::button(ui, "Launch", Kind::Primary, !running)
                .on_hover_text(format!("Starts {what} on the track chosen above."));
            if launch.clicked() {
                let inv = command::viewer(&self.settings, &self.settings.big_root());
                self.start(ui.ctx(), &inv, false);
            }
            self.started_line(ui);
        });
    }

    pub fn started_line(&self, ui: &mut Ui) {
        let name = if self.started_game { "Game" } else { "Viewer" };
        if let Some(e) = &self.launch_error {
            widgets::status(ui, theme::ERROR, e.clone());
        } else if let Some((pid, status)) = self.viewer.as_ref().map(|v| (v.pid, v.status())) {
            match status {
                None => widgets::status(ui, theme::CYAN_SOFT, format!("{name} running (process {pid}).")),
                Some(s) if s == "exited normally" => widgets::status(ui, theme::GOOD, format!("{name} {s}.")),
                Some(s) => widgets::status(ui, theme::ERROR, format!("{name} {s}. See Output.")),
            }
        } else {
            widgets::note(ui, "Starts in its own window; this one stays open.");
        }
    }

    fn track_rows(&mut self, ui: &mut Ui) {
        widgets::field(ui, "Track");
        let big = self.settings.big_root();
        ui.horizontal(|ui| {
            let combo = egui::ComboBox::from_id_salt("track").selected_text(&self.settings.track).height(320.0);
            widgets::combo(ui, combo, |ui| {
                for t in &self.tracks {
                    if ui.selectable_value(&mut self.settings.track, t.clone(), t).changed() {
                        self.settings.drop = 0;
                        self.settings.lighting = None;
                    }
                }
            });
            if let Ok(drops) = self.drops.get(&big, &self.settings.track) {
                let peak = drops.first().map_or("", |d| d.peak.as_str());
                widgets::note(ui, &format!("{peak}  ·  {} drop(s)", drops.len()));
            }
        });
        ui.end_row();
        let drops = match self.drops.get(&big, &self.settings.track) {
            Ok(d) => d.clone(),
            Err(e) => {
                widgets::field(ui, "Drop");
                widgets::status(ui, theme::ERROR, format!("drops unavailable: {e}"));
                ui.end_row();
                return;
            }
        };
        if self.settings.drop > drops.len() {
            self.settings.drop = 0;
        }
        if self.settings.lighting.as_ref().is_some_and(|l| !drops.iter().any(|d| &d.lighting == l)) {
            self.settings.lighting = None;
        }
        let name = |i: usize| drops.get(i).map_or("-".to_string(), |d| format!("{}  ·  {}", d.text_id, d.lighting));
        widgets::field(ui, "Drop");
        ui.add_enabled_ui(self.settings.lighting.is_none(), |ui| {
            let shown = if self.settings.drop == 0 {
                format!("Default: 1. {}", name(0))
            } else {
                format!("{}. {}", self.settings.drop, name(self.settings.drop - 1))
            };
            widgets::combo(ui, egui::ComboBox::from_id_salt("drop").selected_text(shown).width(WIDE), |ui| {
                ui.selectable_value(&mut self.settings.drop, 0, "Default (the first drop)");
                for i in 0..drops.len() {
                    ui.selectable_value(&mut self.settings.drop, i + 1, format!("{}. {}", i + 1, name(i)));
                }
            });
        });
        ui.end_row();
        widgets::field(ui, "Lighting");
        ui.horizontal(|ui| {
            let presets = catalog::presets(&drops);
            let shown = self.settings.lighting.clone().unwrap_or_else(|| "The drop's own".into());
            widgets::combo(ui, egui::ComboBox::from_id_salt("lighting").selected_text(shown), |ui| {
                ui.selectable_value(&mut self.settings.lighting, None, "The drop's own");
                for p in presets {
                    ui.selectable_value(&mut self.settings.lighting, Some(p.clone()), p);
                }
            });
            if self.settings.lighting.is_some() {
                widgets::note(ui, "uses the first drop lit by this preset");
            }
        });
        ui.end_row();
    }
}
