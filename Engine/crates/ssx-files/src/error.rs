use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct Error {
    path: PathBuf,
    source: io::Error,
}

impl Error {
    pub(crate) fn new(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Error { path: path.into(), source }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn kind(&self) -> io::ErrorKind {
        self.source.kind()
    }

    pub fn io(&self) -> &io::Error {
        &self.source
    }

    pub fn into_io(self) -> io::Error {
        self.source
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let path = self.path.to_string_lossy();
        f.write_str(&ssx_core::text::located(&path, &self.source))
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

impl From<Error> for io::Error {
    fn from(e: Error) -> Self {
        io::Error::new(e.source.kind(), e)
    }
}

pub(crate) fn at<T>(path: impl FnOnce() -> PathBuf, result: io::Result<T>) -> Result<T, Error> {
    result.map_err(|e| Error::new(path(), e))
}
