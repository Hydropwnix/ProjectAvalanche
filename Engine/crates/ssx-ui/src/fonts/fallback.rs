use std::sync::{Arc, OnceLock};

use ssx_content::font::Limits;
use ssx_content::font::lazy::Bytes;
use ssx_content::font::shipped::SHIPPED;

use super::ot::OtFont;

static OPENED: [OnceLock<Option<Arc<OtFont>>>; SHIPPED.len()] = [const { OnceLock::new() }; SHIPPED.len()];

pub fn shipped(i: usize) -> Option<Arc<OtFont>> {
    let s = SHIPPED.get(i)?;
    OPENED[i]
        .get_or_init(|| {
            let bytes = match ssx_content::font::shipped::read(i) {
                Ok(b) => b,
                Err(e) => {
                    ssx_core::eprintln!("fonts: {} unavailable: {e}", s.id);
                    return None;
                }
            };
            match OtFont::open(s.id, Bytes(Arc::from(bytes)), &Limits::shipped()) {
                Ok(f) => Some(Arc::new(f)),
                Err(e) => {
                    ssx_core::eprintln!("fonts: {} unavailable: {e}", s.id);
                    None
                }
            }
        })
        .clone()
}

/// A system font the fallback may take after the shipped ones: its file in the system's font folder, its place in
/// the file (a collection's index), and the scripts it's first choice for (ISO 15924).
struct SystemFont {
    file: &'static str,
    index: u32,
    scripts: &'static [&'static str],
}

// TODO(guess): a port choice — the fonts Windows 10 and 11 install, for the scripts the shipped fonts lack (text from
// outside the game only: a mod's song titles, a player's name; the shipped Noto CJK covers Chinese, Japanese and
// Korean); a font that isn't there, or that the loader refuses, is left out; settle: none (port feature).
const SYSTEM: [SystemFont; 22] = [
    SystemFont {
        file: "Nirmala.ttc",
        index: 0,
        scripts: &["Beng", "Gujr", "Guru", "Knda", "Mlym", "Orya", "Sinh", "Taml", "Telu", "Olck", "Sora", "Mtei"],
    },
    SystemFont {
        file: "Nirmala.ttf",
        index: 0,
        scripts: &["Beng", "Gujr", "Guru", "Knda", "Mlym", "Orya", "Sinh", "Taml", "Telu", "Olck", "Sora", "Mtei"],
    },
    SystemFont { file: "segoeui.ttf", index: 0, scripts: &["Armn", "Geor", "Lisu", "Copt", "Glag"] },
    SystemFont { file: "ebrima.ttf", index: 0, scripts: &["Ethi", "Nkoo", "Tfng", "Vaii", "Osma", "Adlm"] },
    SystemFont { file: "LeelawUI.ttf", index: 0, scripts: &["Laoo", "Khmr", "Bugi"] },
    SystemFont { file: "KhmerUI.ttf", index: 0, scripts: &["Khmr"] },
    SystemFont { file: "LaoUI.ttf", index: 0, scripts: &["Laoo"] },
    SystemFont { file: "mmrtext.ttf", index: 0, scripts: &["Mymr"] },
    SystemFont { file: "himalaya.ttf", index: 0, scripts: &["Tibt"] },
    SystemFont { file: "monbaiti.ttf", index: 0, scripts: &["Mong"] },
    SystemFont { file: "gadugi.ttf", index: 0, scripts: &["Cher", "Cans", "Osge"] },
    SystemFont { file: "javatext.ttf", index: 0, scripts: &["Java"] },
    SystemFont { file: "msyi.ttf", index: 0, scripts: &["Yiii"] },
    SystemFont { file: "ntailu.ttf", index: 0, scripts: &["Talu"] },
    SystemFont { file: "taile.ttf", index: 0, scripts: &["Tale"] },
    SystemFont { file: "phagspa.ttf", index: 0, scripts: &["Phag"] },
    SystemFont { file: "sylfaen.ttf", index: 0, scripts: &[] },
    SystemFont { file: "seguihis.ttf", index: 0, scripts: &["Runr", "Ogam", "Goth", "Ital", "Xsux", "Egyp", "Phnx"] },
    SystemFont { file: "seguisym.ttf", index: 0, scripts: &["Zsym", "Brai"] },
    SystemFont { file: "seguiemj.ttf", index: 0, scripts: &["Zsye"] },
    SystemFont { file: "tahoma.ttf", index: 0, scripts: &[] },
    SystemFont { file: "micross.ttf", index: 0, scripts: &[] },
];

static SYSTEM_OPENED: [OnceLock<Option<Arc<OtFont>>>; SYSTEM.len()] = [const { OnceLock::new() }; SYSTEM.len()];

fn system_folder() -> Option<std::path::PathBuf> {
    if cfg!(windows) {
        let root = std::env::var_os("WINDIR").or_else(|| std::env::var_os("SystemRoot"))?;
        Some(std::path::PathBuf::from(root).join("Fonts"))
    } else {
        None
    }
}

fn system(i: usize) -> Option<Arc<OtFont>> {
    let s = SYSTEM.get(i)?;
    SYSTEM_OPENED[i]
        .get_or_init(|| {
            use std::io::Read;
            let path = system_folder()?.join(s.file);
            let limits = Limits::default();
            let file = ssx_files::open_plain(&path).ok()??;
            let mut bytes = Vec::new();
            file.take(limits.max_file_bytes as u64 + 1).read_to_end(&mut bytes).ok()?;
            match OtFont::open_collection(s.file, Bytes(Arc::from(bytes)), s.index, &limits) {
                Ok(f) => {
                    ssx_core::eprintln!("fonts: the system's {} added to the fallback", s.file);
                    Some(Arc::new(f))
                }
                Err(e) => {
                    ssx_core::eprintln!("fonts: the system's {} not used: {e}", s.file);
                    None
                }
            }
        })
        .clone()
}

/// Every fallback font: the shipped ones, then the system's.
pub fn all() -> impl Iterator<Item = Arc<OtFont>> {
    (0..SHIPPED.len()).filter_map(shipped).chain((0..SYSTEM.len()).filter_map(system))
}

pub fn for_cluster(script: &str, chars: &[char]) -> Option<Arc<OtFont>> {
    let draws = |f: &OtFont| chars.iter().all(|&c| f.covers(c) || ignorable(c));
    let own = SHIPPED.iter().position(|s| s.scripts.contains(&script));
    if let Some(f) = own.and_then(shipped).filter(|f| draws(f)) {
        return Some(f);
    }
    // A system font made for the script, then any font that draws the cluster.
    if let Some(f) =
        (0..SYSTEM.len()).filter(|&i| SYSTEM[i].scripts.contains(&script)).filter_map(system).find(|f| draws(f))
    {
        return Some(f);
    }
    (0..SHIPPED.len())
        .filter(|&i| Some(i) != own)
        .filter_map(shipped)
        .chain((0..SYSTEM.len()).filter_map(system))
        .find(|f| draws(f))
}

pub fn draws(c: char) -> bool {
    all().any(|f| f.covers(c))
}

pub fn ignorable(c: char) -> bool {
    icu_properties::CodePointSetData::new::<icu_properties::props::DefaultIgnorableCodePoint>().contains(c)
}
