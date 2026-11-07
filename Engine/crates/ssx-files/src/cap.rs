use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::path::PathBuf;

use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};

use crate::Options;
use crate::error::Error;
use crate::folder::Folder;
use crate::sys::{self, FileId};
use crate::write::{replace_at, write_bytes};

#[derive(Clone, Copy)]
struct DirFolder<'a>(&'a Dir);

impl Folder for DirFolder<'_> {
    fn shown(&self, name: &OsStr) -> PathBuf {
        PathBuf::from(name)
    }

    fn shown_folder(&self) -> PathBuf {
        PathBuf::from(".")
    }

    fn create_new(&self, name: &OsStr, mode: u32) -> io::Result<File> {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true).follow(FollowSymlinks::No);
        #[cfg(unix)]
        cap_std::fs::OpenOptionsExt::mode(&mut options, mode);
        #[cfg(not(unix))]
        let _ = mode;
        Ok(self.0.open_with(name, &options)?.into_std())
    }

    fn open_regular(&self, name: &OsStr) -> io::Result<Option<File>> {
        if !self.0.symlink_metadata(name)?.is_file() {
            return Ok(None);
        }
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        #[cfg(unix)]
        cap_std::fs::OpenOptionsExt::custom_flags(&mut options, libc::O_NONBLOCK);
        let file = self.0.open_with(name, &options)?.into_std();
        Ok(sys::is_regular(&file)?.then_some(file))
    }

    #[cfg(unix)]
    fn id_of(&self, name: &OsStr) -> io::Result<Option<FileId>> {
        use cap_std::fs::MetadataExt;
        match self.0.symlink_metadata(name) {
            Ok(m) => Ok(m.is_file().then(|| FileId(m.dev(), m.ino()))),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    #[cfg(windows)]
    fn id_of(&self, name: &OsStr) -> io::Result<Option<FileId>> {
        use cap_std::fs::OpenOptionsExt;
        let mut options = OpenOptions::new();
        options.follow(FollowSymlinks::No);
        options
            .access_mode(windows_sys::Win32::Storage::FileSystem::FILE_READ_ATTRIBUTES)
            .share_mode(sys::SHARE_ALL)
            .custom_flags(sys::NO_FOLLOW);
        let file = match self.0.open_with(name, &options) {
            Ok(f) => f.into_std(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        if !sys::is_regular(&file)? {
            return Ok(None);
        }
        sys::id(&file).map(Some)
    }

    #[cfg(unix)]
    fn mode_of(&self, name: &OsStr) -> Option<u32> {
        use cap_std::fs::MetadataExt;
        self.0.symlink_metadata(name).ok().filter(cap_std::fs::Metadata::is_file).map(|m| m.mode() & 0o777)
    }

    #[cfg(windows)]
    fn mode_of(&self, _name: &OsStr) -> Option<u32> {
        None
    }

    #[cfg(unix)]
    fn rename_replace(&self, from: &OsStr, to: &OsStr) -> io::Result<()> {
        self.0.rename(from, self.0, to)
    }

    #[cfg(windows)]
    fn rename_replace(&self, from: &OsStr, to: &OsStr) -> io::Result<()> {
        let base = sys::handle_path(self.0)?;
        sys::rename_replace(&base.join(from), &base.join(to))
    }

    fn remove(&self, name: &OsStr) -> io::Result<()> {
        self.0.remove_file(name)
    }

    #[cfg(unix)]
    fn sync(&self) -> io::Result<()> {
        self.0.open_with(".", OpenOptions::new().read(true))?.sync_all()
    }

    #[cfg(windows)]
    fn sync(&self) -> io::Result<()> {
        sys::sync_folder(std::path::Path::new("."))
    }
}

pub fn replace_in(dir: &Dir, name: &str, bytes: &[u8], opts: &Options) -> Result<(), Error> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
        let why = "not one name in the folder";
        return Err(Error::new(name, io::Error::new(io::ErrorKind::InvalidInput, why)));
    }
    replace_at(&DirFolder(dir), OsStr::new(name), opts, write_bytes(bytes))
}
