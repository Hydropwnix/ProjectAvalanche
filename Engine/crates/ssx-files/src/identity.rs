use std::fs::File;
use std::path::Path;

use crate::error::{Error, at};
use crate::sys::{self, FileId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identity(FileId);

impl Identity {
    pub fn of(path: &Path) -> Result<Option<Identity>, Error> {
        at(|| path.to_path_buf(), sys::id_of(path)).map(|id| id.map(Identity))
    }

    pub fn of_file(file: &File) -> std::io::Result<Identity> {
        sys::id(file).map(Identity)
    }
}

pub fn open_plain(path: &Path) -> Result<Option<File>, Error> {
    at(|| path.to_path_buf(), sys::open_regular(path))
}
