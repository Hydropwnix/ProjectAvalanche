use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub const FE_ROOT: &str = "data/common/fe/";

pub trait FileSource: Send + Sync {
    fn read(&self, path: &str) -> Option<Vec<u8>>;
    fn exists(&self, path: &str) -> bool;
}

pub struct Disc {
    archives: Vec<PathBuf>,
}

impl Disc {
    pub fn new(root: &Path) -> Result<Self> {
        let mut archives: Vec<PathBuf> = std::fs::read_dir(root)
            .with_context(|| format!("reading {}", root.display()))?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        archives.sort();
        Ok(Disc { archives })
    }

    pub fn find(&self, rel: &str) -> Option<PathBuf> {
        let rel = normalise(rel);
        self.archives.iter().map(|a| a.join(&rel)).find(|p| p.is_file())
    }

    pub fn list(&self, dir: &str, extension: &str) -> Vec<String> {
        let dir = normalise(dir);
        let mut out = std::collections::BTreeSet::new();
        for a in &self.archives {
            walk(&a.join(&dir), &mut |p| {
                if p.extension().is_some_and(|e| e.eq_ignore_ascii_case(extension))
                    && let Ok(rel) = p.strip_prefix(a)
                {
                    out.insert(rel.to_string_lossy().replace('\\', "/"));
                }
            });
        }
        out.into_iter().collect()
    }
}

fn walk(dir: &Path, f: &mut dyn FnMut(&Path)) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, f);
        } else {
            f(&p);
        }
    }
}

pub fn normalise(rel: &str) -> String {
    let s = rel.replace('\\', "/").to_lowercase();
    s.trim_start_matches("./").trim_start_matches('/').to_string()
}

impl FileSource for Disc {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        std::fs::read(self.find(path)?).ok()
    }

    fn exists(&self, path: &str) -> bool {
        self.find(path).is_some()
    }
}

pub fn join(dir: &str, url: &str) -> String {
    let mut parts: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
    let url = url.replace('\\', "/");
    for seg in url.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_relative_urls() {
        assert_eq!(join("views/hud", "../components/HUD/x.swf"), "views/components/HUD/x.swf");
        assert_eq!(join("", "components/a.swf"), "components/a.swf");
        assert_eq!(normalise("Data\\Common\\FE\\x.GFX"), "data/common/fe/x.gfx");
    }
}
