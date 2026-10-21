use eframe::egui::{self, RichText, Ui};

use crate::app::Launcher;
use crate::art::{Slot, Source};
use crate::command;
use crate::settings::{self, Settings};
use crate::ui::theme::{self, Face, size};
use crate::ui::{footer, widgets};

impl Launcher {
    pub fn about_section(&self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Project Avalanche").font(Face::Title.id(size::TITLE)).color(theme::TEXT));
            ui.label(RichText::new(footer::version()).color(theme::TEXT_DIM));
        });
        widgets::paragraph(
            ui,
            "A native PC port of SSX (2012), written from the ground up in Rust on Bevy. No emulation and no \
             recompilation: the original data formats and the game's behaviour are reverse-engineered and \
             re-implemented. The port is unfinished.",
        );
        ui.add_space(8.0);
        widgets::heading(ui, "Your game data");
        widgets::paragraph(
            ui,
            "No game data is included. The launcher unpacks your own SSX (2012) Xbox 360 disc image into the data \
             folder; the image is only read. The pictures of this window come from that data too, or are painted \
             in code: none is shipped.",
        );
        ui.add_space(8.0);
        widgets::heading(ui, "Credits");
        widgets::paragraph(
            ui,
            "The format research started from the community's work — SSX-Library, bigfile and SSXTools (EA \
             archives), Xenia and XeXtractor (Xbox 360), FFmpeg and vgmstream (movies and audio) — and every \
             format was then re-derived on the retail disc and implemented independently.",
        );
        ui.add_space(8.0);
        widgets::heading(ui, "Licences and names");
        widgets::paragraph(
            ui,
            "Type: Saira Condensed, © 2016 The Saira Project Authors, SIL Open Font License 1.1 (OFL.txt beside \
             the fonts in the source). SSX is a trademark of Electronic Arts Inc.; Project Avalanche is not \
             affiliated with or endorsed by Electronic Arts.",
        );
    }

    pub fn files_section(&self, ui: &mut Ui) {
        let app = settings::app_dir();
        let save = command::save_path();
        let save_note = if save.is_file() { "the game's profile" } else { "none yet: Play starts a new profile" };
        let rows = [
            ("App folder", app.clone(), "settings, saves, the log"),
            ("Settings", Settings::path(), "every choice made here"),
            ("Game data", self.settings.game_data(), "the unpacked archives (big/)"),
            ("Profile save", save, save_note),
            ("Log", app.join("viewer.log"), "the output of the last start"),
            ("Your pictures", app.join("LauncherArt"), "your own <slot>.png replaces a picture below"),
            ("Picture cache", app.join("LauncherCache"), "decoded from the game data; safe to delete"),
        ];
        egui::Grid::new("files").num_columns(3).spacing([16.0, 2.0]).min_row_height(22.0).show(ui, |ui| {
            for (name, path, what) in rows {
                widgets::field(ui, name);
                let text = path.display().to_string();
                let label = egui::Label::new(RichText::new(&text).monospace().color(theme::TEXT)).truncate();
                ui.scope(|ui| {
                    ui.set_max_width(420.0);
                    if ui.add(label.sense(egui::Sense::click())).on_hover_text("Click to copy").clicked() {
                        ui.ctx().copy_text(text.clone());
                    }
                });
                widgets::note(ui, what);
                ui.end_row();
            }
        });
        ui.add_space(4.0);
        widgets::heading(ui, "Pictures of this window");
        egui::Grid::new("art").num_columns(3).spacing([16.0, 0.0]).min_row_height(22.0).show(ui, |ui| {
            for slot in Slot::ALL {
                widgets::field(ui, slot.name());
                let (source, note) = match self.gallery.get(slot) {
                    Some(art) => (
                        match art.source {
                            Source::User => "your file",
                            Source::Disc => "game data",
                            Source::BuiltIn => "built in",
                        },
                        art.note.as_str(),
                    ),
                    None if slot == Slot::Logo => ("none", "shown only from your file or your game data"),
                    None => ("loading", ""),
                };
                ui.label(RichText::new(source).color(theme::TEXT));
                ui.add(egui::Label::new(RichText::new(note).color(theme::TEXT_DIM)).truncate()).on_hover_text(note);
                ui.end_row();
            }
        });
        for problem in &self.gallery.problems {
            widgets::status(ui, theme::WARN, problem.clone());
        }
    }
}
