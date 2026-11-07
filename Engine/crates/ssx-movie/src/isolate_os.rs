#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
pub mod filter;
#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
mod linux;
#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
use linux as imp;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as imp;
#[cfg(not(any(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"), windows)))]
mod other;
#[cfg(not(any(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"), windows)))]
use other as imp;

pub use imp::{Process, Status, spawn};
pub(crate) use imp::{measure_from_here, prepare, seal};

pub const MEMORY_LIMIT: u64 = 768 << 20;

pub const MAX_THREADS: u16 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seal {
    Enforce,
    Measure,
}

pub struct Spawned {
    pub process: Process,
    pub stdin: std::fs::File,
    pub stdout: std::fs::File,
    pub stderr: std::fs::File,
}
