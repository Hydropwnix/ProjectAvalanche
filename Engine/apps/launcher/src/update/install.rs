use std::path::{Path, PathBuf};

use super::Staged;

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
        ]
    }
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

pub fn install_dir() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot find the launcher's folder: {e}"))?;
    exe.parent().map(Path::to_path_buf).ok_or_else(|| "the launcher has no folder".to_string())
}

pub fn apply(staged: &Staged) -> Result<(), String> {
    validate(staged)?;
    #[cfg(windows)]
    {
        spawn_helper(staged)?;
        std::process::exit(0)
    }
    #[cfg(not(windows))]
    {
        apply_files(&staged.dir, &staged.install, &staged.files)?;
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
    if !name.starts_with(".update-") {
        return Err("the update folder is not one the launcher made".into());
    }
    for f in &staged.files {
        if !allowed_files().contains(&f.as_str()) {
            return Err(format!("the update would replace {f}, which is not a file the launcher may replace"));
        }
        let src = staged.dir.join(f);
        let meta = std::fs::symlink_metadata(&src).map_err(|e| format!("the update has no {f}: {e}"))?;
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err(format!("{f} in the update is not a plain file"));
        }
    }
    for need in [launcher_name(), game_name()] {
        if !staged.files.iter().any(|f| f == need) {
            return Err(format!("the update has no {need}"));
        }
    }
    Ok(())
}

fn apply_files(dir: &Path, install: &Path, files: &[String]) -> Result<(), String> {
    let backup = install.join(format!(".update-backup-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&backup);
    std::fs::create_dir(&backup).map_err(|e| format!("cannot make a backup folder: {e}"))?;
    let mut done: Vec<String> = Vec::new();
    let mut had_backup: Vec<String> = Vec::new();
    let result = (|| -> Result<(), String> {
        for name in files {
            if !allowed_files().contains(&name.as_str()) {
                return Err(format!("refusing to replace {name}"));
            }
            let src = dir.join(name);
            let dst = install.join(name);
            if let Ok(meta) = std::fs::symlink_metadata(&dst) {
                if !meta.is_file() {
                    return Err(format!("{} is not a plain file", dst.display()));
                }
                move_file(&dst, &backup.join(name)).map_err(|e| format!("cannot move the old {name} aside: {e}"))?;
                had_backup.push(name.clone());
            }
            if let Err(e) = move_file(&src, &dst) {
                return Err(format!("cannot put the new {name} in place: {e}"));
            }
            #[cfg(unix)]
            {
                if matches!(name.as_str(), "project-avalanche" | "avalanche-game" | "avalanche-server") {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(&dst, std::fs::Permissions::from_mode(0o755));
                }
            }
            done.push(name.clone());
        }
        Ok(())
    })();
    if let Err(e) = result {
        for name in done.iter().rev() {
            let _ = std::fs::remove_file(install.join(name));
        }
        for name in had_backup.iter().rev() {
            let _ = move_file(&backup.join(name), &install.join(name));
        }
        let _ = std::fs::remove_dir_all(&backup);
        return Err(e);
    }
    #[cfg(unix)]
    {
        if let Ok(d) = std::fs::File::open(install) {
            let _ = d.sync_all();
        }
    }
    let _ = std::fs::remove_dir_all(&backup);
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

fn cleanup_staging() {
    let Ok(dir) = install_dir() else {
        return;
    };
    let Ok(entries) = dir.read_dir() else {
        return;
    };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with(".update-") {
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
    if !dir.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(".update-")) {
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
    let files: Vec<String> = files.to_vec();
    if let Err(e) = apply_files(&dir, &install, &files) {
        note(&format!("helper: {e}"));
        let _ = std::process::Command::new(install.join(launcher_name())).current_dir(&install).spawn();
        return 1;
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

fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        let mut last = None;
        for _ in 0..20 {
            match std::fs::rename(from, to) {
                Ok(()) => return Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                    last = Some(e);
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                Err(e) => return Err(e),
            }
        }
        Err(last.unwrap_or_else(|| std::io::Error::other("the file could not be moved")))
    }
    #[cfg(not(windows))]
    {
        std::fs::rename(from, to)
    }
}

#[cfg(not(windows))]
pub fn helper(_args: &[String]) -> i32 {
    2
}

pub fn cleanup_leftovers() {
    cleanup_staging();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("ssx-install-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn backup_left(install: &Path) -> bool {
        std::fs::read_dir(install)
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with(".update-backup-"))
    }

    #[test]
    fn a_failed_install_puts_the_old_files_back() {
        let root = temp_dir("rollback");
        let (install, staged) = (root.join("install"), root.join(".update-0.2.0-0"));
        std::fs::create_dir_all(&install).unwrap();
        std::fs::create_dir_all(&staged).unwrap();
        std::fs::write(install.join(game_name()), b"old").unwrap();
        std::fs::write(staged.join(game_name()), b"new").unwrap();
        let files = vec![game_name().to_string(), "evil.sh".to_string()];
        assert!(apply_files(&staged, &install, &files).is_err());
        assert_eq!(std::fs::read(install.join(game_name())).unwrap(), b"old");
        assert!(!backup_left(&install));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_verified_install_replaces_the_files() {
        let root = temp_dir("replace");
        let (install, staged) = (root.join("install"), root.join(".update-0.2.0-0"));
        std::fs::create_dir_all(&install).unwrap();
        std::fs::create_dir_all(&staged).unwrap();
        for name in [launcher_name(), game_name()] {
            std::fs::write(install.join(name), b"old").unwrap();
            std::fs::write(staged.join(name), b"new").unwrap();
        }
        let files = vec![launcher_name().to_string(), game_name().to_string()];
        apply_files(&staged, &install, &files).unwrap();
        for name in [launcher_name(), game_name()] {
            assert_eq!(std::fs::read(install.join(name)).unwrap(), b"new");
            assert!(!staged.join(name).exists());
        }
        assert!(!backup_left(&install));
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
