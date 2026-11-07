pub mod api;
mod call;
pub mod check;
pub mod compile;
pub mod cpu;
mod env;
pub mod host;
mod require;
pub mod text;
pub mod value;
mod vm;

pub use env::{MAX_LOADSTRING, MAX_OUTPUT_LINE, MAX_OUTPUT_LINES, Output};
pub use require::{FileMap, MAX_REQUIRE_NAME, resolve};
pub use vm::{Handler, Limits, Overrun, Script, ScriptVm};

pub const MAX_ERROR_LEN: usize = 8192;
pub const MAX_SOURCE: usize = 1 << 20;

#[derive(Debug, thiserror::Error)]
pub enum ScriptError {
    #[error("limits out of range: {0:?}")]
    BadLimits(Limits),
    #[error("the script ran past its call budget ({0}) and was stopped")]
    Budget(Overrun),
    #[error("the script went over its memory limit of {0} bytes and was stopped")]
    Memory(usize),
    #[error("the VM was stopped earlier: {0}")]
    Stopped(String),
    #[error("no file {0:?} in the package")]
    NoFile(String),
    #[error("the handle belongs to another VM")]
    ForeignHandle,
    #[error("source {0:?} is over {MAX_SOURCE} bytes")]
    SourceTooLarge(String),
    #[error("{0}")]
    Clock(String),
    #[error("{0}")]
    Script(String),
}

impl From<mlua::Error> for ScriptError {
    fn from(e: mlua::Error) -> Self {
        match e {
            mlua::Error::MismatchedRegistryKey => ScriptError::ForeignHandle,
            e => ScriptError::Script(text::display_bounded(&e, MAX_ERROR_LEN)),
        }
    }
}

#[cfg(test)]
mod tests;
