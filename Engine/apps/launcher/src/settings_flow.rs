use eframe::egui;
use ssx_assets::settings_file::UpdateError;

use crate::app::Launcher;
use crate::settings::Settings;
use crate::ui::{theme, widgets};

#[derive(Default)]
pub struct SettingsSave {
    pub refused: Option<Settings>,
    pub unreadable: Option<String>,
    pub note: Option<(bool, String)>,
}

impl Launcher {
    pub fn settings_frame(&mut self) {
        let game_closed = self.started_game && self.viewer.as_ref().is_some_and(|v| v.status().is_some());
        if !self.displays.synced && game_closed {
            self.displays.synced = true;
            if let Err(e) = self.settings.reload(&mut self.saved) {
                ssx_core::eprintln!("settings: not read again after the game: {e}");
                self.settings_save.note = Some((false, format!("not read again after the game: {e}")));
            }
        }
        if self.settings != self.saved && self.settings_save.refused.as_ref() != Some(&self.settings) {
            let _ = self.save_settings();
        }
    }

    pub fn save_settings(&mut self) -> Result<(), String> {
        match self.settings.save(&self.saved) {
            Ok(written) => {
                self.settings = written.clone();
                self.saved = written;
                self.settings_save.refused = None;
                Ok(())
            }
            Err(e) => {
                let text = format!("{e}");
                ssx_core::eprintln!("settings: not saved: {text}");
                self.settings_save.refused = Some(self.settings.clone());
                match e {
                    UpdateError::Unreadable(why) => self.settings_save.unreadable = Some(why),
                    UpdateError::Failed(_) => self.settings_save.note = Some((false, format!("not saved: {text}"))),
                }
                Err(text)
            }
        }
    }

    pub fn settings_question(&mut self, ctx: &egui::Context) {
        let path = Settings::path();
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let text = self.settings_save.unreadable.as_ref().map(|why| {
            format!(
                "{}\n\nThe launcher's changes are not saved while it cannot be read. Keep it beside it as \
                 {name}.broken-<n> and write a new one with the launcher's settings? What the game kept in it (its \
                 Options, your name on servers, the window) starts over.",
                widgets::shown(why)
            )
        });
        let asked = text.as_deref().map(|text| widgets::Question {
            title: "The settings file cannot be read",
            text,
            yes: "Keep it aside and save",
            yes_enabled: true,
        });
        let Some(yes) = widgets::question(ctx, egui::Id::new("settings-unreadable"), asked) else { return };
        self.settings_save.unreadable = None;
        if !yes {
            self.settings_save.note = Some((false, format!("not saved: {name} cannot be read")));
            return;
        }
        match self.settings.replace_unreadable() {
            Ok((aside, written)) => {
                let aside = aside.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.settings_save.note = Some((true, format!("the unreadable file was kept as {aside}; saved")));
                self.settings = written.clone();
                self.saved = written;
                self.settings_save.refused = None;
            }
            Err(e) => self.settings_save.note = Some((false, format!("not saved: {e:#}"))),
        }
    }

    pub fn settings_notes(&self, ui: &mut egui::Ui) {
        if let Some((good, text)) = &self.settings_save.note {
            let colour = if *good { theme::GOOD } else { theme::ERROR };
            widgets::status(ui, colour, format!("Settings {}", widgets::shown(text)));
        }
        for note in &self.settings.notes {
            widgets::note(ui, &widgets::shown(note));
        }
    }
}
