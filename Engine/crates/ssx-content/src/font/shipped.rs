use std::fmt;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug)]
pub struct Shipped {
    pub id: &'static str,
    pub file: &'static str,
    pub scripts: &'static [&'static str],
}

pub static SHIPPED: [Shipped; 6] = [
    Shipped { id: "ava:font/fallback/noto_sans", file: "NotoSans-Regular.ttf", scripts: &["Latn", "Grek", "Cyrl"] },
    Shipped { id: "ava:font/fallback/noto_sans_arabic", file: "NotoSansArabic-Regular.ttf", scripts: &["Arab"] },
    Shipped { id: "ava:font/fallback/noto_sans_hebrew", file: "NotoSansHebrew-Regular.ttf", scripts: &["Hebr"] },
    Shipped {
        id: "ava:font/fallback/noto_sans_devanagari",
        file: "NotoSansDevanagari-Regular.ttf",
        scripts: &["Deva"],
    },
    Shipped { id: "ava:font/fallback/noto_sans_thai", file: "NotoSansThai-Regular.ttf", scripts: &["Thai"] },
    Shipped {
        id: "ava:font/fallback/noto_sans_cjk",
        file: "NotoSansCJKjp-Regular.otf",
        scripts: &["Hira", "Kana", "Hani", "Hang", "Bopo"],
    },
];

pub const FOLDER: &str = "fonts";
pub const LICENSE_FILE: &str = "OFL.txt";
pub const MANIFEST_FILE: &str = "fonts.toml";

pub const LICENSE: &str = include_str!("../../../../assets/fonts/OFL.txt");

pub const MANIFEST: &str = include_str!("../../../../assets/fonts/fonts.toml");

const MAX_MANIFEST: u64 = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Expected {
    pub bytes: u64,
    pub sha256: [u8; 32],
}

pub fn expected(file: &str) -> Option<Expected> {
    static TABLE: OnceLock<Vec<(String, Expected)>> = OnceLock::new();
    TABLE.get_or_init(|| records(MANIFEST)).iter().find(|(f, _)| f == file).map(|(_, e)| *e)
}

fn records(manifest: &str) -> Vec<(String, Expected)> {
    let Ok(table) = toml::from_str::<toml::Table>(manifest) else { return Vec::new() };
    let Some(fonts) = table.get("font").and_then(|f| f.as_array()) else { return Vec::new() };
    fonts
        .iter()
        .filter_map(|f| {
            let file = f.get("file")?.as_str()?;
            let bytes = u64::try_from(f.get("bytes")?.as_integer()?).ok()?;
            let sha256 = unhex(f.get("sha256")?.as_str()?)?;
            Some((file.to_string(), Expected { bytes, sha256 }))
        })
        .collect()
}

fn unhex(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 || !text.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(text.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    NoFolder,
    NotInManifest(String),
    Missing(PathBuf),
    NotPlain(PathBuf),
    Unreadable(PathBuf, String),
    Size { path: PathBuf, bytes: u64, want: u64 },
    Content(PathBuf),
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refused::NoFolder => write!(f, "no `{FOLDER}` folder beside the program"),
            Refused::NotInManifest(file) => write!(f, "{file} is not in the program's font manifest"),
            Refused::Missing(p) => write!(f, "{} is missing", p.display()),
            Refused::NotPlain(p) => write!(f, "{} is not a plain file", p.display()),
            Refused::Unreadable(p, e) => write!(f, "{} does not read: {e}", p.display()),
            Refused::Size { path, bytes, want } if bytes > want => {
                write!(f, "{} is longer than the {want} bytes the manifest records", path.display())
            }
            Refused::Size { path, bytes, want } => {
                write!(f, "{} has {bytes} bytes, the manifest records {want}", path.display())
            }
            Refused::Content(p) => write!(f, "{} is not the file the manifest records (another SHA-256)", p.display()),
        }
    }
}

pub fn folder() -> Option<&'static Path> {
    static FOUND: OnceLock<Option<PathBuf>> = OnceLock::new();
    FOUND
        .get_or_init(|| {
            let exe = std::env::current_exe().ok();
            let beside = exe.as_deref().and_then(Path::parent).map(|d| d.join(FOLDER));
            if let Some(dir) = beside.filter(|d| d.is_dir()) {
                return Some(dir);
            }
            let mut starts: Vec<PathBuf> = Vec::new();
            starts.extend(std::env::current_dir().ok());
            starts.extend(exe.as_deref().and_then(Path::parent).map(Path::to_path_buf));
            starts.iter().find_map(|start| {
                start.ancestors().find_map(|a| {
                    [a.join("assets").join("fonts"), a.join("Engine").join("assets").join("fonts")]
                        .into_iter()
                        .find(|d| holds_this_manifest(d))
                })
            })
        })
        .as_deref()
}

fn holds_this_manifest(dir: &Path) -> bool {
    let Ok(Some(file)) = ssx_files::open_plain(&dir.join(MANIFEST_FILE)) else { return false };
    let mut text = Vec::new();
    file.take(MAX_MANIFEST + 1).read_to_end(&mut text).is_ok() && text == MANIFEST.as_bytes()
}

pub fn read(i: usize) -> Result<Vec<u8>, Refused> {
    let s = SHIPPED.get(i).ok_or_else(|| Refused::NotInManifest(format!("shipped font {i}")))?;
    let dir = folder().ok_or(Refused::NoFolder)?;
    read_checked(&dir.join(s.file), s.file)
}

pub fn read_checked(path: &Path, file: &str) -> Result<Vec<u8>, Refused> {
    let want = expected(file).ok_or_else(|| Refused::NotInManifest(file.to_string()))?;
    let handle = match ssx_files::open_plain(path) {
        Ok(Some(f)) => f,
        Ok(None) => return Err(Refused::NotPlain(path.to_path_buf())),
        Err(e) if e.kind() == ErrorKind::NotFound => return Err(Refused::Missing(path.to_path_buf())),
        Err(e) => return Err(Refused::Unreadable(path.to_path_buf(), e.to_string())),
    };
    let mut bytes = Vec::new();
    handle
        .take(want.bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| Refused::Unreadable(path.to_path_buf(), e.to_string()))?;
    let got = bytes.len() as u64;
    if got != want.bytes {
        return Err(Refused::Size { path: path.to_path_buf(), bytes: got, want: want.bytes });
    }
    if Sha256::digest(&bytes)[..] != want.sha256[..] {
        return Err(Refused::Content(path.to_path_buf()));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use super::{MANIFEST, Refused, SHIPPED, expected, read, read_checked};
    use crate::font::Limits;
    use crate::font::lazy::{Bytes, GlyphFont};

    #[test]
    fn shipped_fonts_match_their_manifest() {
        let manifest: toml::Table = toml::from_str(MANIFEST).expect("fonts.toml");
        let fonts = manifest["font"].as_array().expect("[[font]]");
        assert_eq!(fonts.len(), SHIPPED.len());
        for (i, (entry, s)) in fonts.iter().zip(&SHIPPED).enumerate() {
            assert_eq!(entry["id"].as_str(), Some(s.id));
            assert_eq!(entry["file"].as_str(), Some(s.file));
            let scripts: Vec<&str> =
                entry["scripts"].as_array().expect("scripts").iter().filter_map(|v| v.as_str()).collect();
            assert_eq!(scripts, s.scripts, "{}", s.file);
            let want = expected(s.file).expect(s.file);
            assert_eq!(entry["bytes"].as_integer(), i64::try_from(want.bytes).ok(), "{}", s.file);
            let bytes = read(i).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(bytes.len() as u64, want.bytes, "{}", s.file);
            for key in ["release", "asset"] {
                let url = entry[key].as_str().expect(key);
                assert!(url.starts_with("https://github.com/notofonts/"), "{}: {url}", s.file);
            }
        }
    }

    #[test]
    fn the_fonts_folder_holds_the_set_its_license_and_its_manifest() {
        let dir = super::folder().expect("the source tree's fonts folder");
        let mut held: Vec<String> = std::fs::read_dir(dir)
            .expect("listing")
            .map(|e| e.expect("entry").file_name().into_string().expect("a UTF-8 name"))
            .collect();
        held.sort();
        let mut want: Vec<String> = SHIPPED.iter().map(|s| s.file.to_string()).collect();
        want.extend([super::LICENSE_FILE.to_string(), super::MANIFEST_FILE.to_string()]);
        want.sort();
        assert_eq!(held, want);
    }

    #[test]
    fn a_file_other_than_the_manifests_is_refused() {
        let file = "NotoSansHebrew-Regular.ttf";
        let i = SHIPPED.iter().position(|s| s.file == file).expect(file);
        let good = read(i).unwrap_or_else(|e| panic!("{e}"));
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let dir: PathBuf = std::env::temp_dir().join(format!("ssx-fonts-{}-{nanos}", std::process::id()));
        std::fs::create_dir(&dir).expect("temp folder");
        let at = |name: &str| dir.join(name);
        std::fs::write(at("good"), &good).expect("write");
        assert_eq!(read_checked(&at("good"), file).as_deref(), Ok(&good[..]));
        assert_eq!(read_checked(&at("none"), file), Err(Refused::Missing(at("none"))));
        std::fs::write(at("short"), &good[..good.len() - 1]).expect("write");
        assert!(matches!(read_checked(&at("short"), file), Err(Refused::Size { .. })));
        let mut long = good.clone();
        long.extend(std::iter::repeat_n(0u8, 4096));
        std::fs::write(at("long"), &long).expect("write");
        let want = good.len() as u64;
        assert_eq!(read_checked(&at("long"), file), Err(Refused::Size { path: at("long"), bytes: want + 1, want }));
        let mut other = good.clone();
        other[100] ^= 1;
        std::fs::write(at("other"), &other).expect("write");
        assert_eq!(read_checked(&at("other"), file), Err(Refused::Content(at("other"))));
        std::fs::create_dir(at("folder")).expect("folder");
        assert!(matches!(read_checked(&at("folder"), file), Err(Refused::NotPlain(_) | Refused::Unreadable(..))));
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(at("good"), at("link")).expect("link");
            assert_eq!(read_checked(&at("link"), file), Err(Refused::NotPlain(at("link"))));
        }
        assert_eq!(read_checked(&at("good"), "Other.ttf"), Err(Refused::NotInManifest("Other.ttf".into())));
        std::fs::remove_dir_all(&dir).expect("clean up");
    }

    #[test]
    fn shipped_glyphs_convert_under_the_sets_caps() {
        for (i, s) in SHIPPED.iter().enumerate() {
            let bytes = read(i).unwrap_or_else(|e| panic!("{e}"));
            let font = GlyphFont::open(Bytes(Arc::from(bytes)), &Limits::shipped()).expect(s.file);
            assert!(font.characters() > 0, "{}", s.file);
            let mut session = font.session().expect(s.file);
            let stride = if font.glyphs > 10_000 { 97 } else { 1 };
            for gid in (0..font.glyphs).step_by(stride) {
                session.glyph(gid).unwrap_or_else(|e| panic!("{} glyph {gid}: {e}", s.file));
            }
        }
    }
}
