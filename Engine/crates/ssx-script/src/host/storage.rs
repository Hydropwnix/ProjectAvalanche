use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions};

use super::limits::StorageCaps;
use crate::value::{self, NetValue, ValueLimits};

const MAGIC: &[u8; 8] = b"AVSTOR\x00\x01";
const FILE: &str = "storage.bin";
pub const MAX_KEY: usize = 64;
const QUEUE: usize = 64;
const FINAL_WAIT: Duration = Duration::from_secs(5);

pub fn check_key(key: &[u8]) -> Result<&str, String> {
    if key.is_empty() || key.len() > MAX_KEY {
        return Err(format!("a storage key is 1 to {MAX_KEY} bytes"));
    }
    if !key.iter().all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(b)) {
        return Err("a storage key is made of A-Z a-z 0-9 _ . -".into());
    }
    std::str::from_utf8(key).map_err(|_| "a storage key is ASCII".into())
}

fn entry_size(key: &str, value: &[u8]) -> usize {
    1 + key.len() + 4 + value.len()
}

pub struct Store {
    map: BTreeMap<String, Vec<u8>>,
    bytes: usize,
    pub dirty: bool,
    pub last_flush: Duration,
    pub unavailable: Option<String>,
    dir: Option<Dir>,
}

impl Store {
    pub fn unavailable(why: impl Into<String>) -> Self {
        Store {
            map: BTreeMap::new(),
            bytes: 0,
            dirty: false,
            last_flush: Duration::ZERO,
            unavailable: Some(why.into()),
            dir: None,
        }
    }

    pub fn open(root: &Path, package: &str, caps: &StorageCaps) -> Self {
        match Self::try_open(root, package, caps) {
            Ok(s) => s,
            Err(e) => Self::unavailable(e),
        }
    }

    fn try_open(root: &Path, package: &str, caps: &StorageCaps) -> Result<Self, String> {
        let top = Dir::open_ambient_dir(root, ambient_authority())
            .map_err(|e| format!("the storage folder {} cannot be opened: {e}", root.display()))?;
        match top.create_dir(package) {
            Ok(()) => ssx_files::sync_folder(root)
                .map_err(|e| format!("the package's storage folder cannot be made: {}", e.io()))?,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(format!("the package's storage folder cannot be made: {e}")),
        }
        let dir = top
            .open_dir_nofollow(package)
            .map_err(|e| format!("the package's storage folder cannot be opened without following links: {e}"))?;
        let mut store = Store {
            map: BTreeMap::new(),
            bytes: 0,
            dirty: false,
            last_flush: Duration::ZERO,
            unavailable: None,
            dir: None,
        };
        let limit = u64::try_from(caps.bytes + 12 + 32).unwrap_or(u64::MAX);
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        #[cfg(unix)]
        cap_std::fs::OpenOptionsExt::custom_flags(&mut options, libc::O_NONBLOCK);
        match dir.open_with(FILE, &options) {
            Ok(f) => {
                let f = f.into_std();
                let meta = f.metadata().map_err(|e| format!("{FILE}: {e}"))?;
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if meta.file_attributes() & 0x400 != 0 {
                        return Err(format!("{FILE} is a reparse point"));
                    }
                }
                if !meta.is_file() || meta.len() > limit {
                    return Err(format!("{FILE} is not a regular file of at most {limit} bytes"));
                }
                let mut bytes = Vec::new();
                f.take(limit + 1).read_to_end(&mut bytes).map_err(|e| format!("{FILE}: {e}"))?;
                let map = parse(&bytes, caps).map_err(|e| format!("{FILE} does not read back: {e}"))?;
                store.bytes = map.iter().map(|(k, v)| entry_size(k, v)).sum();
                store.map = map;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{FILE} cannot be opened without following links: {e}")),
        }
        store.dir = Some(dir);
        Ok(store)
    }

    pub fn get(&self, key: &str) -> Option<NetValue> {
        self.map.get(key).and_then(|b| value::decode(b, &ValueLimits::STORAGE).ok())
    }

    pub fn set(&mut self, key: &str, v: Option<&NetValue>, caps: &StorageCaps) -> Result<(), String> {
        if let Some(why) = &self.unavailable {
            return Err(format!("this package has no storage: {why}"));
        }
        let old = self.map.get(key).map_or(0, |b| entry_size(key, b));
        let Some(v) = v else {
            if self.map.remove(key).is_some() {
                self.bytes -= old;
                self.dirty = true;
            }
            return Ok(());
        };
        let encoded = value::encode(v, &ValueLimits::STORAGE).map_err(|e| e.to_string())?;
        let new = entry_size(key, &encoded);
        if old == 0 && self.map.len() >= caps.keys {
            return Err(format!("the package stores {} keys (at most {})", self.map.len(), caps.keys));
        }
        if self.bytes - old + new > caps.bytes {
            return Err(format!(
                "the package's storage would hold {} bytes (at most {})",
                self.bytes - old + new,
                caps.bytes
            ));
        }
        self.bytes = self.bytes - old + new;
        self.map.insert(key.to_string(), encoded);
        self.dirty = true;
        Ok(())
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn keys(&self) -> usize {
        self.map.len()
    }

    pub fn snapshot(&self) -> Option<(Dir, Vec<u8>)> {
        let dir = self.dir.as_ref()?.try_clone().ok()?;
        let mut out = Vec::with_capacity(12 + self.bytes + 32);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&u32::try_from(self.map.len()).unwrap_or(u32::MAX).to_le_bytes());
        for (k, v) in &self.map {
            out.push(u8::try_from(k.len()).ok()?);
            out.extend_from_slice(k.as_bytes());
            out.extend_from_slice(&u32::try_from(v.len()).unwrap_or(u32::MAX).to_le_bytes());
            out.extend_from_slice(v);
        }
        let hash = blake3::hash(&out);
        out.extend_from_slice(hash.as_bytes());
        Some((dir, out))
    }
}

fn parse(bytes: &[u8], caps: &StorageCaps) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let body_len = bytes.len().checked_sub(32).ok_or("shorter than its checksum")?;
    let (body, hash) = bytes.split_at(body_len);
    if blake3::hash(body).as_bytes() != hash {
        return Err("its checksum does not match".into());
    }
    let rest = body.strip_prefix(MAGIC.as_slice()).ok_or("not a storage file (magic)")?;
    let (count, mut rest) = rest.split_first_chunk::<4>().ok_or("no count")?;
    let count = usize::try_from(u32::from_le_bytes(*count)).map_err(|_| "a count beyond this system's sizes")?;
    if count > caps.keys {
        return Err(format!("{count} keys (at most {})", caps.keys));
    }
    let mut map = BTreeMap::new();
    for _ in 0..count {
        let (&klen, r) = rest.split_first().ok_or("ends in a key")?;
        let (key, r) = r.split_at_checked(usize::from(klen)).ok_or("ends in a key")?;
        let key = check_key(key)?.to_string();
        let (vlen, r) = r.split_first_chunk::<4>().ok_or("ends in a value's length")?;
        let vlen = usize::try_from(u32::from_le_bytes(*vlen)).map_err(|_| "a length beyond this system's sizes")?;
        let (v, r) = r.split_at_checked(vlen).ok_or("ends in a value")?;
        value::decode(v, &ValueLimits::STORAGE).map_err(|e| format!("{key}: {e}"))?;
        if map.insert(key.clone(), v.to_vec()).is_some() {
            return Err(format!("{key} twice"));
        }
        rest = r;
    }
    if !rest.is_empty() {
        return Err(format!("{} bytes after the last value", rest.len()));
    }
    Ok(map)
}

pub fn write_synced(dir: &Dir, bytes: &[u8]) -> std::io::Result<()> {
    ssx_files::replace_in(dir, FILE, bytes, &ssx_files::Options::new()).map_err(ssx_files::Error::into_io)
}

struct Job {
    package: String,
    dir: Dir,
    bytes: Vec<u8>,
}

pub struct Writer {
    jobs: Option<SyncSender<Job>>,
    results: Receiver<(String, Result<(), String>)>,
    done: Receiver<()>,
    pending: Arc<(Mutex<HashMap<String, usize>>, Condvar)>,
    pub root: PathBuf,
}

impl Writer {
    pub fn new(root: PathBuf) -> std::io::Result<Self> {
        let (jobs, rx) = sync_channel::<Job>(QUEUE);
        let (tx, results) = sync_channel(QUEUE);
        let (done_tx, done) = sync_channel(1);
        let pending: Arc<(Mutex<HashMap<String, usize>>, Condvar)> = Arc::default();
        let finished = pending.clone();
        std::thread::Builder::new().name("script-storage".into()).spawn(move || {
            for job in rx {
                let r = write_synced(&job.dir, &job.bytes).map_err(|e| e.to_string());
                {
                    let mut p = finished.0.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(n) = p.get_mut(&job.package) {
                        *n = n.saturating_sub(1);
                        if *n == 0 {
                            p.remove(&job.package);
                        }
                    }
                }
                finished.1.notify_all();
                let _ = tx.try_send((job.package, r));
            }
            let _ = done_tx.send(());
        })?;
        Ok(Writer { jobs: Some(jobs), results, done, pending, root })
    }

    pub fn settled(&self, package: &str) -> bool {
        let (lock, cv) = &*self.pending;
        let deadline = std::time::Instant::now() + FINAL_WAIT;
        let mut p = lock.lock().unwrap_or_else(|e| e.into_inner());
        while p.contains_key(package) {
            let now = std::time::Instant::now();
            if now >= deadline {
                return false;
            }
            p = cv.wait_timeout(p, deadline - now).unwrap_or_else(|e| e.into_inner()).0;
        }
        true
    }

    pub fn queue(&self, package: &str, store: &mut Store) -> bool {
        let Some((dir, bytes)) = store.snapshot() else { return false };
        let Some(jobs) = &self.jobs else { return false };
        let mut p = self.pending.0.lock().unwrap_or_else(|e| e.into_inner());
        if jobs.try_send(Job { package: package.to_string(), dir, bytes }).is_ok() {
            *p.entry(package.to_string()).or_default() += 1;
            store.dirty = false;
            true
        } else {
            false
        }
    }

    pub fn queue_final(&self, package: &str, store: &mut Store) -> bool {
        let deadline = std::time::Instant::now() + FINAL_WAIT;
        loop {
            if self.queue(package, store) {
                return true;
            }
            if std::time::Instant::now() >= deadline || store.snapshot().is_none() {
                return false;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    pub fn results(&self) -> Vec<(String, Result<(), String>)> {
        let mut out = Vec::new();
        loop {
            match self.results.try_recv() {
                Ok(r) => out.push(r),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return out,
            }
        }
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        self.jobs = None;
        let _ = self.done.recv_timeout(FINAL_WAIT);
    }
}

#[cfg(test)]
mod tests;
