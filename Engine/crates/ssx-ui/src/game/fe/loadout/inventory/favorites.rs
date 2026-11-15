//! A port addition (the 3D Rider Select mod): suits marked favourite (Y on an owned suit), listed first and starred.
//! A suit is known by its name and perks: its duplicates (the same name and perks) are favourites with it. Kept in
//! %LOCALAPPDATA%\Project Avalanche\suit_favorites.txt, a suit a line (its name, a tab, its perks).

use std::collections::BTreeSet;
use std::sync::Mutex;

use super::UiItem;

#[derive(Default)]
struct Favourites {
    suits: BTreeSet<String>,
    /// Marked before suits were known by name and perks (their keys and sources): known by name once seen.
    by_key: BTreeSet<(u32, u8)>,
}

static FAVOURITES: Mutex<Option<Favourites>> = Mutex::new(None);

fn path() -> Option<std::path::PathBuf> {
    let dir = std::env::var_os("LOCALAPPDATA")?;
    Some(std::path::Path::new(&dir).join("Project Avalanche").join("suit_favorites.txt"))
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\t', "\\t").replace('\n', "\\n").replace('\r', "")
}

fn identity(u: &UiItem) -> String {
    format!("{}\t{}", escape(&u.name), escape(&u.description))
}

fn load() -> Favourites {
    let mut f = Favourites::default();
    let Some(text) = path().and_then(|p| std::fs::read_to_string(p).ok()) else { return f };
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        if let Some(rest) = line.strip_prefix("suit\t") {
            f.suits.insert(rest.to_string());
            continue;
        }
        let mut w = line.split_whitespace();
        let key = w.next().and_then(|k| u32::from_str_radix(k.trim_start_matches("0x"), 16).ok());
        let src = u8::from_str_radix(w.next().unwrap_or("0"), 16).ok();
        if let (Some(key), Some(src)) = (key, src) {
            f.by_key.insert((key, src));
        }
    }
    f
}

fn save(f: &Favourites) {
    let Some(p) = path() else { return };
    let mut text: String = f.suits.iter().map(|s| format!("suit\t{s}\n")).collect();
    text.extend(f.by_key.iter().map(|(k, src)| format!("{k:08x} {src:x}\n")));
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(p, text);
}

fn with<R>(f: impl FnOnce(&mut Favourites) -> R) -> R {
    let mut g = FAVOURITES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    f(g.get_or_insert_with(load))
}

pub fn is_favourite(u: &UiItem) -> bool {
    with(|f| {
        let id = identity(u);
        if f.by_key.remove(&(u.key, u.src)) {
            f.suits.insert(id.clone());
            save(f);
        }
        f.suits.contains(&id)
    })
}

/// Marks a suit (and its duplicates) favourite, or not any more: whether it is now.
pub fn toggle(u: &UiItem) -> bool {
    with(|f| {
        let id = identity(u);
        f.by_key.remove(&(u.key, u.src));
        let now = if f.suits.remove(&id) {
            false
        } else {
            f.suits.insert(id);
            true
        };
        save(f);
        now
    })
}
