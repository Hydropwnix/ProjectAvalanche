mod about;
mod game_data;
mod mods;
mod options;
mod output;
mod play;
mod tools;

use eframe::egui::Ui;

use crate::app::Launcher;
use crate::ui::layout::Layout;
use crate::ui::{ExtrasTab, SettingsTab, panel};

impl Launcher {
    pub fn settings_page(&mut self, ui: &mut Ui, l: &Layout) {
        let body = panel::frame(ui.painter(), l, "SETTINGS");
        panel::tabs(ui, l, &SettingsTab::ALL, &mut self.settings_tab);
        let tab = self.settings_tab;
        panel::body(ui, body, "settings", |ui| match tab {
            SettingsTab::GameData => self.game_data_section(ui),
            SettingsTab::Graphics => self.graphics_section(ui),
            SettingsTab::Audio => self.audio_section(ui),
            SettingsTab::Controls => self.controls_section(ui),
        });
    }

    pub fn extras_page(&mut self, ui: &mut Ui, l: &Layout) {
        let body = panel::frame(ui.painter(), l, "EXTRAS");
        panel::tabs(ui, l, &ExtrasTab::ALL, &mut self.extras_tab);
        let tab = self.extras_tab;
        panel::body(ui, body, "extras", |ui| match tab {
            ExtrasTab::Tools => self.tools_section(ui),
            ExtrasTab::Output => self.output_section(ui),
            ExtrasTab::About => self.about_section(ui),
            ExtrasTab::Files => self.files_section(ui),
        });
    }
}
