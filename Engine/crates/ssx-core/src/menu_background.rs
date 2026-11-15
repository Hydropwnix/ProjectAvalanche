//! A port addition for mods: the menus' own background, a picture or a video the player picks on this PC
//! (`chooseMenuBackground`), shown behind the globe. Every pick is kept in a library the player can go back to
//! (`getMenuBackgrounds`, `useMenuBackground`, `deleteMenuBackground`). Scripts ask through here; the game shows the
//! file picker, makes a video playable, shows the result, and says here how it's going (`getMenuBackground`).

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// The texture namespace of the library's previews (`draw.image("menu-background-library:texture/<id>")`).
pub const THUMB_NAMESPACE: &str = "menu-background-library";

/// What a script asks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Order {
    /// Show the file picker; the picked picture or video joins the library and becomes the background.
    Choose,
    /// Back to the game's own background (the library is kept).
    Clear,
    /// A library entry becomes the background.
    Use(String),
    /// A library entry is deleted (its files too).
    Delete(String),
}

/// How the background is.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    /// "none", "picking", "converting", "ready" or "failed".
    pub state: &'static str,
    /// "image" or "video"; empty with none.
    pub kind: &'static str,
    /// The picked file's name.
    pub name: String,
    /// How far a video's making ready has got, 0 to 1.
    pub progress: f32,
    /// What went wrong, or what is going on.
    pub message: String,
    /// The library entry showing (empty with none).
    pub id: String,
}

/// One kept background.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: String,
    /// "image" or "video".
    pub kind: &'static str,
    pub name: String,
    /// Whether its preview exists (drawn as `THUMB_NAMESPACE:texture/<id>`).
    pub thumb: bool,
}

static ORDERS: Mutex<Vec<Order>> = Mutex::new(Vec::new());
static STATUS: Mutex<Option<Status>> = Mutex::new(None);
static LIBRARY: Mutex<Vec<Entry>> = Mutex::new(Vec::new());
static THUMBS: Mutex<Option<PathBuf>> = Mutex::new(None);
static IN_USE: AtomicBool = AtomicBool::new(false);
static GLOBE_OUT: AtomicBool = AtomicBool::new(false);

/// Whether the globe is up and zoomed out (the main menu's view: the background shows around it), set each frame.
pub fn set_globe_out(out: bool) {
    GLOBE_OUT.store(out, Ordering::Relaxed);
}

pub fn globe_out() -> bool {
    GLOBE_OUT.load(Ordering::Relaxed)
}

/// A script asked about the background: a mod manages it, so with nothing picked the menus show the game's own
/// backdrop (the globe's light blue), not a package's background movie.
pub fn mark_in_use() {
    IN_USE.store(true, Ordering::Relaxed);
}

pub fn in_use() -> bool {
    IN_USE.load(Ordering::Relaxed)
}

/// Queues an order for the game (false when too many wait).
pub fn order(o: Order) -> bool {
    let Ok(mut q) = ORDERS.lock() else { return false };
    if q.len() >= 8 {
        return false;
    }
    q.push(o);
    true
}

/// The orders since the last call.
pub fn take_orders() -> Vec<Order> {
    ORDERS.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
}

pub fn set_status(s: Status) {
    if let Ok(mut g) = STATUS.lock() {
        *g = Some(s);
    }
}

pub fn status() -> Status {
    STATUS.lock().ok().and_then(|g| g.clone()).unwrap_or(Status { state: "none", ..Status::default() })
}

pub fn set_library(entries: Vec<Entry>) {
    if let Ok(mut g) = LIBRARY.lock() {
        *g = entries;
    }
}

/// The kept backgrounds, newest first.
pub fn library() -> Vec<Entry> {
    LIBRARY.lock().map(|g| g.clone()).unwrap_or_default()
}

/// Where the previews are (`<id>.thumb.png`).
pub fn set_thumb_dir(dir: PathBuf) {
    if let Ok(mut g) = THUMBS.lock() {
        *g = Some(dir);
    }
}

/// A preview's file, for an id of the library's namespace.
pub fn thumb_file(id: &str) -> Option<PathBuf> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return None;
    }
    let dir = THUMBS.lock().ok()?.clone()?;
    Some(dir.join(format!("{id}.thumb.png")))
}
