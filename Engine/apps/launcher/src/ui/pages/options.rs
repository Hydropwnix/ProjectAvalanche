use eframe::egui::{self, RichText, Ui};

use crate::app::Launcher;
use crate::settings::{Music, Shading};
use crate::ui::theme;
use crate::ui::widgets;

fn shading_label(s: Shading) -> &'static str {
    match s {
        Shading::Auto => "Auto (textured when possible)",
        Shading::Textured => "Textured (the game's materials)",
        Shading::Height => "Height tint",
        Shading::Material => "Material debug palette",
    }
}

fn music_label(m: Music) -> &'static str {
    match m {
        Music::Auto => "Auto (the drop's Deadly Descent, else licensed)",
        Music::DeadlyDescent => "Deadly Descent",
        Music::Licensed => "Licensed tracks",
        Music::Off => "Off",
    }
}

impl Launcher {
    pub fn graphics_section(&mut self, ui: &mut Ui) {
        let s = &mut self.settings;
        ui.columns(2, |columns| {
            let ui = &mut columns[0];
            widgets::heading(ui, "Terrain");
            egui::Grid::new("terrain").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
                widgets::field(ui, "Subdivision");
                ui.add(egui::Slider::new(&mut s.subdivide, 0..=3).text("levels (2 = the game's)"));
                ui.end_row();
                widgets::field(ui, "Shading");
                egui::ComboBox::from_id_salt("shading").selected_text(shading_label(s.shading)).width(250.0).show_ui(
                    ui,
                    |ui| {
                        for v in [Shading::Auto, Shading::Textured, Shading::Height, Shading::Material] {
                            ui.selectable_value(&mut s.shading, v, shading_label(v));
                        }
                    },
                );
                ui.end_row();
            });
            ui.add_space(8.0);
            widgets::heading(ui, "Display");
            ui.checkbox(&mut s.vsync, "VSync");
            ui.checkbox(&mut s.objects, "Placed objects");
            ui.checkbox(&mut s.wireframe, "Wireframe (F)");

            let ui = &mut columns[1];
            widgets::heading(ui, "Effects");
            ui.checkbox(&mut s.particles, "Particles");
            ui.checkbox(&mut s.snow_tracks, "Snow tracks");
            ui.checkbox(&mut s.local_lights, "Local lights");
            ui.checkbox(&mut s.game_post, "Post-processing");
            ui.checkbox(&mut s.game_sky, "Sky");
            ui.add_space(8.0);
            widgets::note(ui, "The game and the tools start with these options. They are saved as you change them.");
        });
    }

    pub fn audio_section(&mut self, ui: &mut Ui) {
        let s = &mut self.settings;
        widgets::heading(ui, "Music");
        egui::Grid::new("music").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
            widgets::field(ui, "Source");
            egui::ComboBox::from_id_salt("music").selected_text(music_label(s.music)).width(330.0).show_ui(ui, |ui| {
                for v in [Music::Auto, Music::DeadlyDescent, Music::Licensed, Music::Off] {
                    ui.selectable_value(&mut s.music, v, music_label(v));
                }
            });
            ui.end_row();
            widgets::field(ui, "Volume");
            ui.add_enabled(s.music != Music::Off, egui::Slider::new(&mut s.music_volume, 0.0..=1.0).fixed_decimals(2));
            ui.end_row();
        });
        ui.add_space(8.0);
        widgets::heading(ui, "Sound effects");
        egui::Grid::new("sfx").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
            widgets::field(ui, "Effects");
            ui.checkbox(&mut s.sfx, "On");
            ui.end_row();
            widgets::field(ui, "Volume");
            ui.add_enabled(s.sfx, egui::Slider::new(&mut s.sfx_volume, 0.0..=1.0).fixed_decimals(2));
            ui.end_row();
        });
    }

    pub fn controls_section(&self, ui: &mut Ui) {
        for (what, keys) in CONTROLS {
            widgets::heading(ui, what);
            ui.label(RichText::new(keys).color(theme::TEXT_SOFT));
            ui.add_space(8.0);
        }
    }
}

const CONTROLS: [(&str, &str); 3] = [
    ("Gamepad", "An Xbox-style pad with the game's own layout (recommended: the game was made for a pad)."),
    (
        "Keys in the menus",
        "Enter = START, Space = A, Backspace = B, X, Y, arrow keys = D-pad, Q / E = LB / RB, W A S D = left stick \
         (the globe), F1 skips a movie.",
    ),
    (
        "Keys on the mountain",
        "Arrow keys or W A S D = left stick (steer, tuck, brake), Space = A (jump), I / J / L / K = Y / X / B / A, \
         Shift = RT, Ctrl = LT, E = RB, Backspace = LB (rewind), Esc = START (pause).",
    ),
];
