use std::path::{Path, PathBuf};

use super::journal::{self, BACKUP, Recovered};
use super::{Staged, is_staging_name};

pub(crate) const FONTS: &str = "fonts";

pub(crate) fn allowed_files() -> &'static [&'static str] {
    #[cfg(windows)]
    {
        &[
            "Project Avalanche.exe",
            "avalanche-game.exe",
            "avalanche-server.exe",
            "THIRD_PARTY_NOTICES.txt",
            "THIRD_PARTY_NOTICES-server.txt",
            "README.txt",
            "fonts/NotoSans-Regular.ttf",
            "fonts/NotoSansArabic-Regular.ttf",
            "fonts/NotoSansHebrew-Regular.ttf",
            "fonts/NotoSansDevanagari-Regular.ttf",
            "fonts/NotoSansThai-Regular.ttf",
            "fonts/NotoSansCJKjp-Regular.otf",
            "fonts/OFL.txt",
            "fonts/fonts.toml",
        ]
    }
    #[cfg(not(windows))]
    {
        &[
            "project-avalanche",
            "avalanche-game",
            "avalanche-server",
            "THIRD_PARTY_NOTICES.txt",
            "THIRD_PARTY_NOTICES-server.txt",
            "README.txt",
            "fonts/NotoSans-Regular.ttf",
            "fonts/NotoSansArabic-Regular.ttf",
            "fonts/NotoSansHebrew-Regular.ttf",
            "fonts/NotoSansDevanagari-Regular.ttf",
            "fonts/NotoSansThai-Regular.ttf",
            "fonts/NotoSansCJKjp-Regular.otf",
            "fonts/OFL.txt",
            "fonts/fonts.toml",
        ]
    }
}

pub(crate) fn path_of(base: &Path, name: &str) -> PathBuf {
    name.split('/').fold(base.to_path_buf(), |path, part| path.join(part))
}

pub(crate) fn folder_of(name: &str) -> Option<&str> {
    name.rsplit_once('/').map(|(folder, _)| folder)
}

#[cfg(windows)]
pub(crate) fn launcher_name() -> &'static str {
    "Project Avalanche.exe"
}

#[cfg(not(windows))]
pub(crate) fn launcher_name() -> &'static str {
    "project-avalanche"
}

#[cfg(windows)]
pub(crate) fn game_name() -> &'static str {
    "avalanche-game.exe"
}

#[cfg(not(windows))]
pub(crate) fn game_name() -> &'static str {
    "avalanche-game"
}

#[cfg(unix)]
pub(crate) fn is_program(name: &str) -> bool {
    matches!(name, "project-avalanche" | "avalanche-game" | "avalanche-server")
}

pub fn install_dir() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot find the launcher's folder: {e}"))?;
    exe.parent().map(Path::to_path_buf).ok_or_else(|| "the launcher has no folder".to_string())
}

pub fn apply(staged: &Staged) -> Result<(), String> {
    validate(staged)?;
    match journal::recover(&staged.install) {
        Ok(Recovered::Busy) => return Err("another update is being installed into this folder; wait for it".into()),
        Ok(_) => {}
        Err(e) => return Err(format!("an earlier update did not finish, and it is left as it is: {e}")),
    }
    #[cfg(windows)]
    {
        spawn_helper(staged)?;
        std::process::exit(0)
    }
    #[cfg(not(windows))]
    {
        if let Some(note) = journal::install(&staged.dir, &staged.install, &staged.files)? {
            ssx_core::eprintln!("update: {note}");
        }
        let _ = std::fs::remove_dir_all(&staged.dir);
        let exe = staged.install.join(launcher_name());
        std::process::Command::new(&exe)
            .current_dir(&staged.install)
            .spawn()
            .map_err(|e| format!("the update was installed, but the launcher could not be restarted: {e}"))?;
        std::process::exit(0)
    }
}

fn validate(staged: &Staged) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(&staged.dir)
        .map_err(|e| format!("the update folder {} is missing: {e}", staged.dir.display()))?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err(format!("the update folder {} is not a plain folder", staged.dir.display()));
    }
    if staged.dir.parent() != Some(staged.install.as_path()) {
        return Err("the update folder is not beside the launcher".into());
    }
    let Some(name) = staged.dir.file_name().and_then(|n| n.to_str()) else {
        return Err("the update folder has no name".into());
    };
    if !is_staging_name(name) {
        return Err("the update folder is not one the launcher made".into());
    }
    for f in &staged.files {
        if !allowed_files().contains(&f.as_str()) {
            return Err(format!("the update would replace {f}, which is not a file the launcher may replace"));
        }
        if let Some(folder) = folder_of(f) {
            let meta = std::fs::symlink_metadata(staged.dir.join(folder))
                .map_err(|e| format!("the update has no {folder} folder: {e}"))?;
            if !meta.is_dir() || meta.file_type().is_symlink() {
                return Err(format!("{folder} in the update is not a plain folder"));
            }
        }
        let src = path_of(&staged.dir, f);
        let meta = std::fs::symlink_metadata(&src).map_err(|e| format!("the update has no {f}: {e}"))?;
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err(format!("{f} in the update is not a plain file"));
        }
        #[cfg(unix)]
        if is_program(f) && std::os::unix::fs::PermissionsExt::mode(&meta.permissions()) & 0o100 == 0 {
            return Err(format!("{f} in the update is not executable"));
        }
    }
    for need in [launcher_name(), game_name()] {
        if !staged.files.iter().any(|f| f == need) {
            return Err(format!("the update has no {need}"));
        }
    }
    Ok(())
}

#[cfg(windows)]
fn spawn_helper(staged: &Staged) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot find the launcher: {e}"))?;
    let helper = std::env::temp_dir().join(format!("ProjectAvalanche-updater-{}-{}.exe", std::process::id(), nanos()));
    std::fs::copy(&exe, &helper).map_err(|e| format!("cannot copy the update helper: {e}"))?;
    let mut cmd = std::process::Command::new(&helper);
    cmd.arg("--apply-update")
        .arg(&staged.dir)
        .arg(std::process::id().to_string())
        .arg(&staged.install)
        .arg(launcher_name())
        .args(&staged.files);
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.spawn().map_err(|e| format!("cannot start the update helper: {e}"))?;
    Ok(())
}

fn cleanup_staging(dir: &Path) {
    let Ok(entries) = dir.read_dir() else {
        return;
    };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with(".update-") || name.starts_with(BACKUP) {
            continue;
        }
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.is_dir() || meta.file_type().is_symlink() {
            continue;
        }
        let old =
            meta.modified().ok().and_then(|m| now.duration_since(m).ok()).is_some_and(|d| d.as_secs() > 24 * 3600);
        if old {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

#[cfg(windows)]
pub fn helper(args: &[String]) -> i32 {
    if args.len() < 4 {
        return 2;
    }
    let dir = PathBuf::from(&args[0]);
    let Ok(pid) = args[1].parse::<u32>() else {
        return 2;
    };
    let install = PathBuf::from(&args[2]);
    if args[3] != launcher_name() {
        return 2;
    }
    if dir.parent() != Some(install.as_path()) {
        return 2;
    }
    if !dir.file_name().and_then(|n| n.to_str()).is_some_and(is_staging_name) {
        return 2;
    }
    let files = &args[4..];
    for f in files {
        if !allowed_files().contains(&f.as_str()) {
            return 2;
        }
    }
    if !files.iter().any(|f| f == launcher_name()) {
        return 2;
    }
    let mut log = std::fs::File::create(dir.join("update.log")).ok();
    let mut note = |s: &str| {
        use std::io::Write;
        if let Some(f) = log.as_mut() {
            let _ = writeln!(f, "{s}");
            let _ = f.flush();
        }
    };
    note(&format!("helper: installing {} into {} after process {pid}", dir.display(), install.display()));
    wait_for_exit(pid);
    note("helper: the launcher exited");
    match journal::install(&dir, &install, files) {
        Ok(leftover) => {
            if let Some(leftover) = leftover {
                note(&format!("helper: {leftover}"));
            }
        }
        Err(e) => {
            note(&format!("helper: {e}"));
            let _ = std::process::Command::new(install.join(launcher_name())).current_dir(&install).spawn();
            return 1;
        }
    }
    note("helper: installed");
    let _ = std::fs::remove_dir_all(&dir);
    match std::process::Command::new(install.join(launcher_name())).current_dir(&install).spawn() {
        Ok(_) => {
            if let Ok(me) = std::env::current_exe() {
                let _ = std::fs::remove_file(me);
            }
            0
        }
        Err(e) => {
            note(&format!("helper: the new launcher did not start: {e}"));
            1
        }
    }
}

#[cfg(windows)]
fn wait_for_exit(pid: u32) {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};
    const INFINITE: u32 = u32::MAX;
    unsafe {
        let handle: HANDLE = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if handle.is_null() {
            return;
        }
        let _: u32 = WaitForSingleObject(handle, INFINITE);
        let _: i32 = CloseHandle(handle);
    }
}

#[cfg(windows)]
fn nanos() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default()
}

#[cfg(not(windows))]
pub fn helper(_args: &[String]) -> i32 {
    2
}

pub fn cleanup_leftovers() {
    if let Ok(dir) = install_dir() {
        for note in leftovers_in(&dir) {
            ssx_core::eprintln!("update: {note}");
        }
    }
    #[cfg(windows)]
    {
        let Ok(dir) = std::env::temp_dir().read_dir() else {
            return;
        };
        for entry in dir.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if name.starts_with("ProjectAvalanche-updater-") && name.ends_with(".exe") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

fn leftovers_in(dir: &Path) -> Vec<String> {
    let mut notes = Vec::new();
    match journal::recover(dir) {
        Ok(Recovered::Nothing | Recovered::Busy | Recovered::Cleared) => {}
        Ok(Recovered::Finished) => notes.push("an update cut short after its new files were in place: finished".into()),
        Ok(Recovered::RolledBack) => notes.push("an update cut short: the files from before it are back".into()),
        Err(e) => notes.push(format!("an update cut short could not be finished or undone: {e}")),
    }
    notes.extend(journal::recover_legacy(dir));
    if std::fs::symlink_metadata(dir.join(BACKUP)).is_err() {
        cleanup_staging(dir);
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fonts_allowed_are_the_shipped_set() {
        use ssx_content::font::shipped::{FOLDER, LICENSE_FILE, MANIFEST_FILE, SHIPPED};
        assert_eq!(FONTS, FOLDER);
        let mut fonts: Vec<&str> = allowed_files().iter().filter_map(|n| n.strip_prefix("fonts/")).collect();
        fonts.sort_unstable();
        let mut want: Vec<&str> = SHIPPED.iter().map(|s| s.file).chain([LICENSE_FILE, MANIFEST_FILE]).collect();
        want.sort_unstable();
        assert_eq!(fonts, want);
        assert_eq!(path_of(Path::new("a"), "fonts/OFL.txt"), Path::new("a").join("fonts").join("OFL.txt"));
        assert_eq!((folder_of("fonts/OFL.txt"), folder_of("README.txt")), (Some("fonts"), None));
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("ssx-install-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn backup_left(install: &Path) -> bool {
        std::fs::read_dir(install).unwrap().flatten().any(|e| e.file_name().to_string_lossy().starts_with(BACKUP))
    }

    fn folders(tag: &str, names: &[&str]) -> (PathBuf, PathBuf, PathBuf) {
        let root = temp_dir(tag);
        let (install, staged) = (root.join("install"), root.join("install/.update-0.3.0-0"));
        std::fs::create_dir_all(&staged).unwrap();
        for name in names {
            std::fs::write(install.join(name), b"old").unwrap();
            std::fs::write(staged.join(name), b"new").unwrap();
        }
        (root, install, staged)
    }

    #[test]
    fn a_failed_install_puts_the_old_files_back() {
        let (root, install, staged) = folders("rollback", &[launcher_name(), game_name()]);
        let files = vec![game_name().to_string(), "evil.sh".to_string(), launcher_name().to_string()];
        assert!(journal::install(&staged, &install, &files).is_err());
        std::fs::remove_file(staged.join(game_name())).unwrap();
        let files = vec![game_name().to_string(), launcher_name().to_string()];
        assert!(journal::install(&staged, &install, &files).unwrap_err().contains("has no"));
        assert_eq!(std::fs::read(install.join(game_name())).unwrap(), b"old");
        assert_eq!(std::fs::read(install.join(launcher_name())).unwrap(), b"old");
        assert!(!backup_left(&install));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_verified_install_replaces_the_files() {
        let (root, install, staged) = folders("replace", &[launcher_name(), game_name()]);
        let files = vec![launcher_name().to_string(), game_name().to_string()];
        assert_eq!(journal::install(&staged, &install, &files), Ok(None));
        for name in [launcher_name(), game_name()] {
            assert_eq!(std::fs::read(install.join(name)).unwrap(), b"new");
            assert!(!staged.join(name).exists());
        }
        assert!(!backup_left(&install));
        std::fs::remove_dir_all(&root).unwrap();
    }

    fn staged(install: &Path, dir: &Path, files: &[&str]) -> Staged {
        let asset = super::super::Asset { name: String::new(), url: String::new(), size: 0, sha256: [0; 32] };
        let release = super::super::Release {
            version: semver::Version::new(0, 3, 0),
            tag: "v0.3.0".into(),
            title: String::new(),
            notes: String::new(),
            page: String::new(),
            asset,
        };
        let files = files.iter().map(|f| f.to_string()).collect();
        Staged { release, dir: dir.to_path_buf(), install: install.to_path_buf(), files }
    }

    #[test]
    fn an_update_is_validated_before_it_is_installed() {
        let (root, install, dir) = folders("validate", &[launcher_name(), game_name()]);
        #[cfg(unix)]
        for name in [launcher_name(), game_name()] {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.join(name), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        validate(&staged(&install, &dir, &[launcher_name(), game_name()])).unwrap();
        let backup = install.join(BACKUP);
        std::fs::rename(&dir, &backup).unwrap();
        let refused = validate(&staged(&install, &backup, &[launcher_name(), game_name()])).unwrap_err();
        assert!(refused.contains("not one the launcher made"), "{refused}");
        std::fs::rename(&backup, &dir).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.join(game_name()), std::fs::Permissions::from_mode(0o644)).unwrap();
            let refused = validate(&staged(&install, &dir, &[launcher_name(), game_name()])).unwrap_err();
            assert!(refused.contains("not executable"), "{refused}");
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn the_cleanup_never_removes_a_backup() {
        let root = temp_dir("cleanup");
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3 * 24 * 3600);
        let age = |dir: &Path| std::fs::File::open(dir).unwrap().set_modified(old).unwrap();
        for name in [".update-0.2.0-0", ".update-backup-4242", ".update-0.3.0-1"] {
            std::fs::create_dir(root.join(name)).unwrap();
        }
        std::fs::write(root.join(".update-backup-4242").join(launcher_name()), b"old").unwrap();
        std::fs::write(root.join(launcher_name()), b"new").unwrap();
        for name in [".update-0.2.0-0", ".update-backup-4242"] {
            age(&root.join(name));
        }
        let notes = leftovers_in(&root);
        assert!(notes.iter().any(|n| n.contains("kept")), "{notes:?}");
        assert!(!root.join(".update-0.2.0-0").exists());
        assert!(root.join(".update-0.3.0-1").exists(), "younger than a day");
        assert_eq!(std::fs::read(root.join(".update-backup-4242").join(launcher_name())).unwrap(), b"old");
        std::fs::create_dir(root.join(".update-0.2.0-2")).unwrap();
        age(&root.join(".update-0.2.0-2"));
        std::fs::create_dir(root.join(BACKUP)).unwrap();
        std::fs::write(root.join(BACKUP).join("stray"), b"x").unwrap();
        let notes = leftovers_in(&root);
        assert!(notes.iter().any(|n| n.contains("could not be finished")), "{notes:?}");
        assert!(root.join(".update-0.2.0-2").exists());
        assert!(root.join(BACKUP).join("stray").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_allowlist_names_the_launcher_and_the_game() {
        assert!(allowed_files().contains(&launcher_name()));
        assert!(allowed_files().contains(&game_name()));
        assert!(!allowed_files().contains(&"evil.sh"));
    }

    #[test]
    fn an_install_folder_is_the_launcher_folder() {
        let dir = install_dir().unwrap();
        assert!(dir.is_dir());
    }
}
