use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use ssx_formats::big::BigArchive;
use ssx_formats::chunk;
use ssx_formats::xdvdfs::{Disc, Extent, Layout};
use ssx_formats::xex::Xex;

pub const ARCHIVES: [&str; 14] = [
    "data0",
    "bootcache",
    "memresident",
    "cache0",
    "cache1",
    "cache3",
    "cache4",
    "cache5",
    "cache6",
    "cache7",
    "cache8",
    "audiobestream",
    "feaudiocache",
    "fecache",
];

pub const SSX_PE_NAME: &str = "ssxvzf.exe";
pub const MARKER: &str = "setup.json";
const MARKER_FORMAT: u64 = 2;
pub const MOVIE_DIR: &str = "data/common/fmv";
const COPY_CHUNK: usize = 1 << 20;

pub fn big_root(out: &Path) -> PathBuf {
    out.join("big")
}

pub fn disc_root(out: &Path) -> PathBuf {
    out.join("disc")
}

pub fn app_dir() -> PathBuf {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    if let Some(home) = env("PROJECT_AVALANCHE_HOME") {
        return home;
    }
    if cfg!(windows) {
        let base = env("LOCALAPPDATA").or_else(|| env("USERPROFILE").map(|h| h.join("AppData").join("Local")));
        return base.unwrap_or_else(|| PathBuf::from(".")).join("Project Avalanche");
    }
    if cfg!(target_os = "macos") {
        let home = env("HOME").unwrap_or_else(|| PathBuf::from("."));
        return home.join("Library").join("Application Support").join("Project Avalanche");
    }
    let base = env("XDG_DATA_HOME").or_else(|| env("HOME").map(|h| h.join(".local").join("share")));
    base.unwrap_or_else(|| PathBuf::from(".")).join("project-avalanche")
}

pub fn default_data_dir() -> PathBuf {
    app_dir().join("GameData")
}

#[derive(Debug)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("setup cancelled")
    }
}

impl std::error::Error for Cancelled {}

#[derive(Clone, Debug)]
pub struct DiscCheck {
    pub layout: Layout,
    pub image_size: u64,
    pub files: usize,
    pub pe_name: String,
    pub archives: Vec<(String, u64)>,
    pub movies: Vec<(String, u64)>,
    pub fingerprint: String,
}

impl DiscCheck {
    pub fn archive_bytes(&self) -> u64 {
        self.archives.iter().map(|(_, n)| n).sum()
    }

    pub fn movie_bytes(&self) -> u64 {
        self.movies.iter().map(|(_, n)| n).sum()
    }
}

#[derive(Clone, Debug, Default)]
pub struct Progress {
    pub archive: String,
    pub archive_index: usize,
    pub archive_count: usize,
    pub files_done: usize,
    pub files_total: usize,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub failed: usize,
}

#[derive(Clone, Debug, Default)]
pub struct Summary {
    pub extracted: Vec<String>,
    pub already_done: Vec<String>,
    pub members_written: usize,
    pub members_kept: usize,
    pub bytes_written: u64,
    pub failures: Vec<String>,
    pub movies_written: usize,
    pub movies_kept: usize,
}

struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Fnv(0xCBF2_9CE4_8422_2325)
    }

    fn update(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 ^ b as u64).wrapping_mul(0x0000_0100_0000_01B3);
        }
    }
}

fn open_disc(iso: &Path) -> Result<Disc<File>> {
    let file = File::open(iso).with_context(|| format!("opening {}", iso.display()))?;
    Disc::open(file).with_context(|| format!("{} is not an Xbox / Xbox 360 disc image (XDVDFS)", iso.display()))
}

pub fn inspect(iso: &Path) -> Result<DiscCheck> {
    check_disc(&mut open_disc(iso)?)
}

fn check_disc(disc: &mut Disc<File>) -> Result<DiscCheck> {
    let xex_extent = disc.find("default.xex").context("the image has no default.xex — not a game disc")?.extent();
    let xex = disc.read_extent(xex_extent)?;
    let pe_name =
        Xex::parse(&xex).context("default.xex is not a XEX2 executable")?.original_pe_name()?.unwrap_or_default();
    if !pe_name.eq_ignore_ascii_case(SSX_PE_NAME) {
        bail!("this is not SSX (2012): default.xex was built as {pe_name:?}, expected {SSX_PE_NAME:?}");
    }
    let mut archives = Vec::new();
    for name in ARCHIVES {
        let extent =
            disc.find(&format!("{name}.big")).with_context(|| format!("the image has no {name}.big"))?.extent();
        let mut magic = [0u8; 2];
        disc.reader(extent)?.read_exact(&mut magic).with_context(|| format!("reading {name}.big"))?;
        if &magic != b"EB" {
            bail!("{name}.big is not an EB archive");
        }
        archives.push((name.to_string(), extent.size as u64));
    }
    let movies = movie_files(disc).iter().map(|(p, e)| (p.clone(), e.size as u64)).collect::<Vec<_>>();
    if movies.is_empty() {
        bail!("the image has no movies in {MOVIE_DIR} (*.vp6)");
    }
    let mut h = Fnv::new();
    h.update(&disc.image_size.to_le_bytes());
    h.update(&disc.volume.root_sector.to_le_bytes());
    h.update(&disc.volume.root_size.to_le_bytes());
    h.update(&disc.volume.filetime.to_le_bytes());
    h.update(&disc.volume.unk_reserved);
    for n in &disc.nodes {
        h.update(n.path.as_bytes());
        h.update(&n.entry.extent.sector.to_le_bytes());
        h.update(&n.entry.extent.size.to_le_bytes());
        h.update(&[n.entry.attributes]);
    }
    h.update(&xex[..xex.len().min(0x10000)]);
    Ok(DiscCheck {
        layout: disc.layout,
        image_size: disc.image_size,
        files: disc.files().count(),
        pe_name,
        archives,
        movies,
        fingerprint: format!("{:016x}", h.0),
    })
}

fn movie_files(disc: &Disc<File>) -> Vec<(String, Extent)> {
    let dir = format!("{MOVIE_DIR}/");
    disc.files()
        .filter(|n| {
            let p = n.path.to_ascii_lowercase();
            p.strip_prefix(&dir).is_some_and(|name| !name.contains('/') && name.ends_with(".vp6"))
        })
        .map(|n| (n.path.clone(), n.extent()))
        .collect()
}

pub fn read_marker(out: &Path) -> Option<Value> {
    serde_json::from_str(&fs::read_to_string(out.join(MARKER)).ok()?).ok()
}

pub fn is_complete(out: &Path) -> bool {
    read_marker(out).is_some_and(|m| m["complete"] == json!(true) && m["format"] == json!(MARKER_FORMAT))
}

pub fn needs_update(out: &Path) -> bool {
    read_marker(out).is_some_and(|m| {
        m["complete"] == json!(true) && m["format"].as_u64().is_some_and(|f| (1..MARKER_FORMAT).contains(&f))
    })
}

fn write_marker(out: &Path, marker: &Value) -> Result<()> {
    let tmp = out.join(format!("{MARKER}.partial"));
    fs::write(&tmp, serde_json::to_string_pretty(marker)?)?;
    fs::rename(&tmp, out.join(MARKER))?;
    Ok(())
}

fn safe_relative(path: &str) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for part in path.split('/') {
        let bad = |c: char| c.is_control() || "\\:*?\"<>|".contains(c);
        if part.is_empty() || part == "." || part == ".." || part.chars().any(bad) {
            return None;
        }
        out.push(part);
    }
    Some(out)
}

fn read_index(disc: &mut Disc<File>, name: &str, extent: Extent) -> Result<BigArchive> {
    let mut r = disc.reader(extent)?;
    let mut head = [0u8; 0x30];
    r.read_exact(&mut head)?;
    let size = BigArchive::table_size(&head)?;
    if size as u64 > extent.size as u64 {
        bail!("{name}.big: tables ({size} bytes) are larger than the archive");
    }
    let mut table = vec![0u8; size];
    r.seek(SeekFrom::Start(0))?;
    r.read_exact(&mut table)?;
    BigArchive::parse(&table).with_context(|| format!("parsing {name}.big"))
}

pub fn run(iso: &Path, out: &Path, cancel: &AtomicBool, progress: &mut dyn FnMut(&Progress)) -> Result<Summary> {
    let mut disc = open_disc(iso)?;
    let check = check_disc(&mut disc)?;
    fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    let same_disc = |m: &Value| {
        m["format"].as_u64().is_some_and(|f| (1..=MARKER_FORMAT).contains(&f))
            && m["fingerprint"] == json!(check.fingerprint)
            && m["iso_size"] == json!(check.image_size)
    };
    let previous = read_marker(out).filter(same_disc);
    let resume = previous.is_some();
    let mut finished = previous.and_then(|m| m["finished"].as_object().cloned()).unwrap_or_default();
    let mut summary = Summary::default();
    let mut todo = Vec::new();
    for name in ARCHIVES {
        if finished.contains_key(name) && big_root(out).join(name).is_dir() {
            summary.already_done.push(name.to_string());
            continue;
        }
        finished.remove(name);
        let extent = disc.find(&format!("{name}.big")).context("archive vanished")?.extent();
        let big = read_index(&mut disc, name, extent)?;
        todo.push((name, extent, big));
    }
    let mut marker = json!({
        "format": MARKER_FORMAT,
        "iso": iso.display().to_string(),
        "iso_size": check.image_size,
        "fingerprint": check.fingerprint,
        "layout": format!("{:?}", check.layout),
        "finished": finished,
        "complete": false,
    });
    write_marker(out, &marker)?;

    let movies = movie_files(&disc);
    let member_bytes: u64 = todo.iter().map(|(_, _, b)| b.entries.iter().map(|e| e.size as u64).sum::<u64>()).sum();
    let p = Progress {
        archive_count: todo.len() + 1,
        files_total: todo.iter().map(|(_, _, b)| b.entries.len()).sum::<usize>() + movies.len(),
        bytes_total: member_bytes + movies.iter().map(|(_, e)| e.size as u64).sum::<u64>(),
        ..Default::default()
    };
    let mut job = Job { disc: &mut disc, resume, cancel, p, progress, summary: &mut summary };
    for (i, (name, extent, big)) in todo.iter().enumerate() {
        job.p.archive = name.to_string();
        job.p.archive_index = i;
        (job.progress)(&job.p);
        let counts = job.archive(name, *extent, big, &big_root(out).join(name))?;
        finished.insert(name.to_string(), counts);
        marker["finished"] = Value::Object(finished.clone());
        write_marker(out, &marker)?;
        job.summary.extracted.push(name.to_string());
    }
    job.p.archive = "movies".to_string();
    job.p.archive_index = todo.len();
    (job.progress)(&job.p);
    marker["movies"] = job.movies(&movies, &disc_root(out))?;
    marker["complete"] = json!(true);
    write_marker(out, &marker)?;
    Ok(summary)
}

struct Job<'a> {
    disc: &'a mut Disc<File>,
    resume: bool,
    cancel: &'a AtomicBool,
    p: Progress,
    progress: &'a mut dyn FnMut(&Progress),
    summary: &'a mut Summary,
}

impl Job<'_> {
    fn check_cancel(&self) -> Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(anyhow::Error::new(Cancelled));
        }
        Ok(())
    }

    fn movies(&mut self, list: &[(String, Extent)], root: &Path) -> Result<Value> {
        let mut list: Vec<_> = list.iter().collect();
        list.sort_by_key(|(_, e)| e.sector);
        let (mut written, mut kept, mut bytes) = (0usize, 0usize, 0u64);
        let mut buf = vec![0u8; COPY_CHUNK];
        for (path, extent) in list {
            self.check_cancel()?;
            let rel = safe_relative(path).with_context(|| format!("unusable movie path {path:?}"))?;
            let dest = root.join(rel);
            let size = extent.size as u64;
            let have = fs::metadata(&dest).ok().filter(|m| m.is_file()).map(|m| m.len());
            if self.resume && have == Some(size) {
                kept += 1;
            } else {
                fs::create_dir_all(dest.parent().context("movie path has a parent")?)?;
                let mut tmp_name = dest.file_name().context("movie has a name")?.to_os_string();
                tmp_name.push(".partial");
                let tmp = dest.with_file_name(tmp_name);
                let mut r = self.disc.reader(*extent)?;
                let mut f = File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
                let base = self.p.bytes_done;
                let mut left = extent.size as usize;
                while left > 0 {
                    if self.cancel.load(Ordering::Relaxed) {
                        drop(f);
                        let _ = fs::remove_file(&tmp);
                        return Err(anyhow::Error::new(Cancelled));
                    }
                    let n = left.min(COPY_CHUNK);
                    r.read_exact(&mut buf[..n])?;
                    f.write_all(&buf[..n]).with_context(|| format!("writing {}", tmp.display()))?;
                    left -= n;
                    self.p.bytes_done = base + (extent.size as usize - left) as u64;
                    (self.progress)(&self.p);
                }
                drop(f);
                fs::rename(&tmp, &dest)?;
                self.p.bytes_done = base;
                written += 1;
                bytes += size;
            }
            self.p.files_done += 1;
            self.p.bytes_done += size;
            (self.progress)(&self.p);
        }
        self.summary.movies_written += written;
        self.summary.movies_kept += kept;
        self.summary.bytes_written += bytes;
        Ok(json!({ "files": written + kept, "written": written, "kept": kept, "bytes": bytes }))
    }

    fn archive(&mut self, name: &str, extent: Extent, big: &BigArchive, root: &Path) -> Result<Value> {
        let mut entries: Vec<_> = big.entries.iter().collect();
        entries.sort_by_key(|e| e.offset);
        let (mut written, mut kept, mut bytes) = (0usize, 0usize, 0u64);
        let mut failures = Vec::new();
        let mut buf = vec![0u8; COPY_CHUNK];
        for e in entries {
            self.check_cancel()?;
            let path = e.path();
            if e.offset + e.size as u64 > extent.size as u64 {
                bail!("{name}.big: {path} ends past the archive");
            }
            let rel = safe_relative(&path).with_context(|| format!("{name}.big: unusable member path {path:?}"))?;
            let dest = root.join(rel);
            let mut r = self.disc.reader(extent)?;
            r.seek(SeekFrom::Start(e.offset))?;
            let mut head = vec![0u8; (e.size as usize).min(28)];
            r.read_exact(&mut head)?;
            let container = chunk::detect(&head).is_some();
            let size = if container {
                chunk::read_header(&head).ok().map(|h| h.full_size as u64)
            } else {
                Some(e.size as u64)
            };
            let have = fs::metadata(&dest).ok().filter(|m| m.is_file()).map(|m| m.len());
            if self.resume && size.is_some() && have == size {
                kept += 1;
            } else {
                fs::create_dir_all(dest.parent().context("member path has a parent")?)?;
                let mut tmp_name = dest.file_name().context("member has a name")?.to_os_string();
                tmp_name.push(".partial");
                let tmp = dest.with_file_name(tmp_name);
                r.seek(SeekFrom::Start(e.offset))?;
                if container {
                    let mut stored = vec![0u8; e.size as usize];
                    r.read_exact(&mut stored)?;
                    match chunk::decompress(&stored) {
                        Ok(data) => {
                            fs::write(&tmp, &data).with_context(|| format!("writing {}", tmp.display()))?;
                            fs::rename(&tmp, &dest)?;
                            written += 1;
                            bytes += data.len() as u64;
                        }
                        Err(err) => failures.push(format!("{path}: {err}")),
                    }
                } else {
                    let mut f = File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
                    let base = self.p.bytes_done;
                    let mut left = e.size as usize;
                    while left > 0 {
                        if self.cancel.load(Ordering::Relaxed) {
                            drop(f);
                            let _ = fs::remove_file(&tmp);
                            return Err(anyhow::Error::new(Cancelled));
                        }
                        let n = left.min(COPY_CHUNK);
                        r.read_exact(&mut buf[..n])?;
                        f.write_all(&buf[..n]).with_context(|| format!("writing {}", tmp.display()))?;
                        left -= n;
                        self.p.bytes_done = base + (e.size as usize - left) as u64;
                        (self.progress)(&self.p);
                    }
                    drop(f);
                    fs::rename(&tmp, &dest)?;
                    self.p.bytes_done = base;
                    written += 1;
                    bytes += e.size as u64;
                }
            }
            self.p.files_done += 1;
            self.p.bytes_done += e.size as u64;
            self.p.failed = self.summary.failures.len() + failures.len();
            (self.progress)(&self.p);
        }
        self.summary.members_written += written;
        self.summary.members_kept += kept;
        self.summary.bytes_written += bytes;
        self.summary.failures.extend(failures.iter().map(|f| format!("{name}: {f}")));
        Ok(
            json!({ "members": big.entries.len(), "written": written, "kept": kept, "bytes": bytes, "failures": failures }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn member_paths_stay_inside_the_output() {
        assert_eq!(safe_relative("data/xenon/a.bin"), Some(Path::new("data").join("xenon").join("a.bin")));
        for bad in ["../x", "a/../../x", "a//b", "/abs", "c:x", "a\\b", "a/./b", "nul\0", "x?"] {
            assert_eq!(safe_relative(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn an_older_markers_folder_needs_the_update() {
        let dir = std::env::temp_dir().join(format!("ssx-setup-marker-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let states =
            [(1, true, false, true), (2, true, true, false), (1, false, false, false), (2, false, false, false)];
        for (format, complete, want_complete, want_update) in states {
            write_marker(&dir, &json!({ "format": format, "complete": complete })).unwrap();
            assert_eq!(is_complete(&dir), want_complete, "format {format} complete {complete}");
            assert_eq!(needs_update(&dir), want_update, "format {format} complete {complete}");
        }
        fs::remove_dir_all(&dir).unwrap();
        assert!(!is_complete(&dir) && !needs_update(&dir), "no marker");
    }

    #[test]
    fn fingerprint_hash_is_fnv1a_64() {
        let mut h = Fnv::new();
        h.update(b"a");
        assert_eq!(h.0, 0xAF63_DC4C_8601_EC8C);
    }
}
