use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use thiserror::Error;
use zeroize::Zeroizing;

use crate::keys::{KEY_SIZE, from_hex, to_hex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyKind {
    pub header: &'static str,
    pub what: &'static str,
}

pub const PLAYER_IDENTITY: KeyKind =
    KeyKind { header: "Project Avalanche player identity v1", what: "the player's identity key" };
pub const SERVER_IDENTITY: KeyKind =
    KeyKind { header: "Project Avalanche server identity v1", what: "the server's identity key" };
pub const NETCODE_KEY: KeyKind =
    KeyKind { header: "Project Avalanche netcode key v1", what: "the server's netcode private key" };
pub const OPERATOR_KEY: KeyKind = KeyKind { header: "Project Avalanche operator key v1", what: "an operator's key" };

const MAX_FILE: u64 = 4096;

#[derive(Debug, Error)]
pub enum KeyFileError {
    #[error("{path}: {error}")]
    Io { path: PathBuf, error: std::io::Error },

    #[error("{path} is not a regular file: move it away; a new key is made in its place")]
    NotAFile { path: PathBuf },

    #[error(
        "{path} can be read by other users (mode {mode:o}): run `chmod 600 {path}` (or remove the file to make a new \
         key: everyone who trusts this key will refuse the new one)"
    )]
    Permissions { path: PathBuf, mode: u32 },

    #[error(
        "{path} is not {what} as Project Avalanche writes it ({why}); it was not changed. Restore the file from a \
         backup, or move it away to make a new key (everyone who trusts the old key will refuse the new one)"
    )]
    Foreign { path: PathBuf, what: &'static str, why: &'static str },

    #[error("{path} exists already: it is never replaced; move it away first")]
    Exists { path: PathBuf },
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> KeyFileError + '_ {
    move |error| KeyFileError::Io { path: path.to_path_buf(), error }
}

pub fn load(path: &Path, kind: KeyKind) -> Result<Zeroizing<[u8; KEY_SIZE]>, KeyFileError> {
    let meta = std::fs::symlink_metadata(path).map_err(io(path))?;
    if !meta.is_file() {
        return Err(KeyFileError::NotAFile { path: path.to_path_buf() });
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = meta.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            return Err(KeyFileError::Permissions { path: path.to_path_buf(), mode });
        }
    }
    let mut text = Zeroizing::new(String::new());
    File::open(path)
        .map_err(io(path))?
        .take(MAX_FILE)
        .read_to_string(&mut text)
        .map_err(|_| KeyFileError::Foreign { path: path.to_path_buf(), what: kind.what, why: "not text" })?;
    let foreign = |why| KeyFileError::Foreign { path: path.to_path_buf(), what: kind.what, why };
    let rest = text.strip_prefix(kind.header).and_then(|r| r.strip_prefix('\n')).ok_or(foreign("another header"))?;
    let hex = rest.strip_suffix('\n').ok_or(foreign("no line end after the key"))?;
    from_hex(hex).map(Zeroizing::new).map_err(|_| foreign("the key is not 64 hex digits"))
}

pub fn create(path: &Path, kind: KeyKind, key: &[u8; KEY_SIZE]) -> Result<(), KeyFileError> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        ssx_files::create_dir_all(dir)
            .map_err(|e| KeyFileError::Io { path: e.path().to_path_buf(), error: e.into_io() })?;
    }
    let text = Zeroizing::new(format!("{}\n{}\n", kind.header, to_hex(key)));
    ssx_files::create(path, text.as_bytes(), &ssx_files::Options::owner_only()).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => KeyFileError::Exists { path: path.to_path_buf() },
        _ => KeyFileError::Io { path: e.path().to_path_buf(), error: e.into_io() },
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Created,
    Existing,
}

pub fn load_or_create(
    path: &Path,
    kind: KeyKind,
    generate: impl FnOnce() -> [u8; KEY_SIZE],
) -> Result<(Zeroizing<[u8; KEY_SIZE]>, Origin), KeyFileError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok((load(path, kind)?, Origin::Existing)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let key = Zeroizing::new(generate());
            match create(path, kind, &key) {
                Ok(()) => Ok((key, Origin::Created)),
                Err(KeyFileError::Exists { .. }) => Ok((load(path, kind)?, Origin::Existing)),
                Err(e) => Err(e),
            }
        }
        Err(error) => Err(KeyFileError::Io { path: path.to_path_buf(), error }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ssx-keyfile-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).expect("temp dir");
        d
    }

    #[test]
    fn created_once_owner_only_never_replaced() {
        let d = dir("once");
        let path = d.join("sub").join("identity.key");
        let (key, origin) = load_or_create(&path, PLAYER_IDENTITY, || [7; 32]).expect("create");
        assert_eq!((*key, origin), ([7; 32], Origin::Created));
        let (again, origin) = load_or_create(&path, PLAYER_IDENTITY, || [9; 32]).expect("load");
        assert_eq!((*again, origin), ([7; 32], Origin::Existing), "the existing key, not a new one");
        assert!(matches!(create(&path, PLAYER_IDENTITY, &[1; 32]), Err(KeyFileError::Exists { .. })));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).expect("meta").permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("chmod");
            assert!(matches!(load(&path, PLAYER_IDENTITY), Err(KeyFileError::Permissions { mode: 0o644, .. })));
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn foreign_and_corrupt_files_are_refused_and_kept() {
        let d = dir("foreign");
        let path = d.join("netcode.key");
        let good = format!("{}\n{}\n", NETCODE_KEY.header, "ab".repeat(32));
        let cases = [
            format!("{}\n{}\n", SERVER_IDENTITY.header, "ab".repeat(32)),
            good.replace('\n', "\r\n"),
            good.trim_end().to_string(),
            format!("{good}\n"),
            good.replacen("ab", "zz", 1),
            "x".repeat(10_000),
            String::new(),
        ];
        for text in &cases {
            let _ = std::fs::remove_file(&path);
            create(&path, NETCODE_KEY, &[0; 32]).expect("owner-only file");
            std::fs::write(&path, text).expect("write");
            let err = load_or_create(&path, NETCODE_KEY, || [1; 32]).expect_err(text);
            assert!(matches!(err, KeyFileError::Foreign { .. }), "{err}");
            assert_eq!(&std::fs::read_to_string(&path).expect("read"), text, "never replaced");
        }
        std::fs::remove_file(&path).expect("rm");
        std::fs::create_dir(&path).expect("a directory in its place");
        assert!(matches!(load(&path, NETCODE_KEY), Err(KeyFileError::NotAFile { .. })));
        let _ = std::fs::remove_dir_all(&d);
    }
}
