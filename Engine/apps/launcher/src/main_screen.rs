use eframe::egui::{self, Key};

use crate::app::Launcher;
use crate::command;
use crate::ui::footer::Status;
use crate::ui::layout::{self, Layout};
use crate::ui::{self, MenuItem, Page, theme};

impl Launcher {
    pub fn status(&self) -> Status {
        let status = |colour, label: &str| Status { colour, label: label.to_owned() };
        if let Some(p) = self.setup_progress() {
            return Status {
                colour: theme::CYAN, label: format!("Setting up game data… {:.0} %", p.fraction * 100.0)
            };
        }
        match (self.running(), self.started_game, self.data_ready) {
            (true, true, _) => status(theme::CYAN, "Game running"),
            (true, false, _) => status(theme::CYAN, "Viewer running"),
            (false, _, true) if self.tracks.is_empty() => status(theme::WARN, "Game data incomplete"),
            (false, _, true) => status(theme::GOOD, "Game data ready"),
            (false, _, false) => status(theme::WARN, "Setup needed"),
        }
    }

    fn activate(&mut self, ctx: &egui::Context, item: MenuItem, key: bool) {
        match item {
            MenuItem::Exit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            MenuItem::Page(Page::Play) if key && self.page == Page::Play => self.hero_clicked(ctx),
            MenuItem::Page(page) => self.go(page),
        }
    }

    fn keys(&mut self, ctx: &egui::Context) {
        if ctx.memory(|m| m.focused().is_some()) || ctx.any_popup_open() {
            return;
        }
        let pressed = |key| ctx.input(|i| i.key_pressed(key));
        let step = pressed(Key::ArrowDown) as isize - pressed(Key::ArrowUp) as isize;
        if step != 0 {
            self.cursor = self.cursor.moved(step);
            if let MenuItem::Page(page) = self.cursor {
                self.page = page;
            }
        }
        if pressed(Key::Enter) {
            self.activate(ctx, self.cursor, true);
        }
        if pressed(Key::Escape) {
            self.go(Page::Play);
        }
    }

    fn start_pending(&mut self, ctx: &egui::Context) {
        if !self.data_ready || self.tracks.is_empty() {
            return;
        }
        let big = self.settings.big_root();
        if std::mem::take(&mut self.auto_play_pending) {
            self.start(ctx, &command::game(&self.settings, &big), true);
        }
        if std::mem::take(&mut self.auto_launch_pending) {
            self.start(ctx, &command::viewer(&self.settings, &big), false);
        }
    }

    fn screenshot_step(&mut self, ctx: &egui::Context) {
        let Some(plan) = &mut self.screenshot else { return };
        let extracting = !self.shot_after_setup && self.job.as_ref().is_some_and(|j| j.progress().files_done > 0);
        let viewer_done = self.viewer.as_ref().is_some_and(|v| v.status().is_some()) || self.launch_error.is_some();
        let settled = self.checking.is_none()
            && self.window_pixels.is_none()
            && self.scripted_keys.is_empty()
            && !self.gallery.loading
            && (!self.auto_extract || extracting || self.setup_message.is_some())
            && (!self.auto_launch || viewer_done)
            && !self.auto_play_pending;
        if !plan.requested && self.frame >= 40 && settled {
            plan.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        let image = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image {
            if let Err(e) = crate::save_png(&plan.path, &image) {
                eprintln!("error: saving {}: {e}", plan.path.display());
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint();
    }
}

impl eframe::App for Launcher {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        theme::NIGHT.to_normalized_gamma_f32()
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        let due = self.frame >= 12 && self.frame.is_multiple_of(4) && self.window_pixels.is_none();
        if let Some(key) = due.then(|| self.scripted_keys.pop_front()).flatten() {
            for pressed in [true, false] {
                let modifiers = Default::default();
                raw_input.events.push(egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers });
            }
        }
        if !self.scripted_keys.is_empty() {
            ctx.request_repaint();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.frame += 1;
        self.poll(&ctx);
        let loading = self.gallery.loading;
        self.gallery.poll(&ctx);
        if let Some(started) = self.timing {
            let ms = started.elapsed().as_secs_f64() * 1e3;
            if self.frame == 1 {
                eprintln!("first frame after {ms:.0} ms ({} pictures ready)", self.gallery.ready());
            }
            if loading && !self.gallery.loading {
                eprintln!("pictures complete after {ms:.0} ms (frame {})", self.frame);
            }
        }
        let screen = ctx.content_rect();
        let pixels = screen.size() * ctx.pixels_per_point();
        let wanted = layout::pixels_per_point(pixels);
        let scaled = (wanted - ctx.pixels_per_point()).abs() <= 0.001 * wanted;
        if !scaled {
            ctx.set_pixels_per_point(wanted);
        }
        if let Some([w, h]) = self.window_pixels {
            if ((pixels.x - w).abs() <= 1.0 && (pixels.y - h).abs() <= 1.0) || self.frame > 30 {
                self.window_pixels = None;
            } else {
                if scaled && self.frame > 2 {
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(w, h) / ctx.pixels_per_point()));
                }
                ctx.request_repaint();
            }
        }
        let l = Layout::new(screen, ctx.pixels_per_point());
        self.start_pending(&ctx);

        let painter = ui.painter().clone();
        ui::backdrop::draw(&painter, &l, &self.gallery);
        ui::header::draw(&painter, &l, &self.gallery);
        if let Some(item) = ui::menu::show(ui, &l, self.cursor, self.data_ready, self.forced_hover.as_deref()) {
            self.activate(&ctx, item, false);
        }
        match self.page {
            Page::Play => self.play_page(ui, &l),
            Page::Mods => self.mods_page(ui, &l),
            Page::Settings => self.settings_page(ui, &l),
            Page::Extras => self.extras_page(ui, &l),
        }
        ui::footer::draw(&painter, &l, &self.status());
        self.keys(&ctx);
        self.screenshot_step(&ctx);
    }
}
