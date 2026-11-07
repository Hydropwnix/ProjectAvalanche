use std::path::{Path, PathBuf};

mod commits;
mod copies;
mod slots;
mod steps;
mod tree;
mod writers;

pub(super) struct Scratch(pub PathBuf);

impl Scratch {
    pub fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("ssx-files-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch folder");
        Scratch(dir)
    }

    pub fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    pub fn names(&self) -> Vec<String> {
        names(&self.0)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub(super) fn names(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .expect("a listing")
        .map(|e| e.expect("an entry").file_name().to_string_lossy().into_owned())
        .collect();
    out.sort();
    out
}

pub(super) fn failing_at<T>(step: crate::Step, f: impl FnOnce() -> T) -> T {
    crate::fault::set(Some(step));
    let out = f();
    crate::fault::set(None);
    out
}
