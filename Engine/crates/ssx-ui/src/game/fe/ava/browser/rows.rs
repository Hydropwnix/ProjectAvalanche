use ssx_net_join::browse::{Entry, SortKey, State, ViewParams, view};

use super::sources::MasterState;
use super::{Browser, DP, DP_DETAILS, DP_DROP, DP_ROWS, DP_ROWS_INDEX, TABS};
use crate::avm1::Ctx;
use crate::avm1::value::Value;
use crate::game::fe::{ActionParam, FeEnv, FrontEnd};
use crate::game::loc::Localization;

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
        if d < 1 {
            if self.first > 0 {
                self.first = self.first.saturating_sub(d.unsigned_abs() as usize);
            } else {
                self.focus = self.focus.saturating_sub(d.unsigned_abs() as usize);
            }
        } else if self.band <= self.focus {
            self.first += d as usize;
        } else {
            self.focus += d as usize;
        }
        self.clamp(len);
        *self != before
    }

    pub fn page(&mut self, d: i64, len: usize) -> bool {
        let before = *self;
        let by = self.rows.max(1);
        self.first = if d < 0 { self.first.saturating_sub(by) } else { self.first + by };
        self.clamp(len);
        *self != before
    }

    pub fn clamp(&mut self, len: usize) {
        if len == 0 {
            self.first = 0;
            self.focus = 0;
            return;
        }
        self.focus = self.focus.min(self.rows.saturating_sub(1)).min(len - 1);
        self.first = self.first.min(len.saturating_sub(self.rows)).min(len - 1 - self.focus);
    }
}

pub fn selected(b: &Browser) -> Option<&Entry> {
    b.view.get(b.window.selected()).and_then(|&i| b.entries.get(i))
}

pub fn drop_title(fe: &FrontEnd, loc: &mut Localization, drop: &str) -> String {
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

pub fn rebuild(fe: &mut FrontEnd, env: &mut FeEnv) {
    let keep = selected(&fe.ava.browser).map(|e| e.address.clone());
    let titles: std::collections::HashMap<String, String> = fe
        .ava
        .browser
        .entries
        .iter()
        .filter_map(|e| e.summary().filter(|s| s.map.is_none()).map(|s| s.drop))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .map(|d| {
            let t = drop_title(fe, env.loc, &d);
            (d, t)
        })
        .collect();
    let title = |d: &str| titles.get(d).cloned().unwrap_or_else(|| d.to_string());
    let b = &mut fe.ava.browser;
    let p = ViewParams { filters: &b.filters, search: &b.terms, drop_title: &title };
    b.view = view(&b.entries, &p);
    if std::mem::take(&mut b.restart) {
        b.window.first = 0;
        b.window.focus = 0;
    } else if let Some(at) = keep.and_then(|a| b.view.iter().position(|&i| b.entries[i].address == a)) {
        let w = &mut b.window;
        w.first = at.saturating_sub(w.focus).min(b.view.len().saturating_sub(w.rows));
        w.focus = at - w.first;
    }
    b.window.clamp(b.view.len());
    b.dirty = false;
    super::details::refresh(fe, env);
    fe.publish.extend([DP.into(), DP_ROWS.into(), DP_ROWS_INDEX.into(), DP_DETAILS.into(), DP_DROP.into()]);
}

pub fn action(fe: &mut FrontEnd, env: &mut FeEnv, action: &str, param: &ActionParam) {
    let len = fe.ava.browser.view.len();
    let w = &mut fe.ava.browser.window;
    let moved = match action {
        "A_AvaBrowserSetup" => {
            let (rows, band) = (param.field("maxData"), param.field("unselectableData"));
            if rows.is_finite() && (1.0..=32.0).contains(&rows) && band.is_finite() && band >= 0.0 {
                w.rows = rows as usize;
                w.band = (band as usize).min(w.rows - 1);
            }
            w.clamp(len);
            true
        }
        "A_AvaBrowserRequestData" if param.number.is_finite() => w.step(param.number.clamp(-64.0, 64.0) as i64, len),
        "A_AvaBrowserPage" if param.number.is_finite() => w.page(param.number.signum() as i64, len),
        _ => false,
    };
    if moved {
        super::details::refresh(fe, env);
        fe.publish.extend([DP_ROWS.into(), DP_ROWS_INDEX.into(), DP_DETAILS.into(), DP_DROP.into()]);
    }
}

const LOCK_ICON: &str = "assets\\drop_selector_icons\\drop_locked.dds";

fn mode_icon(mode: &str) -> &'static str {
    match mode {
        "race" => "assets\\drop_selector_icons\\race_icon.dds",
        "trick" => "assets\\drop_selector_icons\\trick_icon.dds",
        "survive" => "assets\\drop_selector_icons\\survive_icon.dds",
        _ => "",
    }
}

pub fn mode_name(loc: &mut Localization, mode: &str) -> String {
    match mode {
        "freeride" => loc.text("TXT_AVA_BROWSER_MODE_FREERIDE"),
        "race" => loc.text("TXT_AVA_BROWSER_MODE_RACE"),
        "trick" => loc.text("TXT_AVA_BROWSER_MODE_TRICK"),
        "survive" => loc.text("TXT_AVA_BROWSER_MODE_SURVIVE"),
        "death" => loc.text("TXT_AVA_BROWSER_MODE_DEATH"),
        other => other.to_string(),
    }
}

pub fn megabytes(bytes: u64) -> String {
    bytes.div_ceil(1 << 20).to_string()
}

pub fn download_cell(loc: &mut Localization, e: &Entry, r: &ssx_net_proto::resources::ResourceSummary) -> String {
    use ssx_net_join::resources::Need;
    if r.count == 0 {
        return String::new();
    }
    match e.need {
        Some(Need::Exactly(0)) => loc.text("TXT_AVA_BROWSER_DOWNLOAD_KEPT"),
        Some(Need::Exactly(n)) => loc.format("TXT_AVA_BROWSER_DOWNLOAD_EXACT", &[&megabytes(n)]),
        Some(Need::Between { most, .. }) => loc.format("TXT_AVA_BROWSER_DOWNLOAD_AT_MOST", &[&megabytes(most)]),
        None => loc.format("TXT_AVA_BROWSER_DOWNLOAD_AT_MOST", &[&megabytes(r.download)]),
    }
}

pub fn map_mode_name(loc: &mut Localization, map: &ssx_net_proto::instance::MapSummary) -> String {
    match (&map.gamemode, ssx_net_proto::instance::built_in_mode(map.event_type)) {
        (Some(g), _) => g.title.clone(),
        (None, Some(m)) => mode_name(loc, m),
        (None, None) => map.event_type.to_string(),
    }
}

fn row(fe: &FrontEnd, cx: &mut Ctx, loc: &mut Localization, e: &Entry, at: usize) -> Value {
    let o = Value::Object(cx.new_object());
    let set = |cx: &mut Ctx, k: &str, v: Value| {
        let _ = cx.set_member(&o, k, v);
    };
    let (mode, icon, drop, players, enabled) = match (&e.state, e.summary()) {
        (State::Incompatible { .. }, _) => {
            (loc.text("TXT_AVA_BROWSER_INCOMPATIBLE"), "", String::new(), String::new(), false)
        }
        (_, Some(s)) if s.map.is_some() => {
            let mode = s.map.as_ref().map(|m| map_mode_name(loc, m)).unwrap_or_default();
            let title = s.level_title(&|d| d.to_string());
            (mode, "", title, format!("{}/{}", s.players, s.max_players), true)
        }
        (_, Some(s)) => {
            let icon = mode_icon(&s.mode);
            let mode = if icon.is_empty() { mode_name(loc, &s.mode) } else { String::new() };
            let on_disc = fe.career.event_named(&s.drop).is_some();
            (mode, icon, drop_title(fe, loc, &s.drop), format!("{}/{}", s.players, s.max_players), on_disc)
        }
        (State::Failed(_), None) => (loc.text("TXT_AVA_BROWSER_NO_ANSWER"), "", String::new(), String::new(), true),
        (_, None) => (loc.text("TXT_AVA_BROWSER_ASKING"), "", String::new(), String::new(), true),
    };
    let download = e.summary().map(|s| download_cell(loc, e, &s.resources)).unwrap_or_default();
    let enabled = enabled && super::resources_refusal(fe, loc, e).is_none();
    let lock = e.summary().is_some_and(|s| s.password);
    let shown = |t: &str| crate::game::fe::ava::shown(t, fe.ava.glyphs.as_ref());
    set(cx, "strName", Value::str(&shown(&e.name())));
    set(cx, "strMode", Value::str(&shown(&mode)));
    set(cx, "strModeIcon", Value::str(icon));
    set(cx, "strLock", Value::str(if lock { LOCK_ICON } else { "" }));
    set(cx, "strDrop", Value::str(&shown(&drop)));
    set(cx, "strPlayers", Value::str(&players));
    set(cx, "strDownload", Value::str(&download));
    let ping = e.ping.map_or_else(String::new, |d| d.as_millis().min(9999).to_string());
    set(cx, "strPing", Value::str(&ping));
    set(cx, "bEnabled", Value::Bool(enabled));
    set(cx, "bFocusable", Value::Bool(true));
    set(cx, "eventAdvance", Value::str("A_AvaBrowserConnect"));
    set(cx, "eventAdvanceParam", Value::Number(at as f64));
    o
}

pub fn build(fe: &FrontEnd, cx: &mut Ctx, loc: &mut Localization, dp: &str) -> Value {
    let b = &fe.ava.browser;
    if dp == DP_ROWS_INDEX {
        return Value::Number(b.window.focus as f64);
    }
    let w = b.window;
    let rows: Vec<Value> = b
        .view
        .iter()
        .enumerate()
        .skip(w.first)
        .take(w.rows)
        .map(|(at, &i)| row(fe, cx, loc, &b.entries[i], at))
        .collect();
    Value::Object(cx.new_array(rows))
}

const TAB_GAP: &str = "    ";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabPart {
    Bumper(crate::game::input::Button),
    Tab(super::Tab),
}

pub fn tab_parts(loc: &mut Localization) -> Vec<(TabPart, std::ops::Range<u32>)> {
    use crate::game::input::Button;
    let mut names: Vec<(TabPart, String)> = vec![(TabPart::Bumper(Button::LeftBumper), "LB".into())];
    names.extend(TABS.iter().map(|(t, key)| (TabPart::Tab(*t), loc.text(key))));
    names.push((TabPart::Bumper(Button::RightBumper), "RB".into()));
    let gap = TAB_GAP.chars().count() as u32;
    let mut at = 0u32;
    let mut out = Vec::new();
    for (part, name) in names {
        let n = name.chars().count().min(u32::MAX as usize) as u32;
        out.push((part, at..at.saturating_add(n)));
        at = at.saturating_add(n).saturating_add(gap);
    }
    out
}

pub fn header(fe: &FrontEnd, cx: &mut Ctx, loc: &mut Localization) -> Value {
    let b = &fe.ava.browser;
    let o = Value::Object(cx.new_object());
    let mut tabs = format!("<font color=\"#5F8C99\">LB</font>{TAB_GAP}");
    for (tab, key) in TABS {
        let colour = if tab == b.tab { "#FFFFFF" } else { "#5F8C99" };
        tabs.push_str(&format!("<font color=\"{colour}\">{}</font>{TAB_GAP}", crate::author::escape(&loc.text(key))));
    }
    tabs.push_str("<font color=\"#5F8C99\">RB</font>");
    let sort = match b.filters.sort {
        SortKey::Name => "TXT_AVA_BROWSER_COL_NAME",
        SortKey::Players => "TXT_AVA_BROWSER_COL_PLAYERS",
        SortKey::Ping => "TXT_AVA_BROWSER_COL_PING",
        SortKey::Mode => "TXT_AVA_BROWSER_COL_MODE",
        SortKey::Drop => "TXT_AVA_BROWSER_COL_DROP",
    };
    let order = loc.text(if b.filters.descending { "TXT_AVA_BROWSER_DESCENDING" } else { "TXT_AVA_BROWSER_ASCENDING" });
    let sort = loc.text(sort);
    let mut status =
        loc.format("TXT_AVA_BROWSER_COUNT", &[&b.view.len().to_string(), &b.entries.len().to_string(), &sort, &order]);
    if !b.search.is_empty() {
        status.push_str(&loc.format("TXT_AVA_BROWSER_SEARCHING", &[&b.search]));
    }
    if b.fake.is_some() {
        status.push_str(&loc.text("TXT_AVA_BROWSER_FAKE"));
    }
    let master = super::sources::master_state(b);
    match &master {
        Some(MasterState::Waiting) => status.push_str(&loc.text("TXT_AVA_BROWSER_MASTER_WAITING")),
        Some(MasterState::Failed(why)) => status.push_str(&loc.format("TXT_AVA_BROWSER_MASTER_FAILED", &[why])),
        Some(MasterState::Answered) | None => {}
    }
    if master.is_some() && b.left_out > 0 {
        status.push_str(&loc.format("TXT_AVA_BROWSER_LEFT_OUT", &[&b.left_out.to_string()]));
    }
    let empty = match (b.entries.is_empty(), b.tab, &b.source_problem) {
        (false, _, _) => String::new(),
        (true, super::Tab::Internet | super::Tab::Lan, Some(why)) => {
            loc.format("TXT_AVA_BROWSER_SOURCE_PROBLEM", &[why])
        }
        (true, super::Tab::Internet, None) => match master {
            None => loc.text("TXT_AVA_BROWSER_NO_MASTER"),
            Some(MasterState::Waiting) => loc.text("TXT_AVA_BROWSER_ASKING_MASTER"),
            Some(_) => loc.text("TXT_AVA_BROWSER_EMPTY_INTERNET"),
        },
        (true, super::Tab::Lan, None) => loc.text("TXT_AVA_BROWSER_EMPTY_LAN"),
        (true, super::Tab::Favourites, _) => loc.text("TXT_AVA_BROWSER_EMPTY_FAVOURITES"),
        (true, super::Tab::Recent, _) => loc.text("TXT_AVA_BROWSER_EMPTY_RECENT"),
    };
    for (k, v) in [
        ("tabs", tabs),
        ("colName", loc.text("TXT_AVA_BROWSER_COL_NAME")),
        ("colMode", loc.text("TXT_AVA_BROWSER_COL_MODE")),
        ("colDrop", loc.text("TXT_AVA_BROWSER_COL_DROP")),
        ("colPlayers", loc.text("TXT_AVA_BROWSER_COL_PLAYERS")),
        ("colPing", loc.text("TXT_AVA_BROWSER_COL_PING")),
        ("colDownload", loc.text("TXT_AVA_BROWSER_COL_DOWNLOAD")),
        ("status", status),
        ("empty", empty),
        ("pingHelp", loc.text("TXT_AVA_BROWSER_PING_HELP")),
    ] {
        let _ = cx.set_member(&o, k, Value::str(&v));
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_steps_as_the_inventory_and_stays_inside_the_view() {
        let mut w = Window { first: 0, focus: 0, rows: 11, band: 10 };
        for _ in 0..10 {
            assert!(w.step(1, 30));
        }
        assert_eq!((w.first, w.focus), (0, 10), "the focus moves to the band");
        assert!(w.step(1, 30));
        assert_eq!((w.first, w.focus, w.selected()), (1, 10, 11), "then the window");
        assert!(w.step(-1, 30));
        assert_eq!((w.first, w.focus), (0, 10), "up from a moved window moves the window");
        assert!(w.step(-1, 30));
        assert_eq!((w.first, w.focus), (0, 9));
        for _ in 0..40 {
            w.step(1, 30);
        }
        assert_eq!((w.first, w.focus, w.selected()), (19, 10, 29), "never past the last row");
        assert!(!w.step(1, 30));
        assert!(w.page(-1, 30) && w.first == 8);
        assert!(w.page(1, 30) && w.first == 19 && !w.page(1, 30));
        let mut small = Window { first: 0, focus: 0, rows: 11, band: 10 };
        assert!(!small.step(1, 1) && small.selected() == 0, "one row");
        small.clamp(0);
        assert_eq!(small.selected(), 0);
        let mut few = Window { first: 0, focus: 9, rows: 11, band: 10 };
        few.clamp(3);
        assert_eq!((few.first, few.focus), (0, 2), "a shorter view keeps the focus on a row");
    }
}
