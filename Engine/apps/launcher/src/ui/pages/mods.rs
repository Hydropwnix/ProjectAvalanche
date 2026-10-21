use eframe::egui::{RichText, Ui};

use crate::app::Launcher;
use crate::ui::layout::Layout;
use crate::ui::theme::{self, Face, size};
use crate::ui::{panel, widgets};

impl Launcher {
    pub fn mods_page(&mut self, ui: &mut Ui, l: &Layout) {
        let body = panel::frame(ui.painter(), l, "MODS");
        panel::body(ui, body, "mods", |ui| {
            ui.add_space(8.0);
            ui.label(RichText::new("Not available yet").font(Face::Title.id(size::TITLE)).color(theme::WARN));
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "Project Avalanche has no mod support. Nothing can be browsed, installed or loaded from here, \
                     and the game does not read modified data.",
                )
                .size(size::TEXT)
                .color(theme::TEXT),
            );
            ui.add_space(12.0);
            widgets::heading(ui, "What is planned");
            widgets::paragraph(
                ui,
                "Mod support is planned for a later stage of the port. None of it is built: there is no mod format, \
                 no loader and no browser yet. The port still decodes and re-implements the original game; that \
                 comes first.",
            );
            ui.add_space(12.0);
            widgets::heading(ui, "Until then");
            widgets::paragraph(
                ui,
                "The pictures of this launcher can be replaced with your own: see Extras, Files. The game itself \
                 runs on the unmodified data of your disc.",
            );
        });
    }
}
