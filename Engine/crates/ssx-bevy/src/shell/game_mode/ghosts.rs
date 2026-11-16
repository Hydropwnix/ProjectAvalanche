use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use bevy::prelude::{error, info, warn};
use ssx_assets::career::save::SaveSchema;
use ssx_assets::career::save::ghosts::OwnGhosts;
use ssx_assets::ghosts::GhostReplay;
use ssx_files::writer::{Asked, Writer};
use ssx_formats::ghost::GhostFile;
use ssx_formats::save::ghosts as group;
use ssx_ui::game::fe::saveload::GhostsRead;

// PORT(choice): the ghosts are the file `ssxvghosts` beside a profile file named `ssxvprofile` (the launcher's
// `Saves/ssxvprofile`), else `<profile file name>.ghosts` beside it, so two profiles in one folder never share one; the
// console keeps the group `ssxvghosts` (vtable 0x8203699C, `SaveLoadManager` 0x8244DCC8) as a content package of the
// signed-in profile, beside its `ssxvprofile`.
pub(super) fn ghosts_path(save: &Path) -> PathBuf {
    let name = save.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    if name == "ssxvprofile" {
        return save.with_file_name("ssxvghosts");
    }
    let mut name = name;
    name.push(".ghosts");
    save.with_file_name(name)
}

pub struct GhostSave {
    path: PathBuf,
    bytes: Option<Arc<Vec<u8>>>,
    own: OwnGhosts,
    fresh: OwnGhosts,
    unread: Option<String>,
    aside: Arc<Mutex<Option<String>>>,
    // PORT(choice): the file is written on a thread of its own, the ghosts writer (the newest file asked for, each
    // after the one before), and the frames never wait for it: a kept run, the sign-in's new group and a launch's
    // slot load go on from the bytes in memory, which are the file once the writer is done; a "Retry" reads the disk
    // only while the writer is idle. Before (main 407047ec) each of them joined the write in flight on the frame's
    // thread (its sync waited 10–20 s under another program's 3 GB write, btrfs). The console's save task writes the
    // group's regions through the save library over frames (0x82541188 → 0x825574F0), the "Saving" icon up meanwhile.
    writer: Writer<WriteJob>,
}

struct WriteJob {
    why: &'static str,
    path: PathBuf,
    bytes: Arc<Vec<u8>>,
    asked: Instant,
}

pub(super) enum SlotLoad {
    Loaded(GhostReplay),
    Missing,
    Damaged(String),
    Refused(String),
}

fn read(path: &Path, schema: &SaveSchema) -> Result<Option<(Vec<u8>, OwnGhosts)>> {
    let file = match ssx_files::open_plain(path) {
        Ok(Some(f)) => f,
        Ok(None) => bail!("{} is not a plain file", path.display()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let len = file.metadata().with_context(|| format!("reading {}", path.display()))?.len();
    if len > group::MAX_FILE_LEN as u64 {
        bail!("{len} bytes, longer than the group's {}", group::MAX_FILE_LEN);
    }
    let mut bytes = Vec::new();
    file.take(group::MAX_FILE_LEN as u64 + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("reading {}", path.display()))?;
    if bytes.len() > group::MAX_FILE_LEN {
        bail!("longer than the group's {} bytes", group::MAX_FILE_LEN);
    }
    Ok(Some(parse(bytes, schema)?))
}

fn parse(bytes: Vec<u8>, schema: &SaveSchema) -> Result<(Vec<u8>, OwnGhosts)> {
    group::check_header(&bytes)?;
    group::check_len(bytes.len())?;
    let at = group::common_offset();
    let chunk = group::CommonChunk::from_region(&bytes[at..at + group::COMMON_REGION_SIZE])?;
    let own = OwnGhosts::from_chunk(&chunk, schema)?;
    Ok((bytes, own))
}

impl GhostSave {
    pub(super) fn load(path: PathBuf, schema: &SaveSchema) -> Result<(Self, String)> {
        Self::load_with(path, schema, write_job)
    }

    fn load_with(
        path: PathBuf,
        schema: &SaveSchema,
        mut write: impl FnMut(WriteJob, &Mutex<Option<String>>) + Send + 'static,
    ) -> Result<(Self, String)> {
        let fresh = OwnGhosts::fresh(schema)?;
        let aside = Arc::new(Mutex::new(None));
        let shared = Arc::clone(&aside);
        let writer = Writer::start("ghosts-save", move |job| write(job, &shared))
            .context("the ghosts' writer (a thread) could not start")?;
        let save = |bytes: Option<Vec<u8>>, own, unread| GhostSave {
            path: path.clone(),
            bytes: bytes.map(Arc::new),
            own,
            fresh: fresh.clone(),
            unread,
            aside,
            writer,
        };
        Ok(match read(&path, schema) {
            Ok(None) => {
                let note = format!("ghosts: {} does not exist: no own ghost kept yet", path.display());
                (save(None, fresh.clone(), None), note)
            }
            Ok(Some((bytes, own))) => {
                let slots = group::check_len(bytes.len())?;
                let drops = own.slots().iter().filter(|&&k| k != 0).count();
                let note = format!(
                    "ghosts: loaded {} ({} bytes, {slots} slot regions, own ghosts of {drops} drops)",
                    path.display(),
                    bytes.len()
                );
                (save(Some(bytes), own, None), note)
            }
            Err(e) => {
                let why = format!("{e:#}");
                let note = format!(
                    "ghosts: ERROR {} is not a ghosts file the game can read ({why}): the sign-in's load shows dialog \
                     0xB",
                    path.display()
                );
                (save(None, fresh.clone(), Some(why)), note)
            }
        })
    }

    pub(super) fn read_state(&self) -> GhostsRead {
        match (&self.unread, &self.bytes) {
            (Some(_), _) => GhostsRead::Corrupt,
            (None, None) => GhostsRead::Missing,
            (None, Some(_)) => GhostsRead::Loaded,
        }
    }

    pub(super) fn reread(&mut self, schema: &SaveSchema, sign_in: bool) -> GhostsRead {
        let found = match (self.writer.writing(), &self.bytes) {
            (true, Some(b)) => parse(b.as_ref().clone(), schema).map(Some),
            _ => read(&self.path, schema),
        };
        match found {
            Ok(None) => (self.bytes, self.unread) = (None, None),
            Ok(Some((bytes, own))) => {
                (self.bytes, self.unread) = (Some(Arc::new(bytes)), None);
                if sign_in {
                    self.own = own;
                }
            }
            Err(e) => {
                warn!("ghosts: {} still fails the group's checks: {e:#}", self.path.display());
                (self.bytes, self.unread) = (None, Some(format!("{e:#}")));
            }
        }
        self.read_state()
    }

    pub(super) fn not_loaded(&mut self) {
        self.own = self.fresh.clone();
    }

    pub(super) fn writing(&self) -> bool {
        self.writer.writing()
    }

    pub(super) fn own_slot(&self, drop: u32, profile_best: u32) -> Option<usize> {
        self.own.own_slot(drop, profile_best)
    }

    pub(super) fn load_slot(&mut self, slot: usize, schema: &SaveSchema) -> SlotLoad {
        if let Some(why) = &self.unread {
            return SlotLoad::Damaged(why.clone());
        }
        let Some(bytes) = self.bytes.clone() else { return SlotLoad::Missing };
        let c = group::common_offset();
        let common = bytes
            .get(c..c + group::COMMON_REGION_SIZE)
            .context("the common region is past the file's end")
            .and_then(|r| Ok(group::CommonChunk::from_region(r)?))
            .and_then(|chunk| OwnGhosts::from_chunk(&chunk, schema));
        match common {
            Ok(own) => self.own = own,
            Err(e) => return SlotLoad::Damaged(format!("{e:#}")),
        }
        let at = group::slot_offset(slot);
        let Some(region) = bytes.get(at..at + group::SLOT_REGION_SIZE) else {
            return SlotLoad::Damaged(format!("slot {slot}'s region is past the file's end"));
        };
        let container = match group::slot_container(region) {
            Ok(c) => c,
            Err(e) => return SlotLoad::Damaged(format!("slot {slot}: {e:#}")),
        };
        // PORT(choice): a slot's region that passes the group's check is still read as hostile input: its `GCMP` is
        // inflated within the replay buffer and the image parsed whole, and a ghost that fails is not drawn (logged);
        // the console's ghost cache takes the container as it is (0x8253FB78 → `GhostManager_AddGhostReplay`).
        match ssx_formats::ghost::read_any(container)
            .map_err(anyhow::Error::from)
            .and_then(|f| GhostReplay::from_file(slot as u32, &f))
        {
            Ok(replay) => SlotLoad::Loaded(replay),
            Err(e) => SlotLoad::Refused(format!("slot {slot}: {e:#}")),
        }
    }

    pub(super) fn keep(&mut self, drop: u32, score: u32, file: &GhostFile) -> Result<usize> {
        let image = file.write()?;
        let region = group::slot_region(&ssx_formats::ghost::compress(&image)?)?;
        let mut own = self.own.clone();
        let slot = own.keep(drop, score)?.context("every one of the 256 ghost slots holds another drop's ghost")?;
        let common = own.to_chunk().region()?;
        let mut bytes = match &self.bytes {
            Some(b) => b.as_ref().clone(),
            None => {
                let mut b = group::header_bytes().to_vec();
                b.resize(group::slot_offset(0), 0);
                b
            }
        };
        let at = group::slot_offset(slot);
        if bytes.len() < at + group::SLOT_REGION_SIZE {
            bytes.resize(at + group::SLOT_REGION_SIZE, 0);
        }
        bytes[at..at + group::SLOT_REGION_SIZE].copy_from_slice(&region);
        let c = group::common_offset();
        bytes[c..c + group::COMMON_REGION_SIZE].copy_from_slice(&common);
        self.write(bytes, "a kept run");
        self.own = own;
        Ok(slot)
    }

    pub(super) fn keep_in_memory(&mut self, drop: u32, score: u32) -> Result<usize> {
        let mut own = self.own.clone();
        let slot = own.keep(drop, score)?.context("every one of the 256 ghost slots holds another drop's ghost")?;
        self.own = own;
        Ok(slot)
    }

    pub(super) fn write_new(&mut self) -> Result<()> {
        let mut bytes = group::header_bytes().to_vec();
        bytes.extend_from_slice(&self.own.to_chunk().region()?);
        self.write(bytes, "the sign-in's new group");
        Ok(())
    }

    fn write(&mut self, bytes: Vec<u8>, why: &'static str) {
        if let Some(unread) = self.unread.take() {
            *self.aside.lock().unwrap_or_else(PoisonError::into_inner) = Some(unread);
        }
        let bytes = Arc::new(bytes);
        let job = WriteJob { why, path: self.path.clone(), bytes: Arc::clone(&bytes), asked: Instant::now() };
        match self.writer.ask(job) {
            Asked::Queued => {}
            Asked::Replaced(old) => {
                info!("ghosts ({}): not written, the newer file ({why}) holds every ghost", old.why)
            }
            Asked::Ended(_) => error!("ghosts ({why}): not written: the ghosts' writer has stopped"),
        }
        self.bytes = Some(bytes);
    }
}

impl Drop for GhostSave {
    fn drop(&mut self) {
        if !self.writer.finish() {
            error!("ghosts: the ghosts' writer stopped with a panic");
        }
    }
}

fn write_job(job: WriteJob, aside: &Mutex<Option<String>>) {
    let started = Instant::now();
    match write_file(&job, aside) {
        Ok(()) => info!(
            "ghosts: wrote {} ({} bytes, {}) in {} ms on the ghosts' writer ({} ms after it was asked for)",
            job.path.display(),
            job.bytes.len(),
            job.why,
            started.elapsed().as_millis(),
            job.asked.elapsed().as_millis(),
        ),
        Err(e) => error!("ghosts ({}): writing {} failed — {e:#}", job.why, job.path.display()),
    }
}

fn write_file(job: &WriteJob, aside: &Mutex<Option<String>>) -> Result<()> {
    let unread = aside.lock().unwrap_or_else(PoisonError::into_inner).take();
    // PORT(choice): a file that failed the group's checks is kept as `<file>.unread<n>` (never written over) before
    // the new one is written; the console's save writes the group's file again in its place (the failed load's
    // 0x8316D968 left the group's record without a file name and +0xAA = 1).
    if let Some(why) = unread
        && job.path.exists()
    {
        let names = (1..100).map(|n| {
            let mut name = job.path.as_os_str().to_os_string();
            name.push(format!(".unread{n}"));
            PathBuf::from(name)
        });
        let moved = ssx_files::move_aside(&job.path, names).map_err(anyhow::Error::from).and_then(|a| {
            a.with_context(|| format!("{}: a hundred unread ghosts files are kept already", job.path.display()))
        });
        match moved {
            Ok(a) => warn!("ghosts: {} ({why}) is kept as {}", job.path.display(), a.display()),
            Err(e) => {
                *aside.lock().unwrap_or_else(PoisonError::into_inner) = Some(why);
                return Err(e);
            }
        }
    }
    // PORT(choice): the whole file is written anew (`ssx_files::replace`: a temporary file synced, then renamed over
    // it), so an interrupted write leaves the last file whole; the console's save task (0x82541188 → 0x825574F0) has
    // the save library write the slot's region and the common region into the group's content package.
    if let Some(dir) = job.path.parent().filter(|d| !d.as_os_str().is_empty()) {
        ssx_files::create_dir_all(dir)?;
    }
    ssx_files::replace(&job.path, &job.bytes, &ssx_files::Options::new())?;
    Ok(())
}

#[cfg(test)]
mod tests;
