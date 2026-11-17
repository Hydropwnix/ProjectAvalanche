mod lists;

use crate::avm1::Ctx;
use crate::avm1::pointer::PointerEvent;
use crate::display::InstanceId;
use crate::display::hit::Hit;

use super::GameUi;
use super::input::{Button, Input, KEYBOARD_DEVICE};
use lists::Reach;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

impl MouseButton {
    fn index(self) -> usize {
        match self {
            MouseButton::Left => 0,
            MouseButton::Right => 1,
            MouseButton::Middle => 2,
        }
    }
}

const MAX_WHEEL_STEPS: i32 = 8;
const MAX_FOCUS_STEPS: u32 = 64;

#[derive(Clone, Debug, Default)]
struct Latch {
    position: Option<(f32, f32)>,
    down: [bool; 3],
    went_down: [bool; 3],
    wheel: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub menu: InstanceId,
    pub item: i32,
}

// PORT(choice): a hover on an entry of a list the pad drives presses the pad's D-pad (one step a read, toward the
// entry's slot, the other axis when a step moved nothing within 8 reads, at most 64 steps) until the list's selection
// is that entry, and a click is the pad's A on it once selected; an entry a carousel shows outside the band its
// selection stays in (`components.RiderMenu`'s `mUnselectableLeft` / `mUnselectableRight` cells, a `GridMenu`'s
// unselectable grids) takes no hover — a step toward it turns the carousel and puts another item under a pointer that
// did not move — and a click on it only selects it; the wheel's notches are capped at 8 a read; the console has no
// pointer: the disc's lists answer only the pad's input.
#[derive(Clone, Copy, Debug)]
struct Focus {
    entry: Entry,
    step: Step<i32>,
}

#[derive(Clone, Copy, Debug)]
struct SlotFocus {
    entry: Entry,
    slot: i32,
    step: Step<i32>,
    click: bool,
    /// Reads still to wait with the option selected before the first step (the option's own slot settles first).
    settle: u32,
}

/// Reads waited, the option selected, before stepping to a slot.
const SLOT_SETTLE: u32 = 2;

/// At most this many left / right steps to a slot.
const MAX_SLOT_STEPS: u32 = 8;

#[derive(Clone, Copy, Debug, Default)]
struct Step<T> {
    at: Option<T>,
    since: u32,
    steps: u32,
    tried: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Next {
    Step,
    Wait,
    Stalled,
}

// (a port mod: a read, not 8: the selection follows the pointer at once (a step a frame), the menus answering a step
// within the frame after it)
const STEP_WAIT: u32 = 1;
const START_RUN_WAIT: u32 = 6;

impl<T: PartialEq + Copy> Step<T> {
    fn next(&mut self, now: T, max_steps: u32, ways: usize) -> Next {
        self.next_waiting(now, max_steps, ways, STEP_WAIT)
    }

    fn next_waiting(&mut self, now: T, max_steps: u32, ways: usize, wait: u32) -> Next {
        if let Some(at) = self.at {
            if at == now {
                self.since += 1;
                if self.since <= wait {
                    return Next::Wait;
                }
                if self.tried + 1 >= ways {
                    return Next::Stalled;
                }
                self.tried += 1;
            } else {
                self.tried = 0;
            }
            self.at = None;
        }
        if self.steps >= max_steps { Next::Stalled } else { Next::Step }
    }

    fn made(&mut self, at: T) {
        self.at = Some(at);
        self.since = 0;
        self.steps += 1;
    }
}

#[derive(Default)]
pub struct PointerRouter {
    latch: Latch,
    read_position: Option<(f32, f32)>,
    read_down: [bool; 3],
    release: Vec<Button>,
    focus: Option<Focus>,
    click: Option<Entry>,
    /// The equipment screen's Gear and Mods options (a port addition): the slot of the option under the pointer the
    /// selection is moving to (left / right within the option), and whether a click waits for it (A once there).
    slot: Option<SlotFocus>,
    globe: Option<GlobeFocus>,
    globe_click: Option<u32>,
    /// The ranges' globe pulled with the left button (a port addition): where the pointer last was, and how far it
    /// has gone since the button went down (a left click that never moved is a click).
    globe_drag: Option<((f32, f32), f32)>,
    #[cfg(feature = "net")]
    tab: Option<(super::fe::ava::browser::Tab, Step<super::fe::ava::browser::Tab>)>,
    pub log: Vec<String>,
}

const MAX_LOG_LINES: usize = 256;
const MAX_LOG_CHARS: usize = 400;

impl PointerRouter {
    fn note(&mut self, line: String) {
        if self.log.len() >= MAX_LOG_LINES {
            self.log.remove(0);
        }
        self.log.push(ssx_core::text::shown_max(&line, MAX_LOG_CHARS).to_string());
    }
}

#[derive(Clone, Copy, Debug)]
struct GlobeFocus {
    key: u32,
    step: Step<u32>,
}

// PORT(choice): the pointer steers the ranges' globe with a light push of the stick (0.26 on its larger axis) toward
// the hovered node's billboard, so the pick follows its direction while the globe turns little; the console has no
// pointer.
const STICK_PUSH: f32 = 0.26;

const MAX_GLOBE_STEPS: u32 = 32;

/// A peak's drops: the nearest within this many stage pixels of the pointer is the one under it.
const DROP_REACH: f32 = 90.0;

/// The ranges' globe: the pointer moved this far (stage pixels) with the left button down is a drag, not a click.
const DRAG_START: f32 = 4.0;

#[derive(Default)]
struct Under {
    prompts_shown: bool,
    globe: Option<(u32, usize)>,
    entry: Option<(Entry, usize)>,
    preview: bool,
    prompt: Option<(Button, usize)>,
}

impl GameUi {
    pub fn pointer_move(&mut self, x: f32, y: f32) {
        if x.is_finite() && y.is_finite() {
            self.pointer.latch.position = Some((x, y));
        }
    }

    pub fn pointer_button(&mut self, b: MouseButton, down: bool) {
        let i = b.index();
        self.pointer.latch.down[i] = down;
        self.pointer.latch.went_down[i] |= down;
    }

    pub fn pointer_position(&self) -> Option<(f32, f32)> {
        self.pointer.latch.position
    }

    pub fn pointer_wheel(&mut self, notches: i32) {
        let w = &mut self.pointer.latch.wheel;
        *w = w.saturating_add(notches).clamp(-MAX_WHEEL_STEPS, MAX_WHEEL_STEPS);
    }

    pub(super) fn pointer_lost(&mut self) {
        self.pointer.latch.went_down = [false; 3];
        self.pointer.latch.wheel = 0;
    }

    pub(super) fn pointer_read(&mut self, cx: &mut Ctx) {
        // (a port mod: where the test peak's markers are drawn, for its path)
        let markers = match self.fe.as_ref() {
            Some(fe) if crate::game::fe::globe::test_peak_drop_screen(fe) => {
                let order = lists::paint_order(cx);
                // (each where it's drawn: the middle of what it draws, on the stage (the clip itself sits at 0, 0,
                // what it draws moved out to the marker's place))
                crate::game::fe::globe::test_peak_markers(fe, lists::billboards(cx, &order))
                    .into_iter()
                    .filter_map(|(p, id, chosen)| {
                        let r = crate::display::hit::drawn_bounds(cx.player, cx.lib, id)
                            .transform(&cx.player.world_matrix(id));
                        (!r.is_empty()).then(|| (p, ((r.x0 + r.x1) / 40.0, (r.y0 + r.y1) / 40.0), id, chosen))
                    })
                    .collect()
            }
            _ => Vec::new(),
        };
        crate::draw::render::set_test_peak_markers(markers);
        for b in std::mem::take(&mut self.pointer.release) {
            self.input.push_from(Input::Button { button: b, down: false }, KEYBOARD_DEVICE);
        }
        let latch = self.pointer.latch.clone();
        self.pointer.latch.went_down = [false; 3];
        self.pointer.latch.wheel = 0;
        let pressed = |i: usize| latch.went_down[i] || (latch.down[i] && !self.pointer.read_down[i]);
        let released = |i: usize| !latch.down[i] && (self.pointer.read_down[i] || latch.went_down[i]);
        let (mut left_down, left_up, right_down) = (pressed(0), released(0), pressed(1));
        let middle_down = pressed(2);
        self.pointer.read_down = latch.down;
        // (a port mod (the test peak): the wheel's click on its drops turns its events between Trick It and Race It,
        // as Y does)
        // (the 3D Rider Select mod: the wheel's click on a suit, board or piece of gear marks it favourite, as Y does)
        if middle_down
            && let Some(fe) = self.fe.as_mut()
            && crate::game::fe::equipment::take_suit_press(fe, Button::DigitalThree)
        {
            self.pointer.note("pointer: the wheel's click on the gear: Y (favourite)".into());
        }
        if middle_down && self.fe.as_ref().is_some_and(crate::game::fe::globe::test_peak_drop_screen) {
            self.pointer.note("pointer: the wheel's click on the test peak: Y (switch event)".into());
            self.fe_call(|fe, _| {
                crate::game::fe::globe::take_test_peak_press(fe, Button::DigitalThree);
            });
        }
        let Some(pos) = latch.position else { return };
        let moved = self.pointer.read_position != Some(pos);
        self.pointer.read_position = Some(pos);
        // (a port addition: on the ranges' globe the left button held pulls the globe round; a left click that didn't
        // move it is a click, given as the button comes up)
        let range_globe = self.fe.as_ref().is_some_and(super::fe::globe::range_screen);
        if range_globe {
            if left_down {
                self.pointer.globe_drag = Some((pos, 0.0));
            }
            left_down = false;
            if let Some((from, gone)) = self.pointer.globe_drag {
                let (dx, dy) = (pos.0 - from.0, pos.1 - from.1);
                if gone + dx.abs() + dy.abs() >= DRAG_START && (dx != 0.0 || dy != 0.0) {
                    self.fe_call(|fe, _| super::fe::globe::pointer_drag(fe, dx, dy));
                }
                let gone = gone + (dx * dx + dy * dy).sqrt();
                self.pointer.globe_drag = Some((pos, gone));
                if left_up || !latch.down[0] {
                    self.pointer.globe_drag = None;
                    left_down = gone < DRAG_START;
                }
            }
        } else {
            self.pointer.globe_drag = None;
        }
        let dragging = self.pointer.globe_drag.is_some();
        // TODO(guess): a move only when the position changed (the game's handler 0x825757F0 posts one every read; whether GFx's mouse state 0x82667938 reports an unchanged point to the movies was not read); settle: 0x82667938 and the AS2 root's mouse processing.
        // (a port mod (the 3D Rider Select mod): on the gear steps, a part drawn moved is found where it's drawn)
        let raw = (pos.0 * 20.0, pos.1 * 20.0);
        let stage = crate::draw::render::pointer_place(raw);
        if moved {
            cx.vm.pointer_event(PointerEvent::Move { x: stage.0 / 20.0, y: stage.1 / 20.0 });
        }
        let need = moved
            || left_down
            || right_down
            || latch.wheel != 0
            || self.pointer.focus.is_some()
            || self.pointer.slot.is_some()
            || self.pointer.globe.is_some()
            || self.pointer_tab_pending();
        if !need {
            if left_up {
                cx.vm.pointer_event(PointerEvent::Button { down: false });
            }
            return;
        }
        let subscribers = self.input.subscribers.clone();
        let order = lists::paint_order(cx);
        let reach = lists::reach(cx, &subscribers, &order);
        // (the entries whose drawn places the drawing notes, for the next read)
        crate::draw::render::pointer_watch(reach.lists.iter().flat_map(|l| l.cells.iter().map(|c| c.clip)));
        let globe = self.globe_level().map(|level| (level, self.globe_nodes()));
        let mut under = under(cx, &order, &reach, raw);
        if let Some((level, nodes)) = &globe {
            under.globe = globe_under(cx, &order, nodes, stage);
            // (a port addition: a peak's drops sit close together: the drop nearest the pointer, near enough, is
            // under it)
            if *level == 3 && under.globe.is_none() && under.entry.is_none() {
                under.globe = globe_near(cx, &order, nodes, pos, DROP_REACH);
            }
        }
        if moved
            && let Some((e, _)) = under.entry
            && !under.preview
        {
            let same = self.pointer.focus.is_some_and(|f| f.entry == e);
            if !same {
                self.pointer.focus = Some(Focus { entry: e, step: Step::default() });
                self.pointer.click = None;
            }
        }
        // (a port addition: on the equipment screen's Gear and Mods options, the slot under the pointer is the one
        // highlighted)
        if moved {
            let on_equipment = self.fe.as_ref().is_some_and(|fe| fe.screen == super::fe::screens::id::EQUIPMENT);
            let hovered = under.entry.filter(|_| !under.preview && on_equipment).and_then(|(e, _)| {
                let found = equipment_slot_under(cx, &reach, e, raw);
                // (what was found, in the log, as it changes)
                static LAST: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
                let line = match &found {
                    Ok((slot, n)) => format!("pointer: option {} slot {slot} of {n}", e.item),
                    Err(why) => format!("pointer: no slots of option {}: {why}", e.item),
                };
                if let Ok(mut l) = LAST.lock()
                    && *l != line
                {
                    l.clone_from(&line);
                    self.pointer.note(line);
                }
                found.ok().map(|(slot, _)| (e, slot))
            });
            match hovered {
                Some((e, slot)) => {
                    if self.pointer.slot.is_none_or(|f| f.entry != e || f.slot != slot) {
                        self.pointer.slot = Some(SlotFocus {
                            entry: e,
                            slot,
                            step: Step::default(),
                            click: false,
                            settle: SLOT_SETTLE,
                        });
                    }
                }
                None => self.pointer.slot = None,
            }
        }
        if moved && !dragging {
            match under.globe {
                Some((k, _)) if self.pointer.globe.is_none_or(|g| g.key != k) => {
                    self.pointer.globe = Some(GlobeFocus { key: k, step: Step::default() });
                    self.pointer.globe_click = None;
                }
                None if self.pointer.globe.is_some() && self.pointer.globe_click.is_none() => self.globe_release(),
                _ => {}
            }
        }
        if left_down && !dragging {
            cx.vm.pointer_event(PointerEvent::Button { down: true });
            self.click_at(cx, stage, &under, &reach, &order);
        }
        if left_up {
            cx.vm.pointer_event(PointerEvent::Button { down: false });
        }
        // PORT(choice): the right button is the pad's B and the wheel the D-pad (up away from the player) on the screen
        // under it (the game reads no wheel: its mouse handler 0x825757F0 posts buttons 0…3 only, and the movies'
        // button events take the primary one); the console has no mouse.
        if right_down {
            self.pointer_press(Button::DigitalTwo, "the right button: B");
        }
        let w = latch.wheel;
        for _ in 0..w.unsigned_abs() {
            let b = if w > 0 { Button::DpadUp } else { Button::DpadDown };
            self.pointer_press(b, "the wheel");
        }
        self.focus_step(cx, &reach);
        self.slot_step(&reach);
        #[cfg(feature = "net")]
        self.tab_step();
        if let Some((level, nodes)) = globe {
            self.globe_step(level, &nodes);
        } else if self.pointer.globe.is_some() {
            self.globe_release();
        }
    }

    fn pointer_press(&mut self, b: Button, why: &str) {
        self.pointer.note(format!("pointer: {why} → {b:?}"));
        // (a port mod (the test peak): Y (a click on its "Switch Event" prompt) turns its events, as the pad's Y does)
        if b == Button::DigitalThree {
            let mut taken = false;
            self.fe_call(|fe, _| taken = crate::game::fe::globe::take_test_peak_press(fe, b));
            if taken {
                return;
            }
        }
        self.input.push_from(Input::Button { button: b, down: true }, KEYBOARD_DEVICE);
        self.pointer.release.push(b);
        if matches!(b, Button::DpadLeft | Button::DpadRight) {
            let x = if b == Button::DpadRight { 1.0 } else { -1.0 };
            self.fe_call(|fe, _| super::fe::globe::dpad(fe, x));
        }
        if matches!(b, Button::DpadUp | Button::DpadDown) {
            let y = if b == Button::DpadDown { 1.0 } else { -1.0 };
            self.fe_call(|fe, _| super::fe::globe::dpad_y(fe, y));
        }
    }

    fn pointer_tab_pending(&self) -> bool {
        #[cfg(feature = "net")]
        return self.pointer.tab.is_some();
        #[cfg(not(feature = "net"))]
        false
    }

    #[cfg(feature = "net")]
    fn tab_step(&mut self) {
        let Some((target, mut step)) = self.pointer.tab else { return };
        let Some(fe) = self.fe.as_ref() else { return };
        let open = fe.ava.browser.tab;
        let Some(b) = super::fe::ava::browser::pointer::tab_step(fe, target) else {
            self.pointer.tab = None;
            return;
        };
        match step.next(open, MAX_FOCUS_STEPS, 1) {
            Next::Wait => self.pointer.tab = Some((target, step)),
            Next::Stalled => self.pointer.tab = None,
            Next::Step => {
                step.made(open);
                self.pointer.tab = Some((target, step));
                self.pointer_press(b, &format!("the browser's tab {open:?} → {target:?}"));
            }
        }
    }

    fn globe_level(&self) -> Option<u8> {
        let fe = self.fe.as_ref()?;
        fe.globe_scene.as_ref()?;
        match fe.screen {
            0xC | 0x42 | 0xF => Some(1),
            0xD | 0x43 | 0x10 => Some(2),
            0xE | 0x44 | 0x11 => Some(3),
            _ => None,
        }
    }

    fn globe_nodes(&self) -> Vec<super::fe::globe_scene::GlobeEntry> {
        self.fe.as_ref().map(super::fe::globe::entries).unwrap_or_default()
    }

    fn globe_step(&mut self, level: u8, nodes: &[super::fe::globe_scene::GlobeEntry]) {
        let Some(mut g) = self.pointer.globe else { return };
        let selected = nodes.iter().find(|n| n.selected);
        let target = nodes.iter().find(|n| n.key == g.key);
        // (a port addition: on the ranges' globe the range under the pointer is selected at once, the globe left as
        // it is; a click on it, once selected, is the pad's A)
        if matches!(level, 1 | 3) && target.is_some() {
            let key = g.key;
            let mut ok = false;
            if level == 1 {
                self.fe_call(|fe, _| ok = super::fe::globe::pointer_focus(fe, key));
            } else {
                self.fe_call(|fe, env| ok = super::fe::globe::pointer_drop(fe, env, key));
            }
            let clicked = self.pointer.globe_click == Some(key);
            self.pointer.globe = None;
            self.pointer.globe_click = None;
            if ok && clicked {
                self.pointer_press(Button::DigitalOne, "a click on a globe node: A");
            }
            return;
        }
        let (Some(sel), Some(target)) = (selected, target) else {
            self.globe_release();
            return;
        };
        if sel.key == g.key {
            self.globe_release();
            if self.pointer.globe_click.take() == Some(g.key) {
                self.pointer_press(Button::DigitalOne, "a click on a globe node, now selected: A");
            }
            return;
        }
        match g.step.next(sel.key, MAX_GLOBE_STEPS, 1) {
            Next::Wait => {
                self.pointer.globe = Some(g);
                return;
            }
            Next::Stalled => {
                let line = format!("pointer: the globe stays on {:#010x}: no focus on {:#010x}", sel.key, g.key);
                self.pointer.note(line);
                self.globe_release();
                return;
            }
            Next::Step => {}
        }
        g.step.made(sel.key);
        self.pointer.globe = Some(g);
        let (dx, dy) = (target.x - sel.x, target.y - sel.y);
        if level == 1 {
            let len = (dx * dx + dy * dy).sqrt();
            if len < 1.0 {
                return;
            }
            let (ux, uy) = (dx / len, -dy / len);
            let k = STICK_PUSH / ux.abs().max(uy.abs());
            let (x, y) = (ux * k, uy * k);
            self.pointer.note(format!("pointer: the stick toward {:#010x} ({x:.2}, {y:.2})", g.key));
            self.fe_call(|fe, _| super::fe::globe::stick(fe, x, y));
        } else {
            let b = if dx >= 0.0 { Button::DpadRight } else { Button::DpadLeft };
            self.pointer_press(b, &format!("the globe toward {:#010x}", g.key));
        }
    }

    fn globe_release(&mut self) {
        if self.pointer.globe.take().is_some_and(|g| g.step.steps > 0) && self.globe_level() == Some(1) {
            self.fe_call(|fe, _| super::fe::globe::stick(fe, 0.0, 0.0));
        }
        self.pointer.globe_click = None;
    }

    fn click_at(&mut self, cx: &mut Ctx, stage: (f32, f32), under: &Under, reach: &Reach, order: &[InstanceId]) {
        let flash = cx.pointer_pick_at(stage);
        if let Some((b, _)) = under.prompt {
            self.pointer_press(b, "a click on a button prompt");
            return;
        }
        #[cfg(feature = "net")]
        if let Some(fe) = self.fe.as_ref() {
            use super::fe::ava::browser::pointer::{Click, click};
            match click(fe, cx, &mut self.loc, order, stage) {
                Some(Click::Press(b)) => {
                    self.pointer_press(b, "a click on the browser's line");
                    return;
                }
                Some(Click::Tab(t)) => {
                    self.pointer.note(format!("pointer: a click on the browser's tab {t:?}"));
                    self.pointer.tab = Some((t, Step::default()));
                    return;
                }
                None => {}
            }
        }
        #[cfg(not(feature = "net"))]
        let _ = order;
        if let Some(f) = flash {
            let name = cx.player.path(f);
            self.pointer.note(format!("pointer: a click on the movies' button {name}"));
            return;
        }
        if under.entry.is_none()
            && let Some((k, _)) = under.globe
        {
            let selected = self.globe_nodes().iter().find(|n| n.selected).map(|n| n.key);
            if selected == Some(k) {
                self.pointer_press(Button::DigitalOne, "a click on the globe's selected node: A");
            } else {
                self.pointer.globe = Some(GlobeFocus { key: k, step: Step::default() });
                self.pointer.globe_click = Some(k);
            }
            return;
        }
        let Some((e, _)) = under.entry else {
            // PORT(choice): a click anywhere on a screen that offers the pad no list and no prompt is that screen's A
            // (the results' continue); the console has no pointer.
            if reach.lists.is_empty() && !under.prompts_shown && under.globe.is_none() && self.globe_level().is_none() {
                self.pointer_press(Button::DigitalOne, "a click on a screen of one action: A");
            }
            return;
        };
        match reach.lists.iter().find(|l| l.clip == e.menu).map(|l| l.selected) {
            Some(s) if s == e.item => {
                if let Some(f) = self.pointer.slot.as_mut().filter(|f| f.entry == e)
                    && Some(f.slot) != self.fe.as_ref().map(|fe| fe.equipment.slot)
                {
                    // (a slot of the option not highlighted yet: A once it is)
                    f.click = true;
                } else {
                    self.pointer_press(Button::DigitalOne, "a click on the selected entry: A");
                }
            }
            Some(_) => {
                self.pointer.focus = Some(Focus { entry: e, step: Step::default() });
                self.pointer.click = (!under.preview).then_some(e);
                if under.preview {
                    self.pointer.note(format!("pointer: a click on {} outside the carousel's band: select it", e.item));
                }
            }
            None => {}
        }
    }

    fn focus_step(&mut self, cx: &mut Ctx, reach: &Reach) {
        let Some(mut f) = self.pointer.focus else { return };
        let Some(menu) = reach.lists.iter().find(|m| m.clip == f.entry.menu) else {
            self.pointer.focus = None;
            self.pointer.click = None;
            return;
        };
        if menu.selected == f.entry.item {
            self.pointer.focus = None;
            if self.pointer.click.take() == Some(f.entry) {
                if let Some(s) = self.pointer.slot.as_mut().filter(|s| s.entry == f.entry) {
                    // (a slot of the option to get to first: A once there)
                    s.click = true;
                } else {
                    self.pointer_press(Button::DigitalOne, "a click on an entry, now selected: A");
                }
            }
            return;
        }
        let ways = lists::step_ways(cx, menu, f.entry.item);
        // (a port mod: off the equipment screen's Start Run (-1) the screen changes its state first: a few reads)
        let wait = if menu.selected < 0 { START_RUN_WAIT } else { STEP_WAIT };
        match f.step.next_waiting(menu.selected, MAX_FOCUS_STEPS, ways.len(), wait) {
            Next::Wait => {
                self.pointer.focus = Some(f);
                return;
            }
            Next::Stalled => {
                let line = format!("pointer: the selection stays at {}: no focus on {}", menu.selected, f.entry.item);
                self.pointer.note(line);
                self.pointer.focus = None;
                self.pointer.click = None;
                return;
            }
            Next::Step => {}
        }
        let Some(&dir) = ways.get(f.step.tried) else {
            self.pointer.focus = None;
            self.pointer.click = None;
            return;
        };
        f.step.made(menu.selected);
        self.pointer.focus = Some(f);
        self.pointer_press(dir, &format!("focus {} → {}", menu.selected, f.entry.item));
    }
}

impl GameUi {
    /// The equipment screen's Gear and Mods options (a port addition): once the option under the pointer is the one
    /// selected, left / right (a step a read) till its slot under the pointer is the one highlighted; then A if a
    /// click waits for it.
    fn slot_step(&mut self, reach: &Reach) {
        let Some(mut f) = self.pointer.slot else { return };
        // (the option itself first)
        if self.pointer.focus.is_some_and(|x| x.entry == f.entry) {
            f.settle = SLOT_SETTLE;
            self.pointer.slot = Some(f);
            return;
        }
        let Some(menu) = reach.lists.iter().find(|m| m.clip == f.entry.menu) else {
            self.pointer.slot = None;
            return;
        };
        if menu.selected != f.entry.item {
            // (the option never came to be selected: nothing to do)
            self.pointer.slot = None;
            return;
        }
        if f.settle > 0 {
            f.settle -= 1;
            self.pointer.slot = Some(f);
            return;
        }
        let Some(current) = self.fe.as_ref().map(|fe| fe.equipment.slot) else { return };
        if current == f.slot {
            // (there: let go, so the keys and the pad move the highlight on from here as they like)
            self.pointer.slot = None;
            if f.click {
                self.pointer_press(Button::DigitalOne, "a click on a slot, now highlighted: A");
            }
            return;
        }
        match f.step.next(current, MAX_SLOT_STEPS, 1) {
            Next::Wait => {}
            Next::Stalled => {
                self.pointer.note(format!("pointer: the slot stays at {current}: no focus on slot {}", f.slot));
                self.pointer.slot = None;
                return;
            }
            Next::Step => {
                f.step.made(current);
                let dir = if f.slot > current { Button::DpadRight } else { Button::DpadLeft };
                self.pointer.slot = Some(f);
                self.pointer_press(dir, &format!("slot {current} → {}", f.slot));
                return;
            }
        }
        self.pointer.slot = Some(f);
    }
}

/// The equipment screen's Gear and Mods options (a port addition): which of the option's slots (its icons, left to
/// right) the pointer is over, for an option of more than one. (`Err`: why not, for the log.)
fn equipment_slot_under(cx: &mut Ctx, reach: &Reach, e: Entry, raw: (f32, f32)) -> Result<(i32, usize), String> {
    let list = reach.lists.iter().find(|l| l.clip == e.menu).ok_or("no list")?;
    let cell = list.cells.iter().find(|c| c.item == e.item).ok_or("no cell")?;
    // (the option's icons: its `mcSlots`, wherever it is in the option)
    let mut icons = find_named(cx.player, cell.clip, "mcSlots", 6)
        .ok_or_else(|| format!("no mcSlots under {}", cx.player.path(cell.clip)))?;
    // (the icons side by side: the first level down with more than one)
    let mut slots = Vec::new();
    for _ in 0..4 {
        slots = side_by_side(cx, icons);
        if slots.len() >= 2 {
            break;
        }
        match visible_kids(cx.player, icons).as_slice() {
            [only] => icons = *only,
            _ => break,
        }
    }
    if slots.len() < 2 {
        return Err(format!("{} slot(s) in {}", slots.len(), cx.player.path(icons)));
    }
    let stage = crate::draw::render::pointer_place_for(cx.player, icons, raw);
    let (x, _) = cx.player.to_local(icons, stage).ok_or("no local place")?;
    // (the slot whose icon the pointer is over, or the nearest one along the row, within the icons' span)
    let (lo, hi) = (slots[0].0, slots[slots.len() - 1].1);
    if x < lo || x > hi {
        return Err(format!("the pointer outside the icons ({} of them)", slots.len()));
    }
    let i = slots
        .iter()
        .enumerate()
        .min_by(|a, b| {
            let d = |r: &(f32, f32)| (r.0 - x).max(x - r.1).max(0.0);
            d(a.1).total_cmp(&d(b.1))
        })
        .map(|(i, _)| i)
        .ok_or("no slot")?;
    Ok((i32::try_from(i).map_err(|e| e.to_string())?, slots.len()))
}

fn visible_kids(player: &crate::display::Player, id: InstanceId) -> Vec<InstanceId> {
    match player.get(id).map(|i| &i.kind) {
        Some(crate::display::InstanceKind::Sprite(s)) => {
            s.children.values().copied().filter(|&k| player.get(k).is_some_and(|i| i.visible)).collect()
        }
        _ => Vec::new(),
    }
}

/// The slots the visible parts of `id` draw, by their left and right edges in its own space, left to right: parts
/// drawn over one another (an icon, its empty mark, the highlight's frame) are one slot, and a part as wide as more
/// than one slot (a backing) isn't one.
fn side_by_side(cx: &mut Ctx, id: InstanceId) -> Vec<(f32, f32)> {
    let mut parts: Vec<(f32, f32)> = visible_kids(cx.player, id)
        .into_iter()
        .filter_map(|k| {
            let m = cx.player.get(k)?.matrix;
            let r = crate::display::hit::drawn_bounds(cx.player, cx.lib, k).transform(&m);
            (!r.is_empty() && r.x1 - r.x0 > 1.0).then_some((r.x0, r.x1))
        })
        .collect();
    let Some(narrowest) = parts.iter().map(|p| p.1 - p.0).min_by(f32::total_cmp) else { return parts };
    parts.retain(|p| p.1 - p.0 <= narrowest * 1.6);
    parts.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut slots: Vec<(f32, f32)> = Vec::new();
    for p in parts {
        match slots.last_mut() {
            // (overlapping the slot before by more than half the narrower of the two: the same slot)
            Some(last) if last.1.min(p.1) - p.0 > 0.5 * (p.1 - p.0).min(last.1 - last.0) => {
                last.0 = last.0.min(p.0);
                last.1 = last.1.max(p.1);
            }
            _ => slots.push(p),
        }
    }
    slots
}

fn find_named(player: &crate::display::Player, id: InstanceId, name: &str, depth: u32) -> Option<InstanceId> {
    let kids: Vec<InstanceId> = match player.get(id).map(|i| &i.kind) {
        Some(crate::display::InstanceKind::Sprite(s)) => s.children.values().copied().collect(),
        _ => return None,
    };
    if let Some(&k) = kids.iter().find(|&&k| player.get(k).is_some_and(|i| i.name == name)) {
        return Some(k);
    }
    if depth == 0 {
        return None;
    }
    kids.into_iter().find_map(|k| find_named(player, k, name, depth - 1))
}

impl Ctx<'_> {
    pub fn pointer_pick_at(&mut self, stage: (f32, f32)) -> Option<InstanceId> {
        let saved = std::mem::replace(&mut self.vm.pointer.position, stage);
        let picked = self.pointer_pick();
        self.vm.pointer.position = saved;
        picked
    }
}

// PORT(choice): an entry is hit anywhere in the slot its menu lays it out in (`mCellWidth` × the row's height from the
// cell's `_x` / `_y`; the rows a view stacks itself — a popup's buttons, a settings page's rows, RiderNet's music rows
// — bands from each row's top to the next's, as wide as the rows are drawn together), a prompt in its own button area
// (`mcHitArea`), a globe's billboard where its body is drawn (the union of the bounds of its visible, not faded, not
// additive / screen / lighten parts; the menus' cells, prompts and billboards are not the movies' buttons, and their
// invisible, faded or glowing decorations reach over their neighbours); the console has no pointer.
fn under(cx: &mut Ctx, order: &[InstanceId], reach: &Reach, stage: (f32, f32)) -> Under {
    let rank: std::collections::HashMap<InstanceId, usize> = order.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    let prompts = reach.prompts(cx, lists::instances_of(cx, order, lists::Kind::Label));
    let mut meshes = std::mem::take(&mut cx.vm.pointer.meshes);
    let mut out = Under { prompts_shown: prompts.iter().any(|&id| lists::drawn(cx, id)), ..Under::default() };
    let mut shown_prompts = Vec::new();
    {
        let mut hit = Hit { player: cx.player, lib: cx.lib, meshes: &mut meshes };
        let mut best_depth: Option<(f32, usize)> = None;
        for l in &reach.lists {
            for c in &l.cells {
                let Some(&r) = rank.get(&c.clip) else { continue };
                if crate::draw::render::pointer_hidden(cx.player, c.clip) {
                    continue;
                }
                // (each entry where it was drawn (on the gear steps, moved): else where the menus have it)
                let on = match crate::draw::render::pointer_on_drawn(c.clip, stage) {
                    Some(on) if crate::draw::render::pointer_map_on() => on && lists::drawn(cx, c.clip),
                    _ => {
                        let stage = crate::draw::render::pointer_place_for(cx.player, c.clip, stage);
                        lists::cell_hit(cx, &mut hit, c, stage)
                    }
                };
                if !on {
                    continue;
                }
                let stage = crate::draw::render::pointer_place_for(cx.player, c.clip, stage);
                // (of entries one over another (a box reaching over its neighbour), the one the pointer's most inside
                // wins: 0 at its middle, 1 at its edge; then the one drawn on top)
                let depth = cx
                    .player
                    .get(c.clip)
                    .and_then(|i| i.parent)
                    .and_then(|p| cx.player.to_local(p, stage))
                    .map_or(1.0, |(x, y)| {
                        let s = c.slot;
                        let (hw, hh) = (((s.x1 - s.x0) * 0.5).max(1e-3), ((s.y1 - s.y0) * 0.5).max(1e-3));
                        ((x - (s.x0 + s.x1) * 0.5).abs() / hw).max((y - (s.y0 + s.y1) * 0.5).abs() / hh)
                    });
                // (a box standing apart from its list, as Start Run from the options (item -1), only when no entry of
                // the list is under the pointer: its area reaches over the first option)
                let apart = |item: i32| item < 0;
                let best_item = out.entry.map(|(e, _)| e.item);
                let better = match best_depth {
                    None => true,
                    _ if best_item.is_some_and(apart) && !apart(c.item) => true,
                    _ if apart(c.item) && best_item.is_some_and(|i| !apart(i)) => false,
                    Some((d, o)) => depth < d - 0.05 || ((depth - d).abs() <= 0.05 && r > o),
                };
                if better {
                    best_depth = Some((depth, r));
                    out.entry = Some((Entry { menu: l.clip, item: c.item }, r));
                    out.preview = c.preview;
                }
            }
        }
        for id in prompts {
            if crate::draw::render::pointer_hidden(cx.player, id) {
                continue;
            }
            let stage = crate::draw::render::pointer_place_for(cx.player, id, stage);
            let area = cx.player.child_by_name(id, "mcHitArea", true);
            let inside = match area {
                Some(a) => hit.bounds(a, stage),
                None => hit.drawn(id, stage),
            };
            if inside && hit.shown_at(id, stage) && lists::drawn(cx, id) {
                shown_prompts.push(id);
            }
        }
    }
    cx.vm.pointer.meshes = meshes;
    for id in shown_prompts {
        let Some(&r) = rank.get(&id) else { continue };
        if out.prompt.is_none_or(|(_, o)| r > o)
            && let Some(b) = lists::prompt_button(cx, id)
        {
            out.prompt = Some((b, r));
        }
    }
    out
}

pub fn targets(cx: &mut Ctx, subscribers: &[String]) -> Vec<String> {
    let px = |b: crate::geom::RectF| {
        format!("({:.0}, {:.0})–({:.0}, {:.0})", b.x0 / 20.0, b.y0 / 20.0, b.x1 / 20.0, b.y1 / 20.0)
    };
    let order = lists::paint_order(cx);
    let reach = lists::reach(cx, subscribers, &order);
    let mut out = Vec::new();
    for m in lists::instances_of(cx, &order, lists::Kind::Menu) {
        let obj = crate::avm1::value::Value::Object(cx.object_for(m));
        let (a, ps) = (lists::flag(cx, &obj, "mIsActive"), lists::flag(cx, &obj, "mIsPassive"));
        let n = |cx: &mut Ctx, k: &str| lists::number(cx, &obj, k).map_or("-".to_string(), |v| v.to_string());
        let fields = ["mFirstVisibleItemIndex", "mSelectedCellIndex", "mNumTotalItems", "mNumVisibleItems"]
            .map(|k| format!("{k} {}", n(cx, k)))
            .join(", ");
        out.push(format!("menu {} active {a} passive {ps}: {fields}", cx.player.path(m)));
    }
    for m in lists::instances_of(cx, &order, lists::Kind::Popup) {
        let obj = crate::avm1::value::Value::Object(cx.object_for(m));
        let r = lists::flag(cx, &obj, "mReceivedPublish");
        let n = |cx: &mut Ctx, k: &str| lists::number(cx, &obj, k).map_or("-".to_string(), |v| v.to_string());
        let (b, sel) = (n(cx, "mNumButtons"), n(cx, "mSelectedIndex"));
        out.push(format!("popup {} received {r} buttons {b} selected {sel}", cx.player.path(m)));
    }
    for (l, p) in lists::listeners(cx, subscribers) {
        out.push(format!("listener {} priority {p}", cx.player.path(l)));
    }
    for l in &reach.lists {
        out.push(format!("list {} selected {}", cx.player.path(l.clip), l.selected));
        for c in &l.cells {
            let parent = cx.player.get(c.clip).and_then(|i| i.parent);
            let w = parent.map_or(crate::geom::Mat2d::IDENTITY, |p| cx.player.world_matrix(p));
            let outside = if c.preview { " outside the band" } else { "" };
            out.push(format!("  entry {} {}{outside}", c.item, px(c.slot.transform(&w))));
        }
    }
    {
        for id in reach.prompts(cx, lists::instances_of(cx, &order, lists::Kind::Label)) {
            {
                let b = lists::prompt_button(cx, id);
                let r = match cx.player.child_by_name(id, "mcHitArea", true) {
                    Some(a) => crate::display::hit::stage_bounds(cx.player, cx.lib, a),
                    None => {
                        crate::display::hit::drawn_bounds(cx.player, cx.lib, id).transform(&cx.player.world_matrix(id))
                    }
                };
                out.push(format!("prompt {} {b:?} {}", cx.player.path(id), px(r)));
            }
        }
    }
    for (k, id) in lists::billboards(cx, &order) {
        let r = crate::display::hit::drawn_bounds(cx.player, cx.lib, id).transform(&cx.player.world_matrix(id));
        out.push(format!("billboard {k:#010x} {} {}", cx.player.path(id), px(r)));
    }
    for id in order {
        let Some(&obj) = cx.vm.display_objs.get(&id) else { continue };
        if crate::avm1::pointer::BUTTON_EVENTS.iter().any(|n| cx.has_property(obj, n)) {
            let r = crate::display::hit::stage_bounds(cx.player, cx.lib, id);
            out.push(format!("button {} {}", cx.player.path(id), px(r)));
        }
    }
    out
}

fn globe_under(
    cx: &mut Ctx,
    order: &[InstanceId],
    nodes: &[super::fe::globe_scene::GlobeEntry],
    stage: (f32, f32),
) -> Option<(u32, usize)> {
    let rank: std::collections::HashMap<InstanceId, usize> = order.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    let boards: Vec<(u32, InstanceId)> =
        lists::billboards(cx, order).into_iter().filter(|(k, _)| nodes.iter().any(|n| n.key == *k)).collect();
    let mut meshes = std::mem::take(&mut cx.vm.pointer.meshes);
    let mut best: Option<(u32, usize)> = None;
    {
        let mut hit = Hit { player: cx.player, lib: cx.lib, meshes: &mut meshes };
        for (k, id) in boards {
            let Some(&r) = rank.get(&id) else { continue };
            if hit.drawn(id, stage)
                && hit.shown_at(id, stage)
                && lists::drawn(cx, id)
                && best.is_none_or(|(_, o)| r > o)
            {
                best = Some((k, r));
            }
        }
    }
    cx.vm.pointer.meshes = meshes;
    best
}

/// The globe's node whose billboard's middle is nearest `at` (stage pixels), within `reach`.
fn globe_near(
    cx: &mut Ctx,
    order: &[InstanceId],
    nodes: &[super::fe::globe_scene::GlobeEntry],
    at: (f32, f32),
    reach: f32,
) -> Option<(u32, usize)> {
    let rank: std::collections::HashMap<InstanceId, usize> = order.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    let mut best: Option<(u32, usize, f32)> = None;
    for (k, id) in lists::billboards(cx, order) {
        if !nodes.iter().any(|n| n.key == k) || !lists::drawn(cx, id) {
            continue;
        }
        let Some(&r) = rank.get(&id) else { continue };
        let b = crate::display::hit::drawn_bounds(cx.player, cx.lib, id).transform(&cx.player.world_matrix(id));
        if b.is_empty() {
            continue;
        }
        let (x, y) = ((b.x0 + b.x1) / 40.0, (b.y0 + b.y1) / 40.0);
        let d = ((x - at.0).powi(2) + (y - at.1).powi(2)).sqrt();
        if d <= reach && best.is_none_or(|(_, _, bd)| d < bd) {
            best = Some((k, r, d));
        }
    }
    best.map(|(k, r, _)| (k, r))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_focus_wait_is_bounded() {
        let mut s = Step::<i32>::default();
        assert_eq!(s.next(0, MAX_FOCUS_STEPS, 2), Next::Step);
        s.made(0);
        for _ in 0..STEP_WAIT {
            assert_eq!(s.next(0, MAX_FOCUS_STEPS, 2), Next::Wait);
        }
        assert_eq!(s.next(0, MAX_FOCUS_STEPS, 2), Next::Step, "the other way");
        assert_eq!(s.tried, 1);
        s.made(0);
        for _ in 0..STEP_WAIT {
            assert_eq!(s.next(0, MAX_FOCUS_STEPS, 2), Next::Wait);
        }
        assert_eq!(s.next(0, MAX_FOCUS_STEPS, 2), Next::Stalled, "no way moved it");
        let mut s = Step::<i32>::default();
        for at in 0..MAX_FOCUS_STEPS as i32 {
            assert_eq!(s.next(at, MAX_FOCUS_STEPS, 1), Next::Step);
            s.made(at);
        }
        assert_eq!(s.next(MAX_FOCUS_STEPS as i32, MAX_FOCUS_STEPS, 1), Next::Stalled, "the steps' cap");
    }

    #[test]
    fn the_log_is_bounded() {
        let mut r = PointerRouter::default();
        for i in 0..MAX_LOG_LINES + 44 {
            r.note(format!("line {i}"));
        }
        assert_eq!(r.log.len(), MAX_LOG_LINES);
        assert_eq!(r.log[0], "line 44");
        r.note("x".repeat(100_000));
        let last = r.log.last().expect("a line");
        assert!(last.ends_with("… (100000 bytes)"), "{last}");
        assert_eq!(last.chars().count(), MAX_LOG_CHARS + "… (100000 bytes)".chars().count());
        r.note("a\nb\u{1b}[2J".into());
        assert_eq!(r.log.last().map(String::as_str), Some("a\\nb\\u{1b}[2J"));
    }
}
