use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;

use super::port_settings::{PortSettings, SaveError, replace_unreadable, save_file};

#[derive(Debug)]
pub enum Ask {
    Save { base: PortSettings, settings: PortSettings, read: u64, popup: String },
    Replace { base: PortSettings, settings: PortSettings, read: u64 },
}

#[derive(Debug)]
pub enum Done {
    Saved { path: PathBuf, settings: PortSettings, read: u64 },
    Replaced { path: PathBuf, aside: PathBuf, settings: PortSettings, read: u64 },
    Unreadable { path: PathBuf, why: String, popup: String },
    Failed(String),
}

pub struct SettingsWriter {
    to: Option<Sender<Ask>>,
    done: Mutex<Receiver<Done>>,
    pending: Arc<AtomicUsize>,
    worker: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for SettingsWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingsWriter").field("pending", &self.pending.load(Ordering::Acquire)).finish()
    }
}

impl SettingsWriter {
    pub fn start(path: PathBuf) -> std::io::Result<Self> {
        let (to, from) = mpsc::channel();
        let (answer, done) = mpsc::channel();
        let pending = Arc::new(AtomicUsize::new(0));
        let left = Arc::clone(&pending);
        let worker = std::thread::Builder::new()
            .name("settings-save".into())
            .spawn(move || work(&path, &from, &answer, &left))?;
        Ok(SettingsWriter { to: Some(to), done: Mutex::new(done), pending, worker: Some(worker) })
    }

    pub fn ask(&self, ask: Ask) -> bool {
        let Some(to) = &self.to else { return false };
        self.pending.fetch_add(1, Ordering::AcqRel);
        if to.send(ask).is_err() {
            self.pending.fetch_sub(1, Ordering::AcqRel);
            return false;
        }
        true
    }

    pub fn writing(&self) -> bool {
        self.pending.load(Ordering::Acquire) > 0
    }

    pub fn take_done(&self) -> Vec<Done> {
        self.done.lock().unwrap_or_else(PoisonError::into_inner).try_iter().collect()
    }

    pub fn finish(&mut self) -> bool {
        self.to.take();
        self.worker.take().is_none_or(|w| w.join().is_ok())
    }
}

impl Drop for SettingsWriter {
    fn drop(&mut self) {
        self.finish();
    }
}

struct Written {
    read: u64,
    base: PortSettings,
    settings: PortSettings,
}

fn work(path: &Path, from: &Receiver<Ask>, answer: &Sender<Done>, pending: &AtomicUsize) {
    let mut last: Option<Written> = None;
    let mut next = from.recv().ok();
    while let Some(ask) = next.take() {
        let mut taken = 1;
        let ask = match ask {
            Ask::Save { base, mut settings, read, mut popup } => {
                loop {
                    match from.try_recv() {
                        Ok(Ask::Save { settings: s, popup: p, .. }) => {
                            (settings, popup) = (s, p);
                            taken += 1;
                        }
                        Ok(other) => {
                            next = Some(other);
                            break;
                        }
                        Err(_) => break,
                    }
                }
                Ask::Save { base, settings, read, popup }
            }
            other => other,
        };
        let since = |base: &PortSettings, read: u64| match &last {
            Some(w) if w.read == read && w.base == *base => w.settings.clone(),
            _ => base.clone(),
        };
        let done = match ask {
            Ask::Save { base, settings, read, popup } => match save_file(path, &since(&base, read), &settings) {
                Ok(()) => {
                    last = Some(Written { read, base, settings: settings.clone() });
                    Done::Saved { path: path.to_path_buf(), settings, read }
                }
                Err(SaveError::Unreadable(why)) => Done::Unreadable { path: path.to_path_buf(), why, popup },
                Err(SaveError::Io(e)) => Done::Failed(format!("{e:#}")),
            },
            Ask::Replace { base, settings, read } => match replace_unreadable(path, &settings) {
                Ok(aside) => {
                    last = Some(Written { read, base, settings: settings.clone() });
                    Done::Replaced { path: path.to_path_buf(), aside, settings, read }
                }
                Err(e) => Done::Failed(format!("{e:#}")),
            },
        };
        let _ = answer.send(done);
        pending.fetch_sub(taken, Ordering::AcqRel);
        if next.is_none() {
            next = from.recv().ok();
        }
    }
}

#[cfg(test)]
mod tests;
