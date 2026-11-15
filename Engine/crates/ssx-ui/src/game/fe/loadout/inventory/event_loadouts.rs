//! A port addition: each kind of event (its mode: Trick It, Race It, Survive It...) keeps the loadout last chosen
//! for it, per rider; playing that kind of event again starts with it (until it's changed). Kept in
//! %LOCALAPPDATA%\Project Avalanche\event_loadouts.txt, a slot a line: rider, mode, kind, slot, item key and source
//! (hex, but the mode).

use std::collections::BTreeMap;
use std::sync::Mutex;

/// (rider, event type, kind, slot) -> (item key, source)
type Loadouts = BTreeMap<(u32, String, usize, usize), (u32, u8)>;

static LOADOUTS: Mutex<Option<Loadouts>> = Mutex::new(None);

fn path() -> Option<std::path::PathBuf> {
    let dir = std::env::var_os("LOCALAPPDATA")?;
    Some(std::path::Path::new(&dir).join("Project Avalanche").join("event_loadouts.txt"))
}

fn load() -> Loadouts {
    let mut out = Loadouts::new();
    let Some(text) = path().and_then(|p| std::fs::read_to_string(p).ok()) else { return out };
    for line in text.lines() {
        let w: Vec<&str> = line.split_whitespace().collect();
        let [rider, event, kind, slot, key, src] = w[..] else { continue };
        let hex = |s: &str| u32::from_str_radix(s, 16).ok();
        let (Some(rider), Ok(kind), Ok(slot), Some(key), Some(src)) =
            (hex(rider), kind.parse::<usize>(), slot.parse::<usize>(), hex(key), hex(src))
        else {
            continue;
        };
        out.insert((rider, event.to_string(), kind, slot), (key, src as u8));
    }
    out
}

fn with<R>(f: impl FnOnce(&mut Loadouts) -> R) -> R {
    let mut g = LOADOUTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    f(g.get_or_insert_with(load))
}

/// An event's kind as kept: its event type (Trick It, Race It...; its mode is the same, "ssx_explore", for all the
/// explore drops), with the event's own name for the log.
pub fn event_kind(db: &ssx_assets::career::CareerDb, drop: u32) -> Option<String> {
    let e = db.event(drop)?;
    Some(format!("type{}", e.event_type))
}

/// The item last chosen for this slot in this kind of event, by this rider.
pub fn get(rider: u32, event: &str, kind: usize, slot: usize) -> Option<(u32, u8)> {
    with(|l| l.get(&(rider, event.to_string(), kind, slot)).copied())
}

/// The item now chosen for this slot in this kind of event, by this rider (kept on disk when it changes).
pub fn set(rider: u32, event: &str, kind: usize, slot: usize, item: (u32, u8)) {
    with(|l| {
        if l.insert((rider, event.to_string(), kind, slot), item) == Some(item) {
            return;
        }
        let Some(p) = path() else { return };
        let text: String = l
            .iter()
            .map(|((r, e, k, s), (key, src))| format!("{r:08x} {e} {k} {s} {key:08x} {src:x}\n"))
            .collect();
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(p, text);
    });
}

/// The kinds of event this rider has a loadout kept for.
pub fn kinds_for(rider: u32) -> Vec<String> {
    with(|l| {
        let mut kinds: Vec<String> = l.keys().filter(|k| k.0 == rider).map(|k| k.1.clone()).collect();
        kinds.dedup();
        kinds
    })
}

fn last_rider_path() -> Option<std::path::PathBuf> {
    let dir = std::env::var_os("LOCALAPPDATA")?;
    Some(std::path::Path::new(&dir).join("Project Avalanche").join("last_rider.txt"))
}

/// The rider last chosen (a port addition: kept on disk at once, so the character select opens on them next launch
/// even when the profile wasn't saved since).
pub fn last_rider() -> Option<u32> {
    let text = std::fs::read_to_string(last_rider_path()?).ok()?;
    u32::from_str_radix(text.trim(), 16).ok().filter(|&r| r != 0)
}

pub fn set_last_rider(rider: u32) {
    if rider == 0 || last_rider() == Some(rider) {
        return;
    }
    if let Some(p) = last_rider_path() {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(p, format!("{rider:08x}\n"));
    }
}
