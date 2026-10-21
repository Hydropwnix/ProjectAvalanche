use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("unexpected end of data at offset {offset:#x} (wanted {wanted} bytes, {available} left)")]
    UnexpectedEof { offset: usize, wanted: usize, available: usize },

    #[error("bad magic at offset {offset:#x}: expected {expected:?}, found {found:?}")]
    BadMagic { offset: usize, expected: &'static [u8], found: Vec<u8> },

    #[error("{0}")]
    Invalid(String),

    #[error("decompression failed: {0}")]
    Decompress(String),

    #[error("container segment {segment} has no stored data but should decode to {expected} bytes (truncated member)")]
    EmptySegment { segment: usize, expected: usize },

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl Error {
    pub fn invalid(msg: impl Into<String>) -> Self {
        Error::Invalid(msg.into())
    }
}
