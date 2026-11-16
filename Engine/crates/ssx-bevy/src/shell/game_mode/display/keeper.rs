use std::path::{Path, PathBuf};

use bevy::log::{info, warn};
use ssx_assets::display::DisplaySettings;
use ssx_files::writer::{Asked, Writer};

pub(super) struct Keeper {
    writer: Writer<DisplaySettings>,
}

impl Keeper {
    pub(super) fn start(file: PathBuf, kept: DisplaySettings) -> std::io::Result<Self> {
        let mut kept = kept;
        let writer = Writer::start("display-keep", move |next| write(&file, &mut kept, next))?;
        Ok(Keeper { writer })
    }

    pub(super) fn keep(&self, settings: DisplaySettings) {
        if let Asked::Ended(_) = self.writer.ask(settings) {
            warn!("display: the settings' writer has ended: the window's state is not kept");
        }
    }

    pub(super) fn writing(&self) -> bool {
        self.writer.writing()
    }

    pub(super) fn finish(&mut self) {
        if !self.writer.finish() {
            warn!("display: the settings' writer stopped with a panic");
        }
    }
}

fn write(file: &Path, kept: &mut DisplaySettings, next: DisplaySettings) {
    if next == *kept {
        return;
    }
    match ssx_assets::display::write_member(file, kept, &next) {
        Ok(()) => {
            info!("display: kept in {}", file.display());
            *kept = next;
        }
        Err(e) => warn!("display: {e:#}"),
    }
}
