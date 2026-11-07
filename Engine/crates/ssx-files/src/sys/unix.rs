use std::fs::{File, Metadata, OpenOptions};
use std::io;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FileId(pub(crate) u64, pub(crate) u64);

pub fn id(file: &File) -> io::Result<FileId> {
    file.metadata().map(|m| id_of_metadata(&m))
}

pub fn id_of_metadata(m: &Metadata) -> FileId {
    FileId(m.dev(), m.ino())
}

#[cfg(feature = "cap-std")]
pub fn is_regular(file: &File) -> io::Result<bool> {
    Ok(file.metadata()?.is_file())
}

pub fn create_new(path: &Path, mode: u32) -> io::Result<File> {
    OpenOptions::new().read(true).write(true).create_new(true).mode(mode).custom_flags(libc::O_NOFOLLOW).open(path)
}

pub fn open_regular(path: &Path) -> io::Result<Option<File>> {
    if !std::fs::symlink_metadata(path)?.is_file() {
        return Ok(None);
    }
    let file = match OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path) {
        Ok(f) => f,
        Err(e) if e.raw_os_error() == Some(libc::ELOOP) => return Ok(None),
        Err(e) => return Err(e),
    };
    Ok(file.metadata()?.is_file().then_some(file))
}

pub fn id_of(path: &Path) -> io::Result<Option<FileId>> {
    match std::fs::symlink_metadata(path) {
        Ok(m) => Ok(m.is_file().then(|| id_of_metadata(&m))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

pub fn mode_of(path: &Path) -> Option<u32> {
    std::fs::symlink_metadata(path).ok().filter(Metadata::is_file).map(|m| m.mode() & 0o777)
}

pub fn rename_replace(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::rename(from, to)
}

pub fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    if native_noreplace(from, to)? {
        return Ok(());
    }
    if std::fs::symlink_metadata(from)?.is_dir() {
        if std::fs::symlink_metadata(to).is_ok() {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("{} exists", to.display())));
        }
        return std::fs::rename(from, to);
    }
    std::fs::hard_link(from, to)?;
    std::fs::remove_file(from)
}

#[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
fn c_path(p: &Path) -> io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(p.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "a NUL in the path"))
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn native_noreplace(from: &Path, to: &Path) -> io::Result<bool> {
    let (f, t) = (c_path(from)?, c_path(to)?);
    let r =
        unsafe { libc::renameat2(libc::AT_FDCWD, f.as_ptr(), libc::AT_FDCWD, t.as_ptr(), libc::RENAME_NOREPLACE as _) };
    if r == 0 {
        return Ok(true);
    }
    let e = io::Error::last_os_error();
    match e.raw_os_error() {
        Some(libc::EINVAL | libc::ENOSYS) => Ok(false),
        _ => Err(e),
    }
}

#[cfg(target_vendor = "apple")]
fn native_noreplace(from: &Path, to: &Path) -> io::Result<bool> {
    let (f, t) = (c_path(from)?, c_path(to)?);
    let r = unsafe { libc::renamex_np(f.as_ptr(), t.as_ptr(), libc::RENAME_EXCL) };
    if r == 0 {
        return Ok(true);
    }
    let e = io::Error::last_os_error();
    match e.raw_os_error() {
        Some(libc::EINVAL | libc::ENOTSUP) => Ok(false),
        _ => Err(e),
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android", target_vendor = "apple")))]
fn native_noreplace(_from: &Path, _to: &Path) -> io::Result<bool> {
    Ok(false)
}

pub fn sync_folder(dir: &Path) -> io::Result<()> {
    OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY).open(dir)?.sync_all()
}

pub fn sync_file(path: &Path) -> io::Result<()> {
    OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path)?.sync_all()
}
