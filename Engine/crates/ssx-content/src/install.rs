use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions};

use crate::check::{Finding, Severity, check_package};
use crate::hash::{ContentHash, ContentIndex};
use crate::limits::{MAX_FILE_BYTES, MAX_PACKAGE_BYTES};
use crate::manifest::{MANIFEST_FILE, Manifest};
use crate::path::{EntryKind, PackageTree, check_package_path};
use crate::texture;
use crate::version::Version;
use crate::zip::{INDEX_FILE, ZipArchive, ZipError, ZipLimits};

const MAX_INDEX_BYTES: u64 = 64 << 20;
pub const STAGING_PREFIX: &str = ".install-";
pub const REPLACED_PREFIX: &str = ".replaced-";

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("{}", ssx_core::text::shown_max(.0, ssx_core::text::LINE_CHARS))]
    Refused(String),
    #[error("{name} {} is already installed in {}: replace it to install this one", .version.as_ref().map_or("(unreadable)".to_string(), ToString::to_string), ssx_core::text::shown_max(&.folder.to_string_lossy(), ssx_core::text::PATH_CHARS))]
    Exists { name: String, version: Option<Version>, folder: PathBuf },
    #[error("the package does not pass its check:\n{}", .0.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n"))]
    Check(Vec<Finding>),
    #[error("{}", ssx_core::text::Text::new(.0))]
    Io(#[from] std::io::Error),
}

impl From<ZipError> for InstallError {
    fn from(e: ZipError) -> Self {
        match e {
            ZipError::Refused(m) => InstallError::Refused(m),
            ZipError::Io(e) => InstallError::Io(e),
        }
    }
}

fn refused(m: impl Into<String>) -> InstallError {
    InstallError::Refused(m.into())
}

#[derive(Debug)]
pub struct Installed {
    pub folder: String,
    pub manifest: Manifest,
    pub files: usize,
    pub bytes: u64,
    pub findings: Vec<Finding>,
    pub indexed: bool,
    pub previous: Option<(String, PathBuf)>,
}

struct Staging {
    path: PathBuf,
    dir: Option<Dir>,
    keep: bool,
    made: Option<PathBuf>,
}

impl Staging {
    fn new(mods: &Path) -> Result<Self, InstallError> {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let made = (!mods.exists()).then(|| mods.to_path_buf());
        ssx_files::create_dir_all(mods).map_err(std::io::Error::from)?;
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = mods.join(format!("{STAGING_PREFIX}{}-{nanos}-{n}", std::process::id()));
        std::fs::create_dir(&path)?;
        let dir = Dir::open_ambient_dir(&path, ambient_authority())?;
        Ok(Staging { path, dir: Some(dir), keep: false, made })
    }

    fn create(&self, rel: &str) -> Result<cap_std::fs::File, InstallError> {
        let dir = self.dir.as_ref().ok_or_else(|| refused("the staging folder is closed"))?;
        if let Some((parent, _)) = rel.rsplit_once('/') {
            dir.create_dir_all(parent)?;
        }
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        Ok(dir.open_with(rel, &options)?)
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.path);
            if let Some(mods) = &self.made {
                let _ = std::fs::remove_dir(mods);
            }
        }
    }
}

fn package_prefix(names: &[(String, bool)]) -> Result<String, InstallError> {
    if names.iter().any(|(n, folder)| n == MANIFEST_FILE && !folder) {
        return Ok(String::new());
    }
    let mut tops: Vec<&str> = names.iter().map(|(n, _)| n.split('/').next().unwrap_or("")).collect();
    tops.sort_unstable();
    tops.dedup();
    if let [top] = tops[..] {
        let inner = format!("{top}/{MANIFEST_FILE}");
        if names.iter().any(|(n, folder)| *n == inner && !folder) {
            return Ok(format!("{top}/"));
        }
    }
    Err(refused(format!("no {MANIFEST_FILE} at the archive's root or in its one top folder: not a package")))
}

pub fn install_zip(zip: &Path, mods: &Path, replace: bool) -> Result<Installed, InstallError> {
    let file = std::fs::File::open(zip)?;
    let mut archive = ZipArchive::open(std::io::BufReader::new(file), &ZipLimits::default())?;
    let names: Vec<(String, bool)> = archive.entries().iter().map(|e| (e.name.clone(), e.folder)).collect();
    let prefix = package_prefix(&names)?;
    let staging = Staging::new(mods)?;
    let mut got: BTreeMap<String, (u64, ContentHash)> = BTreeMap::new();
    let mut index = None;
    for (i, (name, folder)) in names.iter().enumerate() {
        let Some(rel) = name.strip_prefix(&prefix) else { continue };
        if *folder || rel.is_empty() {
            archive.extract(i, &mut std::io::sink())?;
            continue;
        }
        if rel == INDEX_FILE {
            let text = archive.read(i, MAX_INDEX_BYTES)?;
            let text = String::from_utf8(text).map_err(|_| refused(format!("{INDEX_FILE}: not UTF-8")))?;
            index = Some(ContentIndex::from_tsv(&text).map_err(|e| refused(format!("{INDEX_FILE}: {e}")))?);
            continue;
        }
        let mut out = std::io::BufWriter::new(staging.create(rel)?);
        let hash = archive.extract(i, &mut out)?;
        out.flush()?;
        let size = archive.entries()[i].size;
        got.insert(rel.to_string(), (size, hash));
    }
    let indexed = index.is_some();
    if let Some(index) = index {
        compare_index(&index, &got)?;
    }
    finish(staging, mods, replace, got, indexed)
}

fn compare_index(index: &ContentIndex, got: &BTreeMap<String, (u64, ContentHash)>) -> Result<(), InstallError> {
    for (path, want) in &index.entries {
        check_package_path(path, false).map_err(|e| refused(format!("{INDEX_FILE}: {e}")))?;
        match got.get(path) {
            None => return Err(refused(format!("{INDEX_FILE} lists {path}, which the archive does not hold"))),
            Some(have) if have != want => {
                return Err(refused(format!("{path}: its size or BLAKE3 differs from {INDEX_FILE}'s")));
            }
            Some(_) => {}
        }
    }
    if let Some(extra) = got.keys().find(|p| !index.entries.contains_key(*p)) {
        return Err(refused(format!("{extra}: in the archive, not in its {INDEX_FILE}")));
    }
    Ok(())
}

pub fn install_folder(src: &Path, mods: &Path, replace: bool) -> Result<Installed, InstallError> {
    let tree = PackageTree::open(src).map_err(|e| refused(e.to_string()))?;
    if let Some(name) = tree.unnamed_names.first() {
        return Err(refused(format!(
            "{}: a name packages cannot use ({} such)",
            ssx_core::text::quoted(name),
            tree.unnamed
        )));
    }
    let mut total = 0u64;
    for (path, kind) in tree.entries() {
        match kind {
            EntryKind::Link => return Err(refused(format!("{path}: a link (packages hold files and folders only)"))),
            EntryKind::Other => return Err(refused(format!("{path}: a device, FIFO or socket"))),
            EntryKind::File if path == INDEX_FILE => {
                return Err(refused(format!("{INDEX_FILE}: a reserved name (a packed archive's index)")));
            }
            EntryKind::File => total += tree.size(path).unwrap_or(0),
            EntryKind::Folder => {}
        }
    }
    if total > MAX_PACKAGE_BYTES {
        return Err(refused(format!("{total} bytes (a package is at most {MAX_PACKAGE_BYTES})")));
    }
    let staging = Staging::new(mods)?;
    let mut got = BTreeMap::new();
    let files: Vec<String> = tree.files().map(str::to_string).collect();
    for rel in files {
        let mut out = std::io::BufWriter::new(staging.create(&rel)?);
        let got_file = tree.copy_to(&rel, MAX_FILE_BYTES, &mut out).map_err(|e| refused(e.to_string()))?;
        out.flush()?;
        got.insert(rel, got_file);
    }
    finish(staging, mods, replace, got, false)
}

fn finish(
    mut staging: Staging,
    mods: &Path,
    replace: bool,
    got: BTreeMap<String, (u64, ContentHash)>,
    indexed: bool,
) -> Result<Installed, InstallError> {
    let checked = check_package(&staging.path, &texture::Limits::default());
    if checked.errors() > 0 {
        let errors = checked.findings.into_iter().filter(|f| f.severity == Severity::Error).collect();
        return Err(InstallError::Check(errors));
    }
    let findings = checked.findings;
    drop(checked.tree);
    let manifest = crate::package::PackageSource::open(&staging.path).map_err(|e| refused(e.to_string()))?.manifest;
    staging.dir = None;
    ssx_files::sync_tree(&staging.path).map_err(std::io::Error::from)?;
    let name = manifest.resource.name.clone();
    let target = mods.join(&name);
    let existing = std::fs::read_dir(mods)?
        .flatten()
        .map(|e| e.file_name())
        .find(|n| n.to_str().is_some_and(|n| n.eq_ignore_ascii_case(&name)));
    let mut previous = None;
    if let Some(found) = existing {
        let folder = mods.join(&found);
        if !replace {
            let version = crate::package::PackageSource::open(&folder).ok().map(|p| p.manifest.resource.version);
            return Err(InstallError::Exists { name, version, folder });
        }
        let aside = mods.join(format!(
            "{REPLACED_PREFIX}{name}-{}",
            staging.path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned())
        ));
        ssx_files::rename_new(&folder, &aside).map_err(std::io::Error::from)?;
        previous = Some((found.to_string_lossy().into_owned(), aside));
    }
    if let Err(e) = ssx_files::rename_new(&staging.path, &target) {
        if let Some((old, aside)) = &previous {
            let _ = ssx_files::rename_new(aside, &mods.join(old));
        }
        return Err(std::io::Error::from(e).into());
    }
    staging.keep = true;
    let bytes = got.values().map(|(s, _)| s).sum();
    Ok(Installed { folder: name, manifest, files: got.len(), bytes, findings, indexed, previous })
}

#[cfg(test)]
mod tests;
