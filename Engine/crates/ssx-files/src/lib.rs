#[cfg(feature = "cap-std")]
mod cap;
mod commit;
mod error;
mod folder;
mod identity;
mod slot;
mod sys;
#[cfg(test)]
mod tests;
mod tree;
mod write;

#[cfg(feature = "cap-std")]
pub use cap::replace_in;
pub use commit::{Committer, committing};
pub use error::Error;
pub use identity::{Identity, open_plain};
pub use slot::{SLOTS, partial_target};
pub use tree::{
    copy_new, create_dir_all, install_folder, keep_copy, move_aside, rename_new, rename_over, sync_folder, sync_tree,
};
pub use write::{Prepared, create, create_with, prepare_with, replace, replace_with, update};

#[derive(Clone, Copy, Debug)]
pub struct Options {
    mode: u32,
    sync_folder: bool,
}

impl Options {
    pub const fn new() -> Self {
        Options { mode: 0o666, sync_folder: true }
    }

    pub const fn owner_only() -> Self {
        Self::new().mode(0o600)
    }

    pub const fn mode(mut self, mode: u32) -> Self {
        self.mode = mode & 0o777;
        self
    }

    pub const fn folder_synced_by_caller(mut self) -> Self {
        self.sync_folder = false;
        self
    }
}

impl Default for Options {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Write,
    Sync,
    Rename,
    FolderSync,
}

#[cfg(test)]
mod fault {
    use std::cell::Cell;

    thread_local!(static FAIL: Cell<Option<super::Step>> = const { Cell::new(None) });

    pub fn set(step: Option<super::Step>) {
        FAIL.with(|f| f.set(step));
    }

    pub fn check(step: super::Step) -> std::io::Result<()> {
        match FAIL.with(Cell::get) {
            Some(s) if s == step => Err(std::io::Error::other(format!("a failure injected at {step:?}"))),
            _ => Ok(()),
        }
    }
}

#[cfg(not(test))]
mod fault {
    #[inline(always)]
    pub fn check(_step: super::Step) -> std::io::Result<()> {
        Ok(())
    }
}
