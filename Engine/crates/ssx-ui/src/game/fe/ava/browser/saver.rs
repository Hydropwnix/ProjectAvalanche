use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Mutex, PoisonError};

use ssx_files::writer::{Asked, Writer};
use ssx_net_join::browse::ServersFile;

// PORT(choice): the server lists are written on a thread of their own and the frame goes on (a write syncs the file
// and its folder: seconds under another program's write-back, btrfs, round io1); the console has no server browser.
pub struct ServersWriter {
    writer: Writer<(PathBuf, ServersFile)>,
    notes: Mutex<Receiver<String>>,
}

impl ServersWriter {
    pub fn start() -> std::io::Result<Self> {
        let (note, notes) = mpsc::channel();
        let writer = Writer::start("servers-save", move |(dir, file): (PathBuf, ServersFile)| match file.save(&dir) {
            Ok(None) => {}
            Ok(Some(n)) | Err(n) => {
                let _ = note.send(n);
            }
        })?;
        Ok(ServersWriter { writer, notes: Mutex::new(notes) })
    }

    pub fn ask(&self, dir: PathBuf, file: ServersFile) -> bool {
        !matches!(self.writer.ask((dir, file)), Asked::Ended(_))
    }

    pub fn writing(&self) -> bool {
        self.writer.writing()
    }

    pub fn take_notes(&self) -> Vec<String> {
        self.notes.lock().unwrap_or_else(PoisonError::into_inner).try_iter().collect()
    }
}
