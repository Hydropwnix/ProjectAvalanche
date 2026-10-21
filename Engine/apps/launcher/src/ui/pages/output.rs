use eframe::egui::{self, RichText, Ui};

use crate::app::Launcher;
use crate::command;
use crate::ui::theme;
use crate::ui::widgets::{self, Kind};

impl Launcher {
    pub fn output_section(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.show_command, "Show commands");
            self.started_line(ui);
        });
        if self.show_command {
            let big = self.settings.big_root();
            let (game, tool) = (command::game(&self.settings, &big), command::viewer(&self.settings, &big));
            let mut text = format!("{}\n{}", game.display(), tool.display());
            ui.horizontal(|ui| {
                if widgets::button(ui, "Copy", Kind::Plain, true).clicked() {
                    ui.ctx().copy_text(text.clone());
                }
                ui.add(egui::TextEdit::multiline(&mut text).code_editor().desired_rows(2).desired_width(f32::INFINITY));
            });
        }
        ui.add_space(4.0);
        let title = if self.started_game { "Game output" } else { "Viewer output" };
        widgets::heading(ui, title);
        match &self.viewer {
            Some(v) => {
                let lines = v.log.lock().map(|l| l.join("\n")).unwrap_or_default();
                let height = ui.available_height().max(120.0);
                egui::ScrollArea::vertical().id_salt("log").max_height(height).stick_to_bottom(true).show(ui, |ui| {
                    ui.add(
                        egui::Label::new(RichText::new(lines).monospace().size(11.0).color(theme::TEXT_SOFT)).wrap(),
                    );
                });
            }
            None => {
                widgets::note(ui, "Nothing was started yet. Play, or Launch under Tools, and the output appears here.")
            }
        }
    }
}
