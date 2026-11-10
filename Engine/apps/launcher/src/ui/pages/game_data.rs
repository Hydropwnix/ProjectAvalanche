use eframe::egui::Ui;

use crate::app::Launcher;
use crate::data_state;
use crate::setup_flow::gb;
use crate::ui::theme;
use crate::ui::widgets::{self, Kind};

impl Launcher {
    pub fn game_data_section(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        self.check_remembered_image(&ctx);
        let busy = self.job.is_some();
        let data = self.settings.game_data();
        ui.spacing_mut().item_spacing.y = 6.0;
        let notice = if busy { None } else { data_state::notice(&self.data_state) };
        if let Some((colour, text)) = notice.clone().filter(|_| self.data_state.needs_repair()) {
            widgets::status(ui, colour, text);
        } else {
            widgets::paragraph(
                ui,
                "Project Avalanche runs on the data of your own SSX (2012) Xbox 360 disc. Choose the disc image \
                 (.iso) once: the game archives and movies are extracted from it into the data folder (about 4.7 GB). \
                 The image itself is only read, never changed.",
            );
        }
        ui.add_space(6.0);
        widgets::heading(ui, "Disc image");
        match &self.settings.iso {
            Some(iso) => widgets::path(ui, iso),
            None => widgets::note(ui, "(none selected)"),
        }
        ui.horizontal(|ui| {
            if widgets::button(ui, "Select SSX (2012) Xbox 360 ISO…", Kind::Plain, !busy).clicked() {
                self.pick_iso(&ctx);
            }
            if self.checking.is_some() {
                ui.spinner();
                ui.label("Checking the image…");
            }
            match &self.check {
                Some(Ok(c)) => widgets::status(
                    ui,
                    theme::GOOD,
                    format!(
                        "SSX (2012), Xbox 360 image ({:?} layout, {} files): {} archives, {} movies, {} to read",
                        c.layout,
                        c.files,
                        c.archives.len(),
                        c.movies.len(),
                        gb(c.archive_bytes() + c.movie_bytes())
                    ),
                ),
                Some(Err(e)) => widgets::status(ui, theme::ERROR, e.clone()),
                None => {}
            }
        });
        ui.add_space(6.0);
        widgets::heading(ui, "Data folder");
        widgets::path(ui, &data);
        ui.horizontal(|ui| {
            if widgets::button(ui, "Change…", Kind::Plain, !busy).clicked() {
                self.pick_data_dir();
            }
            if widgets::button(ui, "Default", Kind::Plain, !busy && self.settings.game_data.is_some()).clicked() {
                self.settings.game_data = None;
                self.data_changed();
            }
            let (colour, label) = data_state::status(&self.data_state);
            widgets::status(ui, colour, label);
        });
        if let Some((_, text)) = notice.filter(|_| self.data_state == ssx_assets::folder::State::Unfinished) {
            widgets::note(ui, &text);
        }
        if let (Some(Ok(c)), Some(disc)) = (&self.check, &self.data_disc)
            && *disc != c.fingerprint
            && !busy
        {
            widgets::note(
                ui,
                "This image is not the one the folder was set up from: every file that differs from it is written again.",
            );
        }
        ui.add_space(6.0);
        widgets::heading(ui, "Setup");
        self.setup_controls(ui);
    }

    fn setup_controls(&mut self, ui: &mut Ui) {
        if let Some(p) = self.setup_progress() {
            widgets::progress(ui, p.fraction);
            let cancelling = self.job.as_ref().is_some_and(|j| j.cancelling());
            let label = if cancelling { "Cancelling…" } else { "Cancel" };
            ui.horizontal(|ui| {
                if widgets::button(ui, label, Kind::Plain, !cancelling).clicked()
                    && let Some(job) = &self.job
                {
                    job.cancel();
                }
                ui.label(format!("{:.0} %  ·  {}", p.fraction * 100.0, p.line));
            });
        } else {
            let ready = matches!(self.check, Some(Ok(_))) && !self.running();
            ui.horizontal(|ui| {
                if widgets::button(ui, data_state::button(&self.data_state), Kind::Primary, ready).clicked() {
                    self.start_setup(ui.ctx());
                }
                if !ready {
                    widgets::note(ui, "Select a valid image first; the game must not be running.");
                }
            });
        }
        if let Some((ok, message)) = &self.setup_message {
            widgets::status(ui, if *ok { theme::GOOD } else { theme::ERROR }, message.clone());
        }
    }
}
