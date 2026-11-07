use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use crate::sys::{self, FileId};

pub(crate) trait Folder {
    fn shown(&self, name: &OsStr) -> PathBuf;
    fn shown_folder(&self) -> PathBuf;
    fn create_new(&self, name: &OsStr, mode: u32) -> io::Result<File>;
    fn open_regular(&self, name: &OsStr) -> io::Result<Option<File>>;
    fn id_of(&self, name: &OsStr) -> io::Result<Option<FileId>>;
    fn mode_of(&self, name: &OsStr) -> Option<u32>;
    fn rename_replace(&self, from: &OsStr, to: &OsStr) -> io::Result<()>;
    fn remove(&self, name: &OsStr) -> io::Result<()>;
    fn sync(&self) -> io::Result<()>;
}

impl<T: Folder + ?Sized> Folder for &T {
    fn shown(&self, name: &OsStr) -> PathBuf {
        (**self).shown(name)
    }

    fn shown_folder(&self) -> PathBuf {
        (**self).shown_folder()
    }

    fn create_new(&self, name: &OsStr, mode: u32) -> io::Result<File> {
        (**self).create_new(name, mode)
    }

    fn open_regular(&self, name: &OsStr) -> io::Result<Option<File>> {
        (**self).open_regular(name)
    }

    fn id_of(&self, name: &OsStr) -> io::Result<Option<FileId>> {
        (**self).id_of(name)
    }

    fn mode_of(&self, name: &OsStr) -> Option<u32> {
        (**self).mode_of(name)
    }

    fn rename_replace(&self, from: &OsStr, to: &OsStr) -> io::Result<()> {
        (**self).rename_replace(from, to)
    }

    fn remove(&self, name: &OsStr) -> io::Result<()> {
        (**self).remove(name)
    }

    fn sync(&self) -> io::Result<()> {
        (**self).sync()
    }
}

#[derive(Clone)]
pub(crate) struct PathFolder(PathBuf);

impl PathFolder {
    pub fn split(path: &Path) -> io::Result<(Self, &OsStr)> {
        let name = path
            .file_name()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, format!("{} names no file", path.display())))?;
        let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
        Ok((PathFolder(dir.to_path_buf()), name))
    }

    pub fn path(&self, name: &OsStr) -> PathBuf {
        self.0.join(name)
    }
}

impl Folder for PathFolder {
    fn shown(&self, name: &OsStr) -> PathBuf {
        self.path(name)
    }

    fn shown_folder(&self) -> PathBuf {
        self.0.clone()
    }

    fn create_new(&self, name: &OsStr, mode: u32) -> io::Result<File> {
        sys::create_new(&self.path(name), mode)
    }

    fn open_regular(&self, name: &OsStr) -> io::Result<Option<File>> {
        sys::open_regular(&self.path(name))
    }

    fn id_of(&self, name: &OsStr) -> io::Result<Option<FileId>> {
        sys::id_of(&self.path(name))
    }

    fn mode_of(&self, name: &OsStr) -> Option<u32> {
        sys::mode_of(&self.path(name))
    }

    fn rename_replace(&self, from: &OsStr, to: &OsStr) -> io::Result<()> {
        sys::rename_replace(&self.path(from), &self.path(to))
    }

    fn remove(&self, name: &OsStr) -> io::Result<()> {
        std::fs::remove_file(self.path(name))
    }

    fn sync(&self) -> io::Result<()> {
        sys::sync_folder(&self.0)
    }
}
