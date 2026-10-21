use eframe::egui::{self, Color32, CursorIcon, Sense, Ui};

use crate::app::Launcher;
use crate::art::Slot;
use crate::catalog;
use crate::ui::icons::Icon;
use crate::ui::layout::Layout;
use crate::ui::tiles::{self, Hero, Tile};
use crate::ui::{Page, theme};

const TILES: [(Slot, Icon, &str, &str, Page, &str); 3] = [
    (
        Slot::TileMods,
        Icon::Mountain,
        "MOD BROWSER",
        "Not available yet.\nMod support is planned, not built.",
        Page::Mods,
        "tile_mods",
    ),
    (
        Slot::TileSettings,
        Icon::Gear,
        "SETTINGS",
        "Game data, graphics, audio\nand the controls.",
        Page::Settings,
        "tile_settings",
    ),
    (
        Slot::TileExtras,
        Icon::Wrench,
        "EXTRAS",
        "Tools, the output of what was started,\ncredits and where the files are.",
        Page::Extras,
        "tile_extras",
    ),
];

struct HeroText {
    title: &'static str,
    line: (String, Color32),
    tab: (&'static str, Color32),
    enabled: bool,
}

impl Launcher {
    fn hero_text(&self) -> HeroText {
        let soft = theme::TEXT_SOFT;
        if let Some(p) = self.setup_progress() {
            let line = format!("{:.0} %  ·  {}", p.fraction * 100.0, p.line);
            return HeroText {
                title: "SETTING UP GAME DATA",
                line: (line, soft),
                tab: ("SETTING UP", theme::CYAN),
                enabled: true,
            };
        }
        if !self.data_ready {
            let line = if ssx_assets::setup::needs_update(&self.settings.game_data()) {
                "This data folder has no movies yet: select the same disc image again and they are added."
            } else {
                "Choose your SSX (2012) disc image: the game data is unpacked from it once."
            };
            let line = line.to_owned();
            return HeroText {
                title: "SET UP GAME DATA",
                line: (line, soft),
                tab: ("SETUP NEEDED", theme::WARN),
                enabled: true,
            };
        }
        if self.tracks.is_empty() {
            let dir = self.settings.big_root().join(catalog::TRACKS_DIR);
            let line = (format!("No tracks found under {}", dir.display()), theme::ERROR);
            return HeroText { title: "GAME DATA INCOMPLETE", line, tab: ("SETUP NEEDED", theme::WARN), enabled: true };
        }
        let title = "PLAY SSX (2012)";
        if let Some(e) = &self.launch_error {
            return HeroText {
                title,
                line: (e.clone(), theme::ERROR),
                tab: ("START FAILED", theme::ERROR),
                enabled: true,
            };
        }
        let name = if self.started_game { "Game" } else { "Viewer" };
        match self.viewer.as_ref().map(|v| (v.pid, v.status())) {
            Some((pid, None)) => {
                let more = if self.started_game {
                    "The boot movies come first; F1 skips one."
                } else {
                    "Loading a track takes a few seconds."
                };
                let line = (format!("{name} running (process {pid}). {more}"), theme::CYAN_SOFT);
                HeroText { title, line, tab: ("RUNNING", theme::CYAN), enabled: false }
            }
            Some((_, Some(status))) => {
                let ok = status == "exited normally";
                let more = if ok { "" } else { " See Extras, Output." };
                let line = (format!("{name} {status}.{more}"), if ok { theme::GOOD } else { theme::ERROR });
                HeroText { title, line, tab: ("READY", theme::GOOD), enabled: true }
            }
            None => {
                let line = match &self.setup_message {
                    Some((true, message)) => (message.clone(), theme::GOOD),
                    _ => ("Experience the original game, now on PC.".to_owned(), soft),
                };
                HeroText { title, line, tab: ("READY", theme::GOOD), enabled: true }
            }
        }
    }

    pub fn hero_clicked(&mut self, ctx: &egui::Context) {
        if !self.data_ready || self.tracks.is_empty() || self.job.is_some() {
            self.go_to_setup();
        } else if !self.running() {
            self.play(ctx);
        }
    }

    pub fn play_page(&mut self, ui: &mut Ui, l: &Layout) {
        let painter = ui.painter().clone();
        let forced = self.forced_hover.clone();
        let words = self.hero_text();
        let mut response = ui.interact(l.hero, ui.id().with("hero"), Sense::CLICK);
        if words.enabled {
            response = response.on_hover_cursor(CursorIcon::PointingHand);
        }
        if response.clicked() {
            self.hero_clicked(ui.ctx());
        }
        let hero = Hero {
            title: words.title,
            line: (&words.line.0, words.line.1),
            tab: words.tab,
            hovered: response.hovered() || forced.as_deref() == Some("hero"),
            enabled: words.enabled,
        };
        tiles::hero(&painter, l, l.hero, self.gallery.get(Slot::Hero), &hero);
        for (i, (slot, icon, title, text, page, name)) in TILES.into_iter().enumerate() {
            let rect = l.tiles[i];
            let response =
                ui.interact(rect, ui.id().with(name), Sense::CLICK).on_hover_cursor(CursorIcon::PointingHand);
            if response.clicked() {
                self.go(page);
            }
            let hovered = response.hovered() || forced.as_deref() == Some(name);
            tiles::tile(&painter, l, rect, self.gallery.get(slot), &Tile { icon, title, text, hovered });
        }
    }
}
