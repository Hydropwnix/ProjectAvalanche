use std::path::{Path, PathBuf};

use crate::error::{Error, at};
use crate::{Step, fault, sys};

fn parent(path: &Path) -> &Path {
    path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."))
}

pub fn sync_folder(dir: &Path) -> Result<(), Error> {
    at(|| dir.to_path_buf(), fault::check(Step::FolderSync).and_then(|()| sys::sync_folder(dir)))
}

pub fn create_dir_all(dir: &Path) -> Result<(), Error> {
    let mut made = Vec::new();
    let mut d = dir;
    while !d.as_os_str().is_empty() {
        match std::fs::metadata(d) {
            Ok(_) => break,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => made.push(d),
            Err(e) => return Err(Error::new(d, e)),
        }
        match d.parent() {
            Some(p) => d = p,
            None => break,
        }
    }
    at(|| dir.to_path_buf(), std::fs::create_dir_all(dir))?;
    for d in made {
        sync_folder(parent(d))?;
    }
    Ok(())
}

pub fn rename_new(from: &Path, to: &Path) -> Result<(), Error> {
    at(|| to.to_path_buf(), fault::check(Step::Rename).and_then(|()| sys::rename_noreplace(from, to)))?;
    sync_folder(parent(to))?;
    if parent(from) != parent(to) {
        sync_folder(parent(from))?;
    }
    Ok(())
}

pub fn move_aside(path: &Path, names: impl IntoIterator<Item = PathBuf>) -> Result<Option<PathBuf>, Error> {
    for aside in names {
        match rename_new(path, &aside) {
            Ok(()) => return Ok(Some(aside)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Ok(None)
}

pub fn sync_tree(root: &Path) -> Result<(), Error> {
    let mut folders = Vec::new();
    let mut todo = vec![root.to_path_buf()];
    while let Some(dir) = todo.pop() {
        for entry in at(|| dir.clone(), std::fs::read_dir(&dir))? {
            let entry = at(|| dir.clone(), entry)?;
            let path = entry.path();
            let kind = at(|| path.clone(), entry.file_type())?;
            if kind.is_dir() {
                todo.push(path);
            } else if kind.is_file() {
                at(|| path.clone(), fault::check(Step::Sync).and_then(|()| sys::sync_file(&path)))?;
            } else {
                let why = "a link or a special file in a folder written whole";
                return Err(Error::new(path, std::io::Error::new(std::io::ErrorKind::InvalidData, why)));
            }
        }
        folders.push(dir);
    }
    for dir in folders.iter().rev() {
        sync_folder(dir)?;
    }
    Ok(())
}

pub fn install_folder(staged: &Path, to: &Path) -> Result<(), Error> {
    sync_tree(staged)?;
    rename_new(staged, to)
}

pub fn rename_over(from: &Path, to: &Path) -> Result<(), Error> {
    at(|| to.to_path_buf(), fault::check(Step::Rename).and_then(|()| sys::rename_replace(from, to)))?;
    sync_folder(parent(to))?;
    if parent(from) != parent(to) {
        sync_folder(parent(from))?;
    }
    Ok(())
}

pub fn keep_copy(from: &Path, to: &Path) -> Result<(), Error> {
    #[cfg(unix)]
    {
        use std::io::ErrorKind;
        if sys::id_of(from).map_err(|e| Error::new(from, e))?.is_none() {
            let why = "not a plain file";
            return Err(Error::new(from, std::io::Error::new(ErrorKind::InvalidInput, why)));
        }
        match std::fs::hard_link(from, to) {
            Ok(()) => {
                at(|| to.to_path_buf(), fault::check(Step::Sync).and_then(|()| sys::sync_file(to)))?;
                return sync_folder(parent(to));
            }
            Err(e)
                if matches!(
                    e.kind(),
                    ErrorKind::PermissionDenied
                        | ErrorKind::Unsupported
                        | ErrorKind::TooManyLinks
                        | ErrorKind::CrossesDevices
                ) => {}
            Err(e) => return Err(Error::new(to, e)),
        }
    }
    copy_new(from, to)
}

pub fn copy_new(from: &Path, to: &Path) -> Result<(), Error> {
    use std::io::{Read, Write};
    let mut src = match sys::open_regular(from) {
        Ok(Some(f)) => f,
        Ok(None) => {
            let why = "not a plain file";
            return Err(Error::new(from, std::io::Error::new(std::io::ErrorKind::InvalidInput, why)));
        }
        Err(e) => return Err(Error::new(from, e)),
    };
    let opts = crate::Options::new().mode(sys::mode_of(from).unwrap_or(0o666));
    crate::write::create_at(to, &opts, |f, partial| {
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = src.read(&mut buf).map_err(|e| Error::new(from, e))?;
            if n == 0 {
                return Ok::<(), Error>(());
            }
            f.write_all(&buf[..n]).map_err(|e| Error::new(partial, e))?;
        }
    })
}
