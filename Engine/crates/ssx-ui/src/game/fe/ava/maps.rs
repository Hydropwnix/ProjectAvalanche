use std::collections::VecDeque;

use super::bounded;
use crate::avm1::Ctx;
use crate::avm1::value::Value;
use crate::game::fe::{ActionParam, FeEnv, FrontEnd};
use crate::game::loc::Localization;

pub mod session;

pub const DP: &str = "dp_ava_maps";
pub const DP_ROWS: &str = "dp_ava_maps_rows";
pub const DP_ROWS_INDEX: &str = "dp_ava_maps_rows_index";
pub const DP_DETAILS: &str = "dp_ava_maps_details";
pub const DP_WINDOW: &str = "dp_ava_maps_window";
pub const OPEN: &str = "go_to_ava_maps";
pub const MESSAGE: &str = "ava_maps_message";
pub const MESSAGE_OK: &str = "do_ava_maps_message_ok";

pub const MAX_TITLE: usize = 64;
pub const MAX_PACKAGE: usize = 64;
pub const MAX_REASON: usize = 250;
const MAX_MESSAGES: usize = 8;
const MAX_KEPT_REASON: usize = 8192;

#[derive(Clone, Debug, PartialEq)]
pub struct MapRow {
    pub id: String,
    pub package: String,
    pub entry: Result<MapInfo, String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MapInfo {
    pub title: String,
    pub modes: Vec<ModeInfo>,
    pub base: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModeInfo {
    pub choice: String,
    pub title: Option<String>,
    pub event_type: i64,
    pub targets: Vec<u32>,
    pub time_limit_ms: Option<u32>,
    pub refused: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MapRequest {
    List,
}

pub fn event_type(mode: &str) -> Option<i64> {
    ssx_content::map::Mode::parse(mode).map(|m| m.event_type() as i64)
}

pub fn ride(fe: &mut FrontEnd, env: &mut FeEnv, id: &str, mode: Option<&str>) -> Result<(), String> {
    let row = fe.ava.maps.rows.iter().find(|r| r.id == id).ok_or_else(|| format!("{id}: no such map is installed"))?;
    let info = row.entry.as_ref().map_err(Clone::clone)?;
    let m = match mode {
        Some(name) => info.modes.iter().find(|m| m.choice == name),
        None => info.modes.first(),
    };
    let m = m.ok_or_else(|| match mode {
        Some(name) => format!("{id}: no mode {}", ssx_core::text::quoted(name)),
        None => format!("{id}: no mode at all"),
    })?;
    if let Some(why) = &m.refused {
        return Err(format!("{id} in {}: {why}", m.choice));
    }
    let (mode, event_type) = (m.choice.clone(), m.event_type);
    let track = match &info.base {
        None => String::new(),
        Some(drop) => {
            let e =
                fe.career.event_named(drop).ok_or_else(|| format!("{id}: its base drop {drop} is not on the disc"))?;
            fe.career.drop(e.drop).map(|d| d.track.clone()).ok_or_else(|| format!("{id}: {drop} has no track"))?
        }
    };
    let map = crate::game::fe::MapLaunch { id: id.to_string(), title: info.title.clone(), track, mode: mode.clone() };
    fe.note(format!("maps: ride {id} ({mode}: event type {event_type})"));
    crate::game::fe::flow::launch_map(fe, env, map, event_type);
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    pub first: usize,
    pub focus: usize,
    pub rows: usize,
    pub band: usize,
}

impl Default for Window {
    fn default() -> Self {
        Window { first: 0, focus: 0, rows: 9, band: 8 }
    }
}

impl Window {
    pub fn selected(&self) -> usize {
        self.first + self.focus
    }

    pub fn step(&mut self, d: i64, len: usize) -> bool {
        let before = *self;
        let by = d.unsigned_abs() as usize;
        if d < 1 {
            if self.first > 0 {
                self.first = self.first.saturating_sub(by);
            } else {
                self.focus = self.focus.saturating_sub(by);
            }
        } else if self.band <= self.focus {
            self.first = self.first.saturating_add(by);
        } else {
            self.focus = self.focus.saturating_add(by);
        }
        self.clamp(len);
        *self != before
    }

    pub fn page(&mut self, d: i64, len: usize) -> bool {
        let before = *self;
        let by = self.rows.max(1);
        self.first = if d < 0 { self.first.saturating_sub(by) } else { self.first.saturating_add(by) };
        self.clamp(len);
        *self != before
    }

    pub fn clamp(&mut self, len: usize) {
        if len == 0 {
            (self.first, self.focus) = (0, 0);
            return;
        }
        self.focus = self.focus.min(self.rows.saturating_sub(1)).min(len - 1);
        self.first = self.first.min(len.saturating_sub(self.rows)).min(len - 1 - self.focus);
    }
}

#[derive(Clone, Debug, Default)]
pub struct Maps {
    pub active: bool,
    pub rows: Vec<MapRow>,
    pub more: usize,
    pub listed: bool,
    pub window: Window,
    pub requests: Vec<MapRequest>,
    pub messages: VecDeque<String>,
    pub message_up: bool,
    pub riding: Option<String>,
    pub mode: usize,
    pub return_due: bool,
}

pub fn activate(fe: &mut FrontEnd, env: &mut FeEnv) {
    fe.game_mode = -1;
    fe.screen = crate::game::fe::screens::id::MAIN_MENU;
    env.nav_events.push("backgroundcontainer_disable".into());
    env.values.set_str(0x32, env.loc.text("TXT_AVA_HUB_TITLE"));
    env.values.set_str(0x33, env.loc.text("TXT_AVA_MAPS_TITLE"));
    let m = &mut fe.ava.maps;
    m.active = true;
    m.listed = false;
    m.requests.push(MapRequest::List);
    fe.note("maps: open, the list asked of the host");
    fe.publish.extend([DP.into(), DP_ROWS.into(), DP_ROWS_INDEX.into(), DP_DETAILS.into(), DP_WINDOW.into()]);
}

pub fn deactivate(fe: &mut FrontEnd) {
    fe.ava.maps.active = false;
}

pub fn listed(fe: &mut FrontEnd, rows: Vec<MapRow>, more: usize) {
    let keep = fe.ava.maps.rows.get(fe.ava.maps.window.selected()).map(|r| r.id.clone());
    let m = &mut fe.ava.maps;
    m.rows = rows;
    m.more = more;
    m.listed = true;
    if let Some(at) = keep.and_then(|id| m.rows.iter().position(|r| r.id == id)) {
        let w = &mut m.window;
        w.first = at.saturating_sub(w.focus).min(m.rows.len().saturating_sub(w.rows));
        w.focus = at - w.first;
    }
    m.window.clamp(m.rows.len());
    let n = m.rows.get(m.window.selected()).and_then(|r| r.entry.as_ref().ok()).map_or(0, |i| i.modes.len());
    if m.mode >= n {
        m.mode = 0;
    }
    fe.note(format!("maps: {} listed ({} more not)", fe.ava.maps.rows.len(), more));
    fe.publish.extend([DP.into(), DP_ROWS.into(), DP_ROWS_INDEX.into(), DP_DETAILS.into(), DP_WINDOW.into()]);
}

pub fn refused(fe: &mut FrontEnd, reason: &str) {
    let reason: String = reason.chars().take(MAX_KEPT_REASON).collect();
    fe.note(format!("maps: refused: {}", ssx_core::text::Text::new(&reason)));
    if fe.ava.maps.messages.len() < MAX_MESSAGES {
        fe.ava.maps.messages.push_back(reason);
    }
}

pub fn update(fe: &mut FrontEnd, env: &mut FeEnv) {
    if !fe.ava.maps.active || fe.ava.maps.message_up {
        return;
    }
    let Some(text) = fe.ava.maps.messages.pop_front() else { return };
    let glyphs = fe.ava.glyphs.clone();
    let body = format!("{}\n\n{}", env.loc.text("TXT_AVA_MAPS_REFUSED"), bounded(&text, MAX_REASON, glyphs.as_ref()));
    env.values.set_str(crate::game::fe::popup::POPUP_BODY, body);
    env.nav_events.push(MESSAGE.into());
    fe.ava.maps.message_up = true;
}

pub fn action(fe: &mut FrontEnd, env: &mut FeEnv, action: &str, param: &ActionParam) {
    let len = fe.ava.maps.rows.len();
    let w = &mut fe.ava.maps.window;
    let moved = match action {
        "A_AvaMapsSetup" => {
            let (rows, band) = (param.field("maxData"), param.field("unselectableData"));
            if rows.is_finite() && (1.0..=32.0).contains(&rows) && band.is_finite() && band >= 0.0 {
                w.rows = rows as usize;
                w.band = (band as usize).min(w.rows - 1);
            }
            w.clamp(len);
            true
        }
        "A_AvaMapsRequestData" if param.number.is_finite() => w.step(param.number.clamp(-64.0, 64.0) as i64, len),
        "A_AvaMapsPage" if param.number.is_finite() => w.page(param.number.signum() as i64, len),
        "A_AvaMapsRide" => {
            let m = &fe.ava.maps;
            let row = m.rows.get(m.window.selected());
            let mode =
                row.and_then(|r| r.entry.as_ref().ok()).and_then(|i| i.modes.get(m.mode)).map(|x| x.choice.clone());
            if let Some(id) = row.map(|r| r.id.clone())
                && let Err(why) = ride(fe, env, &id, mode.as_deref())
            {
                refused(fe, &why);
            }
            false
        }
        "A_AvaMapsMode" if param.number.is_finite() => {
            let m = &mut fe.ava.maps;
            let n = m.rows.get(m.window.selected()).and_then(|r| r.entry.as_ref().ok()).map_or(0, |i| i.modes.len());
            if n > 1 {
                m.mode = if param.number < 0.0 { (m.mode + n - 1) % n } else { (m.mode + 1) % n };
                fe.publish.push(DP_DETAILS.into());
            }
            false
        }
        "A_AvaMapsRefresh" => {
            fe.ava.maps.requests.push(MapRequest::List);
            false
        }
        "A_AvaMapsBack" => {
            env.nav_events.push(super::hub::OPEN.into());
            false
        }
        _ => false,
    };
    if moved {
        fe.ava.maps.mode = 0;
        fe.publish.extend([DP_ROWS.into(), DP_ROWS_INDEX.into(), DP_DETAILS.into(), DP_WINDOW.into()]);
    }
}

pub fn nav_action(fe: &mut FrontEnd) {
    fe.ava.maps.message_up = false;
}

fn base_title(fe: &FrontEnd, loc: &mut Localization, base: Option<&str>) -> String {
    let Some(drop) = base else { return loc.text("TXT_AVA_MAPS_NEW_TRACK") };
    let Some(e) = fe.career.event_named(drop) else { return drop.to_string() };
    let title = fe.career.drop(e.drop).map(|d| loc.text(&d.title)).unwrap_or_default();
    let (peak, _) = fe.career.peak_and_range(e);
    let peak = fe.career.peak(peak).map(|p| loc.text(&p.title)).unwrap_or_default();
    match (peak.is_empty(), title.is_empty()) {
        (false, false) => format!("{peak} \u{2022} {title}"),
        (true, false) => title,
        (false, true) => peak,
        (true, true) => drop.to_string(),
    }
}

fn mode_name(loc: &mut Localization, m: &ModeInfo, glyphs: Option<&std::collections::BTreeSet<char>>) -> String {
    if let Some(t) = &m.title {
        return bounded(t, MAX_TITLE, glyphs);
    }
    match m.choice.as_str() {
        "race" => loc.text("TXT_AVA_BROWSER_MODE_RACE"),
        "trick" => loc.text("TXT_AVA_BROWSER_MODE_TRICK"),
        "survive" => loc.text("TXT_AVA_BROWSER_MODE_SURVIVE"),
        "death" => loc.text("TXT_AVA_MAPS_MODE_DEATH"),
        "freeride" => loc.text("TXT_AVA_BROWSER_MODE_FREERIDE"),
        other => bounded(other, MAX_TITLE, glyphs),
    }
}

fn mode_names(loc: &mut Localization, modes: &[ModeInfo], glyphs: Option<&std::collections::BTreeSet<char>>) -> String {
    let mut names: Vec<String> =
        modes.iter().filter(|m| m.title.is_none()).map(|m| mode_name(loc, m, glyphs)).collect();
    match modes.iter().filter(|m| m.title.is_some()).count() {
        0 => {}
        1 => names.push(loc.text("TXT_AVA_MAPS_GAMEMODE_ONE")),
        n => names.push(loc.format("TXT_AVA_MAPS_GAMEMODES", &[&n.to_string()])),
    }
    names.join(", ")
}

fn measure_text(loc: &mut Localization, t: i64, v: u32) -> String {
    match t {
        5..=7 => loc.race_time(v),
        3 | 4 | 8 => loc.distance_with_unit(f64::from(v) / 1000.0),
        _ => loc.int(i64::from(v)),
    }
}

fn mode_details(fe: &FrontEnd, loc: &mut Localization, map: &str, modes: &[ModeInfo]) -> (String, Vec<String>) {
    let glyphs = fe.ava.glyphs.as_ref();
    let Some(m) = modes.get(fe.ava.maps.mode.min(modes.len().saturating_sub(1))) else {
        return (String::new(), Vec::new());
    };
    let head = mode_head(loc, m, fe.ava.maps.mode.min(modes.len() - 1) + 1, modes.len(), glyphs);
    let mut about = Vec::new();
    if let [gold, silver, bronze] = m.targets[..] {
        let [g, s, b] = [gold, silver, bronze].map(|v| measure_text(loc, m.event_type, v));
        about.push(loc.format("TXT_AVA_MAPS_TARGETS", &[&g, &s, &b]));
    }
    if let Some(ms) = m.time_limit_ms {
        let time = loc.race_time(ms);
        about.push(loc.format("TXT_AVA_MAPS_TIME_LIMIT", &[&time]));
    }
    let record =
        map.parse().ok().and_then(|id| ssx_content::gamemode::record::read(&fe.profile.mod_state, &id, &m.choice));
    if let Some(r) = record {
        let best = match r.best {
            Some(v) => measure_text(loc, m.event_type, v),
            None => loc.text("TXT_N_A"),
        };
        about.push(match r.attempts {
            1 => loc.format("TXT_AVA_MAPS_BEST_ONE", &[&best]),
            n => loc.format("TXT_AVA_MAPS_BEST", &[&best, &n.to_string()]),
        });
    }
    if let Some(why) = &m.refused {
        about.push(format!("{}\n{}", loc.text("TXT_AVA_MAPS_MODE_REFUSED"), bounded(why, MAX_REASON, glyphs)));
    }
    (head, about)
}

fn mode_head(
    loc: &mut Localization,
    m: &ModeInfo,
    k: usize,
    n: usize,
    glyphs: Option<&std::collections::BTreeSet<char>>,
) -> String {
    let name = mode_name(loc, m, glyphs);
    let kind = usize::try_from(m.event_type)
        .ok()
        .and_then(|t| crate::game::hud::results::EVENT_TYPE_NAMES.get(t))
        .map_or(String::new(), |key| loc.text(key));
    let head = loc.format("TXT_AVA_MAPS_MODE", &[&k.to_string(), &n.to_string(), &name]);
    format!("{}<br>{kind}", crate::author::escape(&head))
}

fn set(cx: &mut Ctx, o: &Value, k: &str, v: Value) {
    let _ = cx.set_member(o, k, v);
}

pub fn build(fe: &FrontEnd, cx: &mut Ctx, loc: &mut Localization, dp: &str) -> Option<Value> {
    let m = &fe.ava.maps;
    let glyphs = fe.ava.glyphs.as_ref();
    match dp {
        DP => {
            let o = Value::Object(cx.new_object());
            for (k, key) in [
                ("colTitle", "TXT_AVA_MAPS_COL_TITLE"),
                ("colModes", "TXT_AVA_MAPS_COL_MODES"),
                ("colBase", "TXT_AVA_MAPS_COL_BASE"),
            ] {
                set(cx, &o, k, Value::str(&loc.text(key)));
            }
            let empty = match (m.listed, m.rows.is_empty()) {
                (false, _) => loc.text("TXT_AVA_MAPS_LISTING"),
                (true, true) => loc.text("TXT_AVA_MAPS_EMPTY"),
                (true, false) => String::new(),
            };
            set(cx, &o, "empty", Value::str(&empty));
            let count = loc.format("TXT_AVA_MAPS_COUNT", &[&m.rows.len().to_string()]);
            let status = match m.more {
                0 => count,
                n => format!("{count}  {}", loc.format("TXT_AVA_MAPS_MORE", &[&n.to_string()])),
            };
            set(cx, &o, "status", Value::str(&status));
            Some(o)
        }
        DP_ROWS_INDEX => Some(Value::Number(m.window.focus as f64)),
        DP_WINDOW => Some(super::window_object(cx, m.rows.len(), m.window.first)),
        DP_ROWS => {
            let w = m.window;
            let mut out = Vec::new();
            for (at, r) in m.rows.iter().enumerate().skip(w.first).take(w.rows) {
                let o = Value::Object(cx.new_object());
                let (title, modes, base, enabled) = match &r.entry {
                    Ok(i) => (
                        bounded(&i.title, MAX_TITLE, glyphs),
                        mode_names(loc, &i.modes, glyphs),
                        base_title(fe, loc, i.base.as_deref()),
                        true,
                    ),
                    Err(_) => {
                        (bounded(&r.id, MAX_TITLE, glyphs), loc.text("TXT_AVA_MAPS_BROKEN"), String::new(), false)
                    }
                };
                set(cx, &o, "strTitle", Value::str(&title));
                set(cx, &o, "strModes", Value::str(&modes));
                set(cx, &o, "strBase", Value::str(&base));
                set(cx, &o, "bEnabled", Value::Bool(enabled));
                set(cx, &o, "bFocusable", Value::Bool(true));
                set(cx, &o, "eventAdvance", Value::str("A_AvaMapsRide"));
                set(cx, &o, "eventAdvanceParam", Value::Number(at as f64));
                out.push(o);
            }
            Some(Value::Object(cx.new_array(out)))
        }
        DP_DETAILS => {
            let o = Value::Object(cx.new_object());
            let r = m.rows.get(m.window.selected());
            set(cx, &o, "bShown", Value::Bool(r.is_some()));
            let Some(r) = r else { return Some(o) };
            let package = loc.format("TXT_AVA_MAPS_FROM", &[&bounded(&r.package, MAX_PACKAGE, glyphs)]);
            set(cx, &o, "package", Value::str(&package));
            match &r.entry {
                Ok(i) => {
                    set(cx, &o, "title", Value::str(&bounded(&i.title, MAX_TITLE, glyphs)));
                    let (modes, mut about) = mode_details(fe, loc, &r.id, &i.modes);
                    set(cx, &o, "modes", Value::str(&modes));
                    let base = match &i.base {
                        Some(_) => {
                            let title = base_title(fe, loc, i.base.as_deref());
                            loc.format("TXT_AVA_MAPS_LAYOUT", &[&title])
                        }
                        None => loc.text("TXT_AVA_MAPS_NEW_TRACK"),
                    };
                    set(cx, &o, "base", Value::str(&base));
                    if about.is_empty() {
                        about.push(String::new());
                    }
                    about.truncate(4);
                    set(cx, &o, "about", Value::str(&about.join("\n")));
                }
                Err(why) => {
                    set(cx, &o, "title", Value::str(&bounded(&r.id, MAX_TITLE, glyphs)));
                    set(cx, &o, "modes", Value::str(&crate::author::escape(&loc.text("TXT_AVA_MAPS_BROKEN"))));
                    set(cx, &o, "base", Value::str(""));
                    set(cx, &o, "about", Value::str(&bounded(why, MAX_REASON, glyphs)));
                }
            }
            Some(o)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostile_text_is_bounded_and_shown_with_the_fonts_glyphs() {
        let wrap = crate::text::isolate::wrap;
        let long = "x".repeat(10_000);
        assert_eq!(bounded(&long, MAX_TITLE, None), wrap(&format!("{}...", "x".repeat(MAX_TITLE - 3))));
        assert_eq!(bounded(&"x".repeat(MAX_TITLE), MAX_TITLE, None), wrap(&"x".repeat(MAX_TITLE)));
        assert_eq!(bounded("<b>Map</b>&\u{7}", 64, None), wrap("?b?Map?/b???"));
        assert_eq!(bounded("short", 64, None), wrap("short"));
    }

    #[test]
    fn a_maps_modes_are_the_games_event_types() {
        use crate::game::hud::results::EVENT_TYPE_NAMES;
        let name =
            |m| event_type(m).and_then(|t| usize::try_from(t).ok()).and_then(|t| EVENT_TYPE_NAMES.get(t).copied());
        assert_eq!(name("race"), Some("TXT_EVENTTYPE_RACE_CLOCK"));
        assert_eq!(name("trick"), Some("TXT_EVENTTYPE_TRICK_SCORE"));
        assert_eq!(name("freeride"), Some("TXT_EVENTTYPE_FREERIDE"));
        assert_eq!(name("survive"), Some("TXT_EVENTTYPE_SURVIVE_TO_BOTTOM"));
        assert_eq!(name("death"), Some("TXT_EVENTTYPE_SURVIVE_TO_DEATH"));
        assert_eq!(event_type("golf"), None);
    }

    #[test]
    fn a_row_counts_its_gamemodes_and_a_title_is_escaped_in_the_details() {
        let mut loc = Localization::empty(crate::game::loc::Language::English);
        for (k, t) in crate::game::port::builtin_strings().unwrap() {
            loc.add_port_string(&k, &t).unwrap();
        }
        let mode = |choice: &str, title: Option<&str>| ModeInfo {
            choice: choice.into(),
            title: title.map(String::from),
            event_type: 5,
            targets: Vec::new(),
            time_limit_ms: None,
            refused: None,
        };
        let built_in = [mode("race", None), mode("freeride", None)];
        assert_eq!(mode_names(&mut loc, &built_in, None), "Race, Freeride");
        let mut modes = built_in.to_vec();
        modes.push(mode("p:gamemode/a", Some("A")));
        assert_eq!(mode_names(&mut loc, &modes, None), "Race, Freeride, 1 gamemode");
        modes.extend((0..9).map(|k| mode(&format!("p:gamemode/g{k}"), Some("G"))));
        assert_eq!(mode_names(&mut loc, &modes, None), "Race, Freeride, 10 gamemodes");
        assert_eq!(mode_names(&mut loc, &modes[2..], None), "10 gamemodes");
        let head = mode_head(&mut loc, &mode("p:gamemode/x", Some("A & <b>B</b>")), 2, 12, None);
        let title = crate::text::isolate::wrap("A ? ?b?B?/b?");
        assert!(head.starts_with(&format!("Mode 2 of 12: {title}<br>")), "{head}");
    }

    #[test]
    fn the_window_pages_within_the_rows() {
        let mut w = Window::default();
        assert!(!w.step(-1, 0));
        for _ in 0..20 {
            w.step(1, 12);
        }
        assert_eq!((w.first, w.focus, w.selected()), (3, 8, 11));
        w.page(-1, 12);
        assert_eq!(w.first, 0);
        w.page(1, 12);
        assert_eq!(w.first, 3);
        w.clamp(2);
        assert_eq!((w.first, w.focus), (0, 1));
    }
}
