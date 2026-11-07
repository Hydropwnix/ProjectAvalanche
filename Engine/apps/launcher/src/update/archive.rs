use std::io::Read;
use std::path::{Component, Path};

use flate2::read::GzDecoder;
use tar::EntryType;

#[cfg(unix)]
use super::install::is_program;
use super::install::{FONTS, allowed_files, folder_of, path_of};

const MAX_FILE: u64 = 256 * 1024 * 1024;
const MAX_UNPACKED: u64 = 512 * 1024 * 1024;
const MAX_STREAM: u64 = MAX_UNPACKED + 64 * 1024 * 1024;

pub fn extract(archive: &Path, dir: &Path) -> Result<Vec<String>, String> {
    let file = std::fs::File::open(archive).map_err(|e| format!("cannot read the update archive: {e}"))?;
    let mut tar = tar::Archive::new(GzDecoder::new(file).take(MAX_STREAM));
    let mut written = Vec::new();
    let mut made: Vec<String> = Vec::new();
    let mut total = 0u64;
    let entries = tar.entries().map_err(|e| format!("the update archive does not read: {e}"))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| format!("the update archive is damaged: {e}"))?;
        let kind = entry.header().entry_type();
        if kind == EntryType::Directory {
            continue;
        }
        let path = entry.path().map_err(|e| format!("an entry of the update archive has no readable path: {e}"))?;
        let Some(name) = entry_name(&path)? else {
            continue;
        };
        let size = entry.header().size().map_err(|e| format!("an entry of the update archive has no size: {e}"))?;
        total = total.saturating_add(size);
        if size > MAX_FILE || total > MAX_UNPACKED {
            return Err(format!("the update archive unpacks to more than {} MiB", MAX_UNPACKED / (1024 * 1024)));
        }
        if !allowed_files().contains(&name.as_str()) {
            continue;
        }
        if written.contains(&name) {
            return Err(format!("the update archive holds {name} twice"));
        }
        if kind != EntryType::Regular && kind != EntryType::Continuous {
            return Err(format!("the update archive holds {name} as something other than a plain file"));
        }
        if let Some(folder) = folder_of(&name)
            && !made.iter().any(|m| m == folder)
        {
            let sub = dir.join(folder);
            std::fs::create_dir(&sub).map_err(|e| format!("cannot make {}: {e}", sub.display()))?;
            made.push(folder.to_string());
        }
        let target = path_of(dir, &name);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, if is_program(&name) { 0o755 } else { 0o666 });
        let mut out = options.open(&target).map_err(|e| format!("cannot write {}: {e}", target.display()))?;
        let copied =
            std::io::copy(&mut entry, &mut out).map_err(|e| format!("cannot write {}: {e}", target.display()))?;
        if copied != size {
            return Err(format!("{name} in the update archive is shorter than its header says"));
        }
        out.sync_all().map_err(|e| format!("cannot finish {}: {e}", target.display()))?;
        written.push(name);
    }
    Ok(written)
}

fn entry_name(path: &Path) -> Result<Option<String>, String> {
    let components: Vec<Component<'_>> = path.components().filter(|c| !matches!(c, Component::CurDir)).collect();
    let font = |file: &std::ffi::OsStr| file.to_str().map(|f| format!("{FONTS}/{f}"));
    match components.as_slice() {
        [Component::Normal(name)] => Ok(name.to_str().map(str::to_owned)),
        [Component::Normal(first), Component::Normal(name)] => {
            match font(name).filter(|f| *first == FONTS && allowed_files().contains(&f.as_str())) {
                Some(f) => Ok(Some(f)),
                None => Ok(name.to_str().map(str::to_owned)),
            }
        }
        [Component::Normal(_), Component::Normal(sub), Component::Normal(name)] if *sub == FONTS => Ok(font(name)),
        _ => Err(format!("the update archive holds an unsafe path: {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("ssx-update-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn archive(dir: &Path, build: impl FnOnce(&mut tar::Builder<&mut Vec<u8>>)) -> PathBuf {
        let mut bytes = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut bytes);
            build(&mut builder);
            builder.finish().unwrap();
        }
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(&bytes).unwrap();
        let path = dir.join("a.tar.gz");
        std::fs::write(&path, encoder.finish().unwrap()).unwrap();
        path
    }

    fn raw(builder: &mut tar::Builder<&mut Vec<u8>>, name: &str, kind: u8, link: Option<&str>, data: &[u8]) {
        let mut header = [0u8; 512];
        header[..name.len()].copy_from_slice(name.as_bytes());
        header[100..108].copy_from_slice(b"0000644\0");
        header[108..116].copy_from_slice(b"0000000\0");
        header[116..124].copy_from_slice(b"0000000\0");
        header[124..136].copy_from_slice(format!("{:011o}\0", data.len()).as_bytes());
        header[136..148].copy_from_slice(b"00000000000\0");
        header[156] = kind;
        if let Some(link) = link {
            header[157..157 + link.len()].copy_from_slice(link.as_bytes());
        }
        header[148..156].copy_from_slice(b"        ");
        let sum: u32 = header.iter().map(|&b| b as u32).sum();
        header[148..156].copy_from_slice(format!("{:06o}\0 ", sum).as_bytes());
        builder.get_mut().extend_from_slice(&header);
        builder.get_mut().extend_from_slice(data);
        let pad = (512 - data.len() % 512) % 512;
        builder.get_mut().extend_from_slice(&vec![0u8; pad]);
    }

    #[test]
    fn files_under_one_folder_are_written_bare() {
        let root = temp_dir("plain");
        let path = archive(&root, |b| {
            raw(b, "ProjectAvalanche/project-avalanche", b'0', None, b"launcher");
            raw(b, "ProjectAvalanche/avalanche-game", b'0', None, b"game");
            raw(b, "ProjectAvalanche/README.txt", b'0', None, b"read me");
        });
        let out = root.join("out");
        std::fs::create_dir(&out).unwrap();
        let written = extract(&path, &out).unwrap();
        assert_eq!(written, ["project-avalanche", "avalanche-game", "README.txt"]);
        assert_eq!(std::fs::read(out.join("avalanche-game")).unwrap(), b"game");
        #[cfg(unix)]
        for (name, exec) in [("project-avalanche", true), ("avalanche-game", true), ("README.txt", false)] {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(out.join(name)).unwrap().permissions().mode();
            assert_eq!(mode & 0o100 != 0, exec, "{name}: {mode:o}");
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_fonts_are_written_in_their_folder() {
        let root = temp_dir("fonts");
        let game = super::super::install::game_name();
        for (tag, prefix) in [("bare", "./"), ("folder", "ProjectAvalanche/")] {
            let path = archive(&root, |b| {
                raw(b, &format!("{prefix}{game}"), b'0', None, b"game");
                raw(b, &format!("{prefix}fonts/NotoSans-Regular.ttf"), b'0', None, b"font");
                raw(b, &format!("{prefix}fonts/OFL.txt"), b'0', None, b"license");
                raw(b, &format!("{prefix}fonts/Other.ttf"), b'0', None, b"not ours");
            });
            let out = root.join(tag);
            std::fs::create_dir(&out).unwrap();
            let written = extract(&path, &out).unwrap();
            assert_eq!(written, [game, "fonts/NotoSans-Regular.ttf", "fonts/OFL.txt"], "{tag}");
            assert_eq!(std::fs::read(out.join("fonts").join("NotoSans-Regular.ttf")).unwrap(), b"font");
            assert!(!out.join("fonts").join("Other.ttf").exists());
        }
        let path = archive(&root, |b| raw(b, &format!("fonts/{game}"), b'0', None, b"game"));
        let out = root.join("named-fonts");
        std::fs::create_dir(&out).unwrap();
        assert_eq!(extract(&path, &out).unwrap(), [game]);
        for bad in ["ProjectAvalanche/other/NotoSans-Regular.ttf", "a/b/fonts/NotoSans-Regular.ttf", "fonts/../x"] {
            let path = archive(&root, |b| raw(b, bad, b'0', None, b"x"));
            let out = root.join(format!("bad-{}", bad.len()));
            std::fs::create_dir(&out).unwrap();
            assert!(extract(&path, &out).is_err(), "{bad}");
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_path_that_leaves_the_folder_is_refused() {
        let root = temp_dir("escape");
        let path = archive(&root, |b| raw(b, "ProjectAvalanche/../../evil", b'0', None, b"x"));
        let out = root.join("out");
        std::fs::create_dir(&out).unwrap();
        assert!(extract(&path, &out).is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_link_is_refused() {
        let root = temp_dir("link");
        let path = archive(&root, |b| raw(b, "ProjectAvalanche/project-avalanche", b'2', Some("/etc/passwd"), b""));
        let out = root.join("out");
        std::fs::create_dir(&out).unwrap();
        assert!(extract(&path, &out).is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_name_twice_is_refused() {
        let root = temp_dir("twice");
        let path = archive(&root, |b| {
            raw(b, "a/avalanche-game", b'0', None, b"one");
            raw(b, "avalanche-game", b'0', None, b"two");
        });
        let out = root.join("out");
        std::fs::create_dir(&out).unwrap();
        assert!(extract(&path, &out).is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
