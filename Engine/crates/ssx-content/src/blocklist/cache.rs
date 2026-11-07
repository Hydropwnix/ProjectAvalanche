use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{FUTURE_SLACK, MAX_ENTRIES};
use crate::hash::ContentHash;

pub const MAGIC: [u8; 8] = *b"SSXABLK\0";
pub const FORMAT: u32 = 1;
pub const MAX_MASTER: usize = 256;
pub const MAX_ETAG: usize = 128;
const FIXED: usize = 8 + 4 + 8 + 8 + 2 + 1 + 4 + 32;
pub const MAX_BYTES: usize = FIXED + MAX_MASTER + MAX_ETAG + 32 * MAX_ENTRIES;
const EXTENSION: &str = "blocklist";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cached {
    pub master: String,
    pub etag: Option<String>,
    pub fetched: u64,
    pub checked: u64,
    pub hashes: Vec<ContentHash>,
}

fn printable(s: &str, max: usize, what: &str) -> Result<(), String> {
    if s.len() > max || !s.bytes().all(|b| (0x21..=0x7E).contains(&b)) {
        return Err(format!("{what}: at most {max} printable ASCII characters"));
    }
    Ok(())
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        if self.0.len() < n {
            return Err("cut short".into());
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(head)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], String> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    fn text(&mut self, n: usize, max: usize, what: &str) -> Result<String, String> {
        let s = std::str::from_utf8(self.take(n)?).map_err(|_| format!("{what}: not text"))?;
        printable(s, max, what)?;
        Ok(s.to_string())
    }
}

impl Cached {
    pub fn validate(&self) -> Result<(), String> {
        if self.master.is_empty() {
            return Err("no master".into());
        }
        if self.checked < self.fetched {
            return Err(format!("checked at {} before its list was fetched at {}", self.checked, self.fetched));
        }
        printable(&self.master, MAX_MASTER, "the master")?;
        if let Some(e) = &self.etag {
            if e.is_empty() {
                return Err("an empty ETag".into());
            }
            printable(e, MAX_ETAG, "the ETag")?;
        }
        super::List::new(String::new(), self.hashes.clone()).map(|_| ())
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let etag = self.etag.as_deref().unwrap_or("");
        let mut out = Vec::with_capacity(FIXED + self.master.len() + etag.len() + 32 * self.hashes.len());
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&FORMAT.to_le_bytes());
        out.extend_from_slice(&self.fetched.to_le_bytes());
        out.extend_from_slice(&self.checked.to_le_bytes());
        out.extend_from_slice(&u16::try_from(self.master.len()).map_err(|e| e.to_string())?.to_le_bytes());
        out.extend_from_slice(self.master.as_bytes());
        out.push(u8::try_from(etag.len()).map_err(|e| e.to_string())?);
        out.extend_from_slice(etag.as_bytes());
        out.extend_from_slice(&u32::try_from(self.hashes.len()).map_err(|e| e.to_string())?.to_le_bytes());
        for h in &self.hashes {
            out.extend_from_slice(&h.0);
        }
        let sum = blake3::hash(&out);
        out.extend_from_slice(sum.as_bytes());
        Ok(out)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_BYTES {
            return Err(format!("{} bytes, a kept list is at most {MAX_BYTES}", bytes.len()));
        }
        let Some(body_len) = bytes.len().checked_sub(32) else { return Err("cut short".into()) };
        let (body, sum) = bytes.split_at(body_len);
        if blake3::hash(body).as_bytes() != sum {
            return Err("its checksum does not match".into());
        }
        let mut r = Reader(body);
        if r.array::<8>()? != MAGIC {
            return Err("not a kept blocklist".into());
        }
        let format = u32::from_le_bytes(r.array()?);
        if format != FORMAT {
            return Err(format!("format {format}, this program reads {FORMAT}"));
        }
        let fetched = u64::from_le_bytes(r.array()?);
        let checked = u64::from_le_bytes(r.array()?);
        let master_len = usize::from(u16::from_le_bytes(r.array()?));
        if master_len == 0 || master_len > MAX_MASTER {
            return Err(format!("a master's address of {master_len} bytes"));
        }
        let master = r.text(master_len, MAX_MASTER, "the master")?;
        let etag_len = usize::from(u8::from_le_bytes(r.array()?));
        if etag_len > MAX_ETAG {
            return Err(format!("an ETag of {etag_len} bytes"));
        }
        let etag = r.text(etag_len, MAX_ETAG, "the ETag")?;
        let count = u32::from_le_bytes(r.array()?);
        let count = usize::try_from(count).map_err(|e| e.to_string())?;
        if count > MAX_ENTRIES {
            return Err(format!("{count} entries, a list holds at most {MAX_ENTRIES}"));
        }
        if r.0.len() != 32 * count {
            return Err(format!("{} bytes for {count} hashes", r.0.len()));
        }
        let (whole, rest) = r.0.as_chunks::<32>();
        if !rest.is_empty() {
            return Err("cut short".into());
        }
        let hashes: Vec<ContentHash> = whole.iter().map(|c| ContentHash(*c)).collect();
        let out = Cached { master, etag: (!etag.is_empty()).then_some(etag), fetched, checked, hashes };
        out.validate()?;
        Ok(out)
    }

    pub fn age(&self, now: u64) -> Result<Duration, String> {
        let latest = self.fetched.max(self.checked);
        if latest > now.saturating_add(FUTURE_SLACK.as_secs()) {
            return Err(format!("its time is {} s past this machine's clock", latest - now));
        }
        Ok(Duration::from_secs(now.saturating_sub(self.checked)))
    }
}

pub fn file_name(master: &str) -> String {
    let mut h = blake3::Hasher::new();
    h.update(b"SSXA blocklist cache v1\0");
    h.update(master.as_bytes());
    let hex: String = h.finalize().as_bytes()[..16].iter().map(|b| format!("{b:02x}")).collect();
    format!("{hex}.{EXTENSION}")
}

pub fn path_of(dir: &Path, master: &str) -> PathBuf {
    dir.join(file_name(master))
}

pub fn read(dir: &Path, master: &str) -> Result<Option<Cached>, String> {
    let path = path_of(dir, master);
    let file = match ssx_files::open_plain(&path) {
        Ok(Some(f)) => f,
        Ok(None) => return Ok(None),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let mut bytes = Vec::new();
    file.take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    let cached = Cached::from_bytes(&bytes)?;
    if cached.master != master {
        return Err("it holds another master's list".into());
    }
    Ok(Some(cached))
}

pub fn write(dir: &Path, c: &Cached) -> Result<(), String> {
    let bytes = c.to_bytes()?;
    ssx_files::create_dir_all(dir).map_err(|e| e.to_string())?;
    ssx_files::replace(&path_of(dir, &c.master), &bytes, &ssx_files::Options::new()).map_err(|e| e.to_string())
}
