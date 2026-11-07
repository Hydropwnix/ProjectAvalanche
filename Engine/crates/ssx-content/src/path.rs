use std::collections::BTreeMap;
use std::io::{Read, Seek, Write};

use crate::hash::ContentHash;
use std::path::{Path, PathBuf};

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions};

pub const MAX_PATH_LEN: usize = 512;
pub const MAX_SEGMENT_LEN: usize = 128;

const RESERVED: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1", "lpt2",
    "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

fn name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '+' | '-')
}

fn glob_char(c: char) -> bool {
    matches!(c, '*' | '?' | '[' | ']')
}

pub fn check_package_path(p: &str, glob: bool) -> Result<(), String> {
    if p.is_empty() {
        return Err("empty path".into());
    }
    if p.len() > MAX_PATH_LEN {
        return Err(format!("path of {} bytes (at most {MAX_PATH_LEN})", p.len()));
    }
    let shown = ssx_core::text::quoted(p);
    if p.starts_with('/') {
        return Err(format!("{shown} is absolute (paths are relative to the package root)"));
    }
    for seg in p.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." {
            return Err(format!(
                "{shown} has a {} segment (paths stay inside the package)",
                ssx_core::text::quoted(seg)
            ));
        }
        if seg.len() > MAX_SEGMENT_LEN {
            return Err(format!("{shown} has a segment over {MAX_SEGMENT_LEN} bytes"));
        }
        if !glob && seg.chars().any(glob_char) {
            return Err(format!("{shown} names one file, not a pattern"));
        }
        if let Some(c) = seg.chars().find(|&c| !(name_char(c) || glob && glob_char(c))) {
            let hint = if c == '\\' { " (use `/` between folders)" } else { " (allowed: A-Z a-z 0-9 _ . + -)" };
            return Err(format!("{shown} contains {c:?}{hint}"));
        }
        if seg.ends_with('.') {
            return Err(format!("{shown} has a segment ending in '.'"));
        }
        let stem = seg.split('.').next().unwrap_or(seg).to_ascii_lowercase();
        if RESERVED.contains(&stem.as_str()) {
            return Err(format!("{shown} uses the reserved device name {}", ssx_core::text::quoted(&stem)));
        }
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("{}", ssx_core::text::Text::new(.0))]
    BadPath(String),
    #[error("{}", ssx_core::text::located(&.path.to_string_lossy(), .message))]
    Refused { path: PathBuf, message: String },
    #[error("{}", ssx_core::text::located(&.path.to_string_lossy(), .source))]
    Io { path: PathBuf, source: std::io::Error },
}

pub const MAX_PACKAGE_ENTRIES: usize = 65_536;
pub const MAX_PACKAGE_DEPTH: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Folder,
    Link,
    Other,
}

pub struct PackageTree {
    root_path: PathBuf,
    root: Dir,
    entries: BTreeMap<String, EntryKind>,
    sizes: BTreeMap<String, u64>,
    pub unnamed: usize,
    pub unnamed_names: Vec<String>,
}

pub const MAX_UNNAMED_SHOWN: usize = 64;

fn refused(path: &Path, message: impl Into<String>) -> ReadError {
    ReadError::Refused { path: path.to_path_buf(), message: message.into() }
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> ReadError + '_ {
    move |source| ReadError::Io { path: path.to_path_buf(), source }
}

fn check_handle(path: &Path, file: &std::fs::File) -> Result<std::fs::Metadata, ReadError> {
    let meta = file.metadata().map_err(io(path))?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(refused(path, "is a reparse point; packages may not contain links"));
        }
    }
    Ok(meta)
}

impl PackageTree {
    pub fn open(root: &Path) -> Result<Self, ReadError> {
        let dir = Dir::open_ambient_dir(root, ambient_authority()).map_err(io(root))?;
        let mut tree = PackageTree {
            root_path: root.to_path_buf(),
            root: dir,
            entries: BTreeMap::new(),
            sizes: BTreeMap::new(),
            unnamed: 0,
            unnamed_names: Vec::new(),
        };
        let mut pending = vec![String::new()];
        let mut seen = 0usize;
        while let Some(prefix) = pending.pop() {
            let folder = tree.open_folder(&prefix)?;
            let here = tree.root_path.join(&prefix);
            for entry in folder.entries().map_err(io(&here))? {
                let entry = entry.map_err(io(&here))?;
                seen += 1;
                if seen > MAX_PACKAGE_ENTRIES {
                    return Err(refused(root, format!("more than {MAX_PACKAGE_ENTRIES} entries")));
                }
                let name = entry.file_name();
                let lossy = || {
                    let n = name.to_string_lossy();
                    if prefix.is_empty() { n.into_owned() } else { format!("{prefix}/{n}") }
                };
                let Some(text) = name.to_str() else {
                    tree.unnamed(lossy());
                    continue;
                };
                let rel = if prefix.is_empty() { text.to_string() } else { format!("{prefix}/{text}") };
                if check_package_path(&rel, false).is_err() {
                    tree.unnamed(lossy());
                    continue;
                }
                let t = entry.file_type().map_err(io(&here))?;
                let kind = if t.is_symlink() {
                    EntryKind::Link
                } else if t.is_dir() {
                    EntryKind::Folder
                } else if t.is_file() {
                    EntryKind::File
                } else {
                    EntryKind::Other
                };
                if kind == EntryKind::Folder {
                    if rel.split('/').count() >= MAX_PACKAGE_DEPTH {
                        return Err(refused(
                            &here.join(name),
                            format!("folders nested deeper than {MAX_PACKAGE_DEPTH}"),
                        ));
                    }
                    pending.push(rel.clone());
                }
                if kind == EntryKind::File {
                    tree.sizes.insert(rel.clone(), entry.metadata().map_err(io(&here))?.len());
                }
                tree.entries.insert(rel, kind);
            }
        }
        Ok(tree)
    }

    fn open_folder(&self, rel: &str) -> Result<Dir, ReadError> {
        let mut dir = self.root.try_clone().map_err(io(&self.root_path))?;
        let mut at = self.root_path.clone();
        for seg in rel.split('/').filter(|s| !s.is_empty()) {
            at.push(seg);
            let next = dir.open_dir_nofollow(seg).map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => {
                    refused(&at, "no such folder (it changed after the package was opened)")
                }
                _ => refused(&at, format!("cannot be opened as a folder without following links: {e}")),
            })?;
            let std_dir = next.into_std_file();
            check_handle(&at, &std_dir)?;
            dir = Dir::from_std_file(std_dir);
        }
        Ok(dir)
    }

    fn unnamed(&mut self, name: String) {
        self.unnamed += 1;
        if self.unnamed_names.len() < MAX_UNNAMED_SHOWN {
            self.unnamed_names.push(name);
        }
    }

    pub fn entries(&self) -> impl Iterator<Item = (&str, EntryKind)> {
        self.entries.iter().map(|(p, k)| (p.as_str(), *k))
    }

    pub fn size(&self, rel: &str) -> Option<u64> {
        self.sizes.get(rel).copied()
    }

    pub fn root(&self) -> &Path {
        &self.root_path
    }

    pub fn kind(&self, rel: &str) -> Option<EntryKind> {
        self.entries.get(rel).copied()
    }

    pub fn files(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().filter(|(_, k)| **k == EntryKind::File).map(|(p, _)| p.as_str())
    }

    pub fn read(&self, rel: &str, max: u64) -> Result<Vec<u8>, ReadError> {
        let (file, path) = self.open_file(rel, max)?;
        let len = file.metadata().map_err(io(&path))?.len().min(max);
        let mut out = Vec::with_capacity(usize::try_from(len).unwrap_or(0));
        file.take(max.saturating_add(1)).read_to_end(&mut out).map_err(io(&path))?;
        if out.len() as u64 > max {
            return Err(refused(&path, format!("grew past {max} bytes while read")));
        }
        Ok(out)
    }

    pub fn file_size(&self, rel: &str) -> Result<u64, ReadError> {
        let (file, path) = self.open_file(rel, u64::MAX)?;
        Ok(file.metadata().map_err(io(&path))?.len())
    }

    pub fn read_range(&self, rel: &str, offset: u64, len: u64) -> Result<Vec<u8>, ReadError> {
        let (mut file, path) = self.open_file(rel, u64::MAX)?;
        let size = file.metadata().map_err(io(&path))?.len();
        if offset.checked_add(len).is_none_or(|end| end > size) {
            return Err(refused(&path, format!("bytes {offset}+{len} lie outside its {size} bytes")));
        }
        file.seek(std::io::SeekFrom::Start(offset)).map_err(io(&path))?;
        let mut out = vec![0u8; usize::try_from(len).map_err(|_| refused(&path, "a range larger than memory"))?];
        file.read_exact(&mut out).map_err(io(&path))?;
        Ok(out)
    }

    pub fn open_read(&self, rel: &str, max: u64) -> Result<(std::fs::File, u64), ReadError> {
        let (file, path) = self.open_file(rel, max)?;
        let len = file.metadata().map_err(io(&path))?.len();
        Ok((file, len))
    }

    pub fn copy_to(&self, rel: &str, max: u64, out: &mut dyn Write) -> Result<(u64, ContentHash), ReadError> {
        let (mut file, path) = self.open_file(rel, max)?;
        let mut hash = blake3::Hasher::new();
        let mut buf = vec![0u8; 1 << 20];
        let mut n = 0u64;
        loop {
            let k = file.read(&mut buf).map_err(io(&path))?;
            if k == 0 {
                break;
            }
            n += k as u64;
            if n > max {
                return Err(refused(&path, format!("grew past {max} bytes while read")));
            }
            hash.update(&buf[..k]);
            out.write_all(&buf[..k]).map_err(io(&path))?;
        }
        Ok((n, ContentHash(*hash.finalize().as_bytes())))
    }

    fn open_file(&self, rel: &str, max: u64) -> Result<(std::fs::File, PathBuf), ReadError> {
        check_package_path(rel, false).map_err(ReadError::BadPath)?;
        let path = self.root_path.join(rel);
        match self.kind(rel) {
            Some(EntryKind::File) => {}
            Some(EntryKind::Link) => return Err(refused(&path, "is a link; packages may not contain links")),
            Some(_) => return Err(refused(&path, "is not a regular file")),
            None => return Err(refused(&path, "no entry of exactly this name (names are case-sensitive)")),
        }
        let (folder, name) = rel.rsplit_once('/').unwrap_or(("", rel));
        let dir = self.open_folder(folder)?;
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        #[cfg(unix)]
        cap_std::fs::OpenOptionsExt::custom_flags(&mut options, libc::O_NONBLOCK);
        let file = dir.open_with(name, &options).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => refused(&path, "no such file (it changed after the package was opened)"),
            _ => refused(&path, format!("cannot be opened without following links: {e}")),
        })?;
        let file = file.into_std();
        let meta = check_handle(&path, &file)?;
        if !meta.is_file() {
            return Err(refused(&path, "is not a regular file"));
        }
        if meta.len() > max {
            return Err(refused(&path, format!("{} bytes (at most {max})", meta.len())));
        }
        Ok((file, path))
    }
}

#[cfg(test)]
mod tests;
