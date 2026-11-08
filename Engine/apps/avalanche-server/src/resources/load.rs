use std::path::{Path, PathBuf};
use std::time::SystemTime;

use ssx_content::delivery::{self, FileRole, FileStore};
use ssx_content::install::{InstallError, install_folder_as, install_zip_as};
use ssx_content::manifest::Manifest;
use ssx_content::package::{Origin, PackageSource};

#[derive(Clone, Debug)]
pub struct Loaded {
    pub name: String,
    pub manifest: Manifest,
    pub dir: PathBuf,
    pub files: Vec<delivery::File>,
    pub kept_back: usize,
    pub package: ssx_content::ContentHash,
    pub source: SourceStamp,
    pub findings: usize,
    pub hashes: Vec<(String, ssx_content::ContentHash)>,
}

impl Loaded {
    pub fn bytes(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    pub fn client_scripts(&self) -> bool {
        self.files.iter().any(|f| f.role == FileRole::Script)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceStamp(Vec<(String, u64, Option<SystemTime>)>);

const MAX_STAMP_ENTRIES: usize = ssx_content::path::MAX_PACKAGE_ENTRIES;

pub fn locate(folder: &Path, name: &str) -> Result<(PathBuf, bool), String> {
    let (dir, zip) = (folder.join(name), folder.join(format!("{name}.zip")));
    let is = |p: &Path, folder: bool| {
        std::fs::symlink_metadata(p).is_ok_and(|m| if folder { m.is_dir() } else { m.is_file() })
    };
    match (is(&dir, true), is(&zip, false)) {
        (true, true) => Err(format!("{name}: both {} and {} are there; keep one", dir.display(), zip.display())),
        (true, false) => Ok((dir, false)),
        (false, true) => Ok((zip, true)),
        (false, false) => Err(format!("{name}: no folder {name}/ and no {name}.zip in {}", folder.display())),
    }
}

pub fn stamp(path: &Path, packed: bool) -> Result<SourceStamp, String> {
    let at = |p: &Path, e: std::io::Error| format!("{}: {e}", p.display());
    if packed {
        let m = std::fs::symlink_metadata(path).map_err(|e| at(path, e))?;
        return Ok(SourceStamp(vec![(String::new(), m.len(), m.modified().ok())]));
    }
    let mut out = Vec::new();
    let mut stack = vec![(path.to_path_buf(), String::new())];
    while let Some((dir, rel)) = stack.pop() {
        for e in std::fs::read_dir(&dir).map_err(|e| at(&dir, e))? {
            let e = e.map_err(|e| at(&dir, e))?;
            let name = e.file_name().to_string_lossy().into_owned();
            let rel = if rel.is_empty() { name } else { format!("{rel}/{name}") };
            let m = std::fs::symlink_metadata(e.path()).map_err(|err| at(&e.path(), err))?;
            if out.len() == MAX_STAMP_ENTRIES {
                return Err(format!("{}: more than {MAX_STAMP_ENTRIES} entries", path.display()));
            }
            if m.is_dir() {
                stack.push((e.path(), rel.clone()));
            }
            out.push((rel, m.len(), m.modified().ok()));
        }
    }
    out.sort();
    Ok(SourceStamp(out))
}

fn install_error(e: InstallError) -> String {
    match e {
        InstallError::Check(findings) => {
            let shown: Vec<String> = findings.iter().take(8).map(ToString::to_string).collect();
            format!("its check fails ({} errors): {}", findings.len(), shown.join("; "))
        }
        e => e.to_string(),
    }
}

pub type DiscFile<'a> = &'a dyn Fn(&ssx_content::ContentHash, u64) -> Result<Option<String>, String>;

pub fn load(
    folder: &Path,
    name: &str,
    state: &Path,
    seq: u64,
    store: &FileStore,
    known: DiscFile,
) -> Result<Loaded, String> {
    let (source, packed) = locate(folder, name)?;
    let before = stamp(&source, packed)?;
    let incoming = state.join("incoming");
    if std::fs::symlink_metadata(&incoming).is_ok() {
        std::fs::remove_dir_all(&incoming).map_err(|e| format!("{}: {e}", incoming.display()))?;
    }
    ssx_files::create_dir_all(&incoming).map_err(|e| e.to_string())?;
    let installed = if packed {
        install_zip_as(&source, &incoming, false, Origin::Server)
    } else {
        install_folder_as(&source, &incoming, false, Origin::Server)
    }
    .map_err(|e| format!("{name}: {}", install_error(e)))?;
    if installed.manifest.resource.name != name {
        return Err(format!("{name}: its resource.toml names it {:?}", installed.manifest.resource.name));
    }
    let dir = state.join("installed").join(format!("{name}.{seq}"));
    ssx_files::create_dir_all(&state.join("installed")).map_err(|e| e.to_string())?;
    ssx_files::rename_new(&incoming.join(&installed.folder), &dir).map_err(|e| e.to_string())?;
    let opened = PackageSource::open_as(&dir, Origin::Server);
    let result = opened.map_err(|e| format!("{name}: {e}")).and_then(|p| {
        let files = delivery::files_of(p.tree(), &p.manifest).map_err(|e| format!("{name}: {e}"))?;
        for f in &files.files {
            if let Some(what) = known(&f.hash, f.size)? {
                return Err(format!(
                    "{name}: {} is the disc's {what} (its bytes): a server never sends the disc",
                    f.path
                ));
            }
        }
        for f in &files.files {
            let (mut file, _) = p.tree().open_read(&f.path, f.size).map_err(|e| format!("{name}: {e}"))?;
            store.put(&f.hash, f.size, &mut file).map_err(|e| format!("{name}: {}: {e}", f.path))?;
        }
        let hashes = ssx_content::blocklist::hash_tree(p.tree()).map_err(|e| format!("{name}: {e}"))?;
        Ok((p.manifest, files, hashes))
    });
    let (manifest, files, hashes) = match result {
        Ok(r) => r,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(e);
        }
    };
    let source = stamp(&locate(folder, name)?.0, packed)?;
    if source != before {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(format!("{name}: its files changed while it was read; start it again"));
    }
    Ok(Loaded {
        name: name.to_string(),
        manifest,
        dir,
        files: files.files,
        kept_back: files.kept_back,
        package: files.package,
        source,
        findings: installed.findings.len(),
        hashes,
    })
}
