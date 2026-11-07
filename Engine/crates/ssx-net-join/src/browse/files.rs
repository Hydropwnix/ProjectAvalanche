use std::io;
use std::path::{Path, PathBuf};

const ASIDE_NAMES: u32 = 999;

pub fn move_aside(path: &Path) -> io::Result<PathBuf> {
    let name = path.file_name().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?;
    let names = (1..=ASIDE_NAMES).map(|n| {
        let mut aside = name.to_os_string();
        aside.push(format!(".broken-{n}"));
        path.with_file_name(aside)
    });
    ssx_files::move_aside(path, names)
        .map_err(ssx_files::Error::into_io)?
        .ok_or_else(|| io::Error::new(io::ErrorKind::AlreadyExists, format!("{ASIDE_NAMES} files moved aside already")))
}

pub fn write_whole(path: &Path, text: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        ssx_files::create_dir_all(dir).map_err(ssx_files::Error::into_io)?;
    }
    ssx_files::replace(path, text, &ssx_files::Options::new()).map_err(ssx_files::Error::into_io)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_moved_aside_never_replaces_another() {
        let dir = std::env::temp_dir().join(format!("ava-aside-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("servers.json");
        std::fs::write(&path, "first").unwrap();
        std::fs::write(dir.join("servers.json.broken-1"), "kept").unwrap();
        let aside = move_aside(&path).unwrap();
        assert_eq!(aside, dir.join("servers.json.broken-2"));
        assert_eq!(std::fs::read_to_string(dir.join("servers.json.broken-1")).unwrap(), "kept");
        assert_eq!(std::fs::read_to_string(&aside).unwrap(), "first");
        assert!(!path.exists());
        std::fs::write(dir.join("servers.json.partial"), "an old half").unwrap();
        write_whole(&path, b"new").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        assert!(!dir.join("servers.json.partial").exists());
        let folder = dir.join("taken");
        std::fs::create_dir_all(folder.join("inside")).unwrap();
        assert!(write_whole(&folder, b"x").is_err());
        assert!(!dir.join("taken.partial").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
