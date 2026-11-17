//! A port addition for mods: the menus' 3-D rider showcase (the 3D Rider Select mod). On the equipment screen's
//! rider step the highlighted rider stands in 3-D, in its suit and gear, doing its idle, in its card in the list (the
//! one showing its portrait pictures, not faded) in place of its portrait; the others keep their portraits. A script switches it on
//! (`setRiderShowcase`); the game does the rest.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(false);
/// The highlighted rider's portrait pictures' name (`mac`): its card in the list is the one with those pictures.
static PORTRAIT: Mutex<String> = Mutex::new(String::new());
static PORTRAIT_HIDDEN: AtomicBool = AtomicBool::new(false);
/// The highlighted card's portrait on the screen (fractions of the drawing's width and height: x0, y0, x1, y1): as
/// found while the menus were last drawn, and as being found in the drawing under way (with its size in pixels).
static SLOT: Mutex<Option<[f32; 4]>> = Mutex::new(None);
static FOUND: Mutex<(Option<[f32; 4]>, (f32, f32))> = Mutex::new((None, (1.0, 1.0)));

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn set_highlight_portrait(name: &str) {
    if let Ok(mut p) = PORTRAIT.lock()
        && *p != name
    {
        *p = name.to_string();
    }
}

pub fn highlight_portrait() -> String {
    PORTRAIT.lock().map(|p| p.clone()).unwrap_or_default()
}

/// While its 3-D rider shows: the highlighted card's portrait isn't drawn.
pub fn set_portrait_hidden(on: bool) {
    PORTRAIT_HIDDEN.store(on, Ordering::Relaxed);
}

pub fn portrait_hidden() -> bool {
    PORTRAIT_HIDDEN.load(Ordering::Relaxed)
}

/// The menus' drawing starts (nothing found yet), `width` x `height` pixels.
pub fn begin_frame(width: u32, height: u32) {
    if let Ok(mut f) = FOUND.lock() {
        *f = (None, (width.max(1) as f32, height.max(1) as f32));
    }
}

/// Whether the highlighted card was found yet in the drawing under way.
pub fn slot_found() -> bool {
    FOUND.lock().is_ok_and(|f| f.0.is_some())
}

/// The highlighted card's portrait, found while drawing (in the drawing's pixels); the first one counts.
pub fn found_slot(rect: [f32; 4]) {
    if let Ok(mut f) = FOUND.lock()
        && f.0.is_none()
    {
        let (w, h) = f.1;
        f.0 = Some([rect[0] / w, rect[1] / h, rect[2] / w, rect[3] / h]);
    }
}

/// The menus' drawing ends: what it found is the slot.
pub fn end_frame() {
    let found = FOUND.lock().ok().and_then(|f| f.0);
    if let Ok(mut s) = SLOT.lock() {
        *s = found;
    }
}

/// Where the highlighted card's portrait is (fractions of the screen), when the menus show it.
pub fn slot() -> Option<[f32; 4]> {
    SLOT.lock().ok().and_then(|s| *s)
}

static DRAGGED: AtomicBool = AtomicBool::new(false);

/// Whether a point of the screen (fractions of its width and height) is on the 3-D rider (its card's view).
pub fn over_model(x: f32, y: f32) -> bool {
    (portrait_hidden() || gear_view()) && slot().is_some_and(|[x0, y0, x1, y1]| (x0..=x1).contains(&x) && (y0..=y1).contains(&y))
}

/// A press on the 3-D rider turned it (a drag), rather than being a click: the menus then don't get it.
pub fn set_dragged(on: bool) {
    DRAGGED.store(on, Ordering::Relaxed);
}

pub fn dragged() -> bool {
    DRAGGED.load(Ordering::Relaxed)
}

/// The list's riders in the game's order (their portraits' names): the rider step lays its cards out by it.
static ORDER: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn set_list_order(names: Vec<String>) {
    if let Ok(mut o) = ORDER.lock()
        && *o != names
    {
        *o = names;
    }
}

pub fn list_order() -> Vec<String> {
    ORDER.lock().map(|o| o.clone()).unwrap_or_default()
}

/// The equipment screen's gear steps (past the rider step): the chosen rider stands in 3-D in a view of its own on the
/// left (in place of its card), the screen's options in a column to its right.
static GEAR_VIEW: AtomicBool = AtomicBool::new(false);

pub fn set_gear_view(on: bool) {
    GEAR_VIEW.store(on, Ordering::Relaxed);
}

pub fn gear_view() -> bool {
    GEAR_VIEW.load(Ordering::Relaxed)
}

/// The favourite suits' items (the 3D Rider Select mod), by their cells: the gear list's and the full list's (as
/// shown now). Their items show a star.
/// (the full list's: each of its items, and the highlighted one's place in it: its scrolling grid's cells are known by
/// where they are from the highlight)
static FAVOURITE_CELLS: Mutex<(Vec<bool>, Vec<bool>, Option<usize>)> = Mutex::new((Vec::new(), Vec::new(), None));

pub fn set_favourite_cells(gear: Vec<bool>, full: Vec<bool>, highlighted: Option<usize>) {
    if let Ok(mut f) = FAVOURITE_CELLS.lock()
        && (f.0 != gear || f.1 != full || f.2 != highlighted)
    {
        *f = (gear, full, highlighted);
    }
}

pub fn favourite_cells() -> (Vec<bool>, Vec<bool>, Option<usize>) {
    FAVOURITE_CELLS.lock().map(|f| f.clone()).unwrap_or_default()
}

/// The equipment's full list (View More) is up, on the gear steps: the chosen rider stands in its view there too, the
/// list moved right of it.
static FULL_LIST_VIEW: AtomicBool = AtomicBool::new(false);

pub fn set_full_list_view(on: bool) {
    FULL_LIST_VIEW.store(on, Ordering::Relaxed);
}

pub fn full_list_view() -> bool {
    FULL_LIST_VIEW.load(Ordering::Relaxed)
}

/// The rider's view's (and its frame's) long moves (from the list's card to the gear view's place, and back): this
/// long, eased in and out.
pub const VIEW_MOVE_SECS: f32 = 0.5;

/// How far along a long move is (0 to 1) this far into it (0 to 1): starting and stopping gently.
pub fn view_move_ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// A rectangle moving to where it should be: a long way (more than `far` on any side) in one eased move of
/// `VIEW_MOVE_SECS`, a short way (the place nudged) as before. `mv` the move under way: where from, and how long it's
/// been going.
pub fn view_step(cur: [f32; 4], to: [f32; 4], dt: f32, far: f32, rate: f32, mv: &mut Option<([f32; 4], f32)>) -> [f32; 4] {
    let away = |a: [f32; 4], b: [f32; 4]| a.iter().zip(&b).any(|(x, y)| (x - y).abs() > far);
    if mv.is_none() && away(cur, to) {
        *mv = Some((cur, 0.0));
    }
    if let Some((from, t)) = mv.as_mut() {
        *t += dt;
        let p = view_move_ease(*t / VIEW_MOVE_SECS);
        let (from, done) = (*from, *t >= VIEW_MOVE_SECS);
        if done {
            *mv = None;
            return to;
        }
        return std::array::from_fn(|i| from[i] + (to[i] - from[i]) * p);
    }
    let k = 1.0 - (-dt * rate).exp();
    std::array::from_fn(|i| cur[i] + (to[i] - cur[i]) * k)
}

/// The rider step's highlighted rider has a level for sale (the equipment screen's X "Purchase Level" prompt and price
/// show): drawn at the highlighted card's bottom right, the rider's description under them.
static LEVEL_OFFER: AtomicBool = AtomicBool::new(false);

pub fn set_level_offer(on: bool) {
    LEVEL_OFFER.store(on, Ordering::Relaxed);
}

pub fn level_offer() -> bool {
    LEVEL_OFFER.load(Ordering::Relaxed)
}
