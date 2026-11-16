use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use bevy::prelude::{error, info, warn};
use ssx_content::profile_state::ModState;
use ssx_files::writer::Asked;

pub(super) struct Job {
    pub(super) why: String,
    pub(super) path: PathBuf,
    pub(super) bytes: Vec<u8>,
    pub(super) state: ModState,
    pub(super) summary: String,
    pub(super) asked: Instant,
    pub(super) made: Duration,
}

pub(super) type Writer = ssx_files::writer::Writer<Job>;

#[derive(Default)]
pub(super) struct Asides {
    pub(super) state_unread: Mutex<Option<String>>,
    pub(super) damaged: AtomicBool,
}

pub(super) fn start(asides: Arc<Asides>) -> std::io::Result<Writer> {
    Writer::start("profile-save", move |job| write_job(job, &asides))
}

pub(super) fn ask(writer: &Writer, job: Job) {
    let why = job.why.clone();
    match writer.ask(job) {
        Asked::Queued => {}
        Asked::Replaced(old) => {
            info!("save ({}): not written, the newer save ({why}) holds the whole profile", old.why)
        }
        Asked::Ended(_) => error!("save ({why}) not written: the save writer has stopped"),
    }
}

pub(super) fn write_job(job: Job, asides: &Asides) {
    let started = Instant::now();
    let r = (|| -> Result<()> {
        if asides.damaged.load(Ordering::Acquire) {
            keep_damaged_aside(&job.path, &asides.state_unread)?;
            asides.damaged.store(false, Ordering::Release);
        }
        if let Some(dir) = job.path.parent().filter(|d| !d.as_os_str().is_empty()) {
            ssx_files::create_dir_all(dir)?;
        }
        ssx_files::replace(&job.path, &job.bytes, &ssx_files::Options::new())?;
        write_mod_state(&job.path, &job.state, &asides.state_unread)
    })();
    match r {
        Ok(()) => info!(
            "save ({}): {} ({} bytes): {}; made in {} ms, written in {} ms on the save writer ({} ms after the save \
             was asked for)",
            job.why,
            job.path.display(),
            job.bytes.len(),
            job.summary,
            job.made.as_millis(),
            started.elapsed().as_millis(),
            job.asked.elapsed().as_millis(),
        ),
        Err(e) => error!("save ({}) failed: {} — {e:#}", job.why, job.path.display()),
    }
}

fn keep_damaged_aside(path: &Path, unread: &Mutex<Option<String>>) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let names = (1..100).map(|n| path.with_extension(format!("damaged{n}")));
    let aside = ssx_files::move_aside(path, names)
        .map_err(|e| anyhow::anyhow!("the damaged {} could not be moved aside: {}", path.display(), e.io()))?
        .with_context(|| format!("a hundred damaged saves are kept already; {} stays", path.display()))?;
    info!("save: the damaged file is kept as {}", aside.display());
    let state = ssx_content::profile_state::state_path(path);
    if !state.exists() {
        return Ok(());
    }
    let to = ssx_content::profile_state::state_path(&aside);
    let moved = ssx_files::rename_new(&state, &to).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => format!("{} exists", to.display()),
        _ => e.io().to_string(),
    });
    let mut unread = unread.lock().unwrap_or_else(PoisonError::into_inner);
    match moved {
        Ok(()) => {
            info!("save: the damaged profile's mod state is kept as {}", to.display());
            *unread = None;
        }
        Err(e) => {
            error!("save: the damaged profile's mod state {} could not be moved aside: {e}", state.display());
            *unread = Some("the damaged profile's".to_string());
        }
    }
    Ok(())
}

fn write_mod_state(save: &Path, state: &ModState, unread: &Mutex<Option<String>>) -> Result<()> {
    let path = ssx_content::profile_state::state_path(save);
    let why = unread.lock().unwrap_or_else(PoisonError::into_inner).take();
    if let Some(why) = why
        && path.exists()
    {
        let names = (1..100).map(|n| path.with_extension(format!("toml.unread{n}")));
        let moved = ssx_files::move_aside(&path, names).map_err(anyhow::Error::from).and_then(|aside| {
            aside.with_context(|| format!("{}: a hundred unread states are kept already", path.display()))
        });
        match moved {
            Ok(aside) => warn!("save: the mod state {} ({why}) is kept as {}", path.display(), aside.display()),
            Err(e) => {
                *unread.lock().unwrap_or_else(PoisonError::into_inner) = Some(why);
                return Err(e);
            }
        }
    }
    if state.entries.is_empty() && !path.exists() {
        return Ok(());
    }
    state.write_file(&path).map_err(anyhow::Error::msg)
}

#[cfg(test)]
mod tests;
