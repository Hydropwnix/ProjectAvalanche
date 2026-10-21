use crate::avm1::Ctx;
use crate::avm1::value::Value;
use crate::game::fe::{ActionParam, FeEnv, FrontEnd};
use crate::game::loc::Localization;

pub const DP_MENU: &str = "dp_music_menu";
pub const DP_MENU_INDEX: &str = "dp_music_menu_index";
pub const DP_CATEGORIES: &str = "dp_music_personalize_category_data";
pub const DP_SORT_TYPES: &str = "dp_music_personalize_category_sort_types";
pub const DP_CATEGORY_MUSIC: &str = "dp_music_personalize_category_music_data";
pub const DP_SSX_MUSIC: &str = "dp_music_ssx_music_data";
pub const DP_SCREEN: &str = "dp_music_screen_data";
pub const DPS: [&str; 7] =
    [DP_MENU, DP_MENU_INDEX, DP_CATEGORIES, DP_SORT_TYPES, DP_CATEGORY_MUSIC, DP_SSX_MUSIC, DP_SCREEN];

pub const TRACK_LISTING: u32 = 0x3C;
pub const PERSONALIZE: u32 = 0x3B;
pub const ROWS: usize = 8;
const NONE: i32 = 0x7FFF_FFFF;
const PREVIEW_HOLD: u32 = 10;
pub const SOURCE_NAMES: [&str; 6] = [
    "TXT_MUSIC_OPTION_DEFAULT",
    "TXT_MUSIC_OPTION_SONG",
    "TXT_MUSIC_OPTION_PLAYLIST",
    "TXT_MUSIC_OPTION_ALBUM",
    "TXT_MUSIC_OPTION_ARTIST",
    "TXT_MUSIC_OPTION_GENRE",
];
pub const CATEGORY_NAMES: [&str; 4] =
    ["TXT_RACE_MUSIC", "TXT_TRICK_MUSIC", "TXT_SURVIVE_MUSIC", "TXT_USER_INTERFACE_MUSIC"];
pub const CUSTOM_SLOTS: usize = 20;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LicensedSong {
    pub id: u32,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub on: bool,
}

pub fn licensed_songs(mb: &ssx_assets::musicbox::MusicBox) -> Vec<LicensedSong> {
    ssx_assets::sfx::store::in_game_music(&mb.songs)
        .into_iter()
        .map(|(id, on)| {
            let s = mb.song(id);
            let get = |k: &str| s.and_then(|s| s.get(k)).unwrap_or("").to_string();
            LicensedSong { id, title: get("Title"), artist: get("Artist"), album: get("Album"), on: on != 0 }
        })
        .collect()
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Browser {
    pub state: u8,
    pub first: usize,
    pub preview: Option<usize>,
    hold: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CustomSlot {
    pub override_type: i32,
    pub name: String,
    pub item_type: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MusicRequest {
    Preview(u32),
    StopPreview,
}

#[derive(Clone, Debug)]
pub struct Music {
    pub songs: Vec<LicensedSong>,
    pub browser: Browser,
    pub page: Page,
    pub custom: Vec<CustomSlot>,
    pub controller_playing: bool,
    pub requests: Vec<MusicRequest>,
    pub alive: bool,
    preview_row: i32,
    total: i32,
    exit_after_save: bool,
    category: i32,
    source: i32,
    state: i32,
    input_free: bool,
    wait: i32,
    category_name: String,
    source_name: String,
    music_heading: String,
    placeholder: bool,
    page_written: bool,
    custom_written: bool,
    pub last_screen: u32,
    last_row: i32,
    last_playing: bool,
    last_state: i32,
}

impl Default for Music {
    fn default() -> Self {
        Music {
            songs: Vec::new(),
            browser: Browser::default(),
            page: Page::default(),
            custom: vec![CustomSlot::default(); CUSTOM_SLOTS],
            controller_playing: false,
            requests: Vec::new(),
            alive: false,
            preview_row: NONE,
            total: -1,
            exit_after_save: false,
            category: 0,
            source: -1,
            state: 1,
            input_free: true,
            wait: 0,
            category_name: String::new(),
            source_name: String::new(),
            music_heading: String::new(),
            placeholder: false,
            page_written: false,
            custom_written: false,
            last_screen: TRACK_LISTING,
            last_row: NONE,
            last_playing: false,
            last_state: 0,
        }
    }
}

impl Music {
    pub fn with_songs(songs: Vec<LicensedSong>) -> Self {
        Music { songs, ..Default::default() }
    }

    pub fn set_flags(&mut self, flags: &[(u32, bool)]) {
        for s in &mut self.songs {
            if let Some(&(_, on)) = flags.iter().find(|(id, _)| *id == s.id) {
                s.on = on;
            }
        }
    }

    pub fn flags(&self) -> Vec<(u32, bool)> {
        self.songs.iter().map(|s| (s.id, s.on)).collect()
    }

    pub fn rows(&self) -> &[LicensedSong] {
        let first = self.browser.first.min(self.songs.len());
        &self.songs[first..(first + ROWS).min(self.songs.len())]
    }

    pub fn has_next(&self) -> bool {
        self.browser.first + ROWS < self.songs.len()
    }

    pub fn has_prev(&self) -> bool {
        self.browser.first > 0
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page {
    pub rows: Vec<LicensedSong>,
}

impl Music {
    // TODO(guess): the service's update runs before the providers' in a frame (`FMSoundSystem` 0x82015EE0 +0x1C 0x823F7730 → 0x824F9BE0 → 0x8254BB58, the FE manager's 0x8237E4C0); settle: the managers' update loop order.
    pub fn service_update(&mut self) {
        self.page_written = false;
        match self.browser.state {
            2 => self.browser.state = 3,
            3 => {
                self.page.rows = self.rows().to_vec();
                self.page_written = true;
                self.browser.state = 4;
            }
            4 => self.browser.state = 5,
            _ => {}
        }
        if self.browser.hold > 0 {
            self.browser.hold -= 1;
            if self.browser.hold == 0 {
                self.requests.push(MusicRequest::StopPreview);
                self.browser.preview = None;
            }
        }
    }

    fn service_preview(&mut self, row: i32) {
        match self.browser.state {
            0 => self.browser.state = 2,
            5 if row == NONE => self.browser.preview = None,
            5 => {
                let abs = self.browser.first + row.max(0) as usize;
                if self.browser.preview != Some(abs)
                    && let Some(s) = self.songs.get(abs).filter(|s| s.id != 0)
                {
                    self.requests.push(MusicRequest::Preview(s.id));
                }
                self.browser.preview = Some(abs);
                self.browser.hold = PREVIEW_HOLD;
            }
            _ => {}
        }
    }

    fn service_end(&mut self) {
        self.browser.first = 0;
        self.browser.state = 0;
    }

    fn service_toggle(&mut self, row: usize, on: bool) {
        if let Some(s) = self.songs.get_mut(self.browser.first + row) {
            s.on = on;
        }
        if let Some(r) = self.page.rows.get_mut(row) {
            r.on = on;
        }
    }

    fn service_reset(&mut self) {
        for s in &mut self.songs {
            s.on = true;
        }
        for r in &mut self.page.rows {
            r.on = true;
        }
    }

    fn service_next(&mut self) -> bool {
        if self.browser.state != 5 {
            return false;
        }
        if self.has_next() {
            self.browser.state = 2;
            self.browser.first += 1;
        }
        self.browser.state == 2
    }

    fn service_prev(&mut self) -> bool {
        if self.browser.state != 5 {
            return false;
        }
        if self.has_prev() {
            self.browser.state = 2;
            self.browser.first -= 1;
        }
        self.browser.state == 2
    }

    fn user_ready(&self) -> bool {
        true
    }

    fn category_default(&mut self, category: i32) {
        if let Some(c) = usize::try_from(category - 1).ok().and_then(|i| self.custom.get_mut(i)) {
            *c = CustomSlot::default();
        }
        self.custom_written = true;
    }
}

const WAITS: [i32; 6] = [0, 10, 10, 30, 30, 30];

fn busy_icon(fe: &mut FrontEnd, env: &mut FeEnv, on: bool) {
    fe.busy.set_music(env, Some(on));
}

pub fn activate(fe: &mut FrontEnd, env: &mut FeEnv) {
    let m = &mut fe.music;
    m.alive = true;
    m.preview_row = NONE;
    m.total = -1;
    m.exit_after_save = false;
    m.category = 0;
    m.source = -1;
    m.state = 1;
    m.input_free = true;
    m.wait = 0;
    m.placeholder = false;
    m.category_name.clear();
    m.source_name.clear();
    m.music_heading.clear();
    busy_icon(fe, env, false);
    fe.publish.extend(DPS.iter().map(|d| d.to_string()));
}

pub fn deactivate(fe: &mut FrontEnd, env: &mut FeEnv) {
    fe.music.alive = false;
    fe.music.service_end();
    if !fe.music.input_free {
        fe.input_lock -= 1;
        fe.music.input_free = true;
    }
    fe.busy.set_music(env, None);
}

pub fn update(fe: &mut FrontEnd, env: &mut FeEnv) {
    fe.music.service_update();
    if !fe.music.alive {
        fe.music.custom_written = false;
        return;
    }
    let m = &mut fe.music;
    let now = m.controller_playing;
    if m.total < 0 {
        m.total = m.songs.len() as i32;
    }
    let mut changed = !m.last_playing && now && m.last_row == NONE;
    if m.last_row != m.preview_row {
        changed = true;
    }
    if m.last_playing && !now && m.preview_row != NONE {
        changed = true;
        m.preview_row = NONE;
    }
    m.last_row = m.preview_row;
    let changed = changed || m.page_written;
    m.last_playing = now;
    let row = m.preview_row;
    m.service_preview(row);
    let mut publish = Vec::new();
    if changed {
        publish.push(DP_SSX_MUSIC);
    }
    if std::mem::take(&mut m.custom_written) {
        publish.push(DP_CATEGORIES);
    }
    let state = m.state;
    let mut free = m.input_free;
    if m.last_state != state {
        match state {
            1 => free = true,
            2 => {
                free = false;
                publish.push(DP_SORT_TYPES);
                m.category_name = name_at(&CATEGORY_NAMES, m.category - 1, env);
            }
            3 => {
                free = false;
                publish.push(DP_CATEGORY_MUSIC);
                m.music_heading = heading(m, env);
                publish.push(DP_SCREEN);
            }
            4 => free = false,
            _ => {}
        }
        free = free && state != 5;
    }
    if free != m.input_free {
        if free {
            fe.input_lock -= 1;
        } else {
            m.wait = WAITS.get(state as usize).copied().unwrap_or(0);
            fe.input_lock += 1;
        }
        m.input_free = free;
    }
    m.last_state = m.state;
    match m.state {
        2..=5 if m.wait >= 1 => m.wait -= 1,
        2 | 3 | 5 if m.user_ready() => m.state = 1,
        4 if m.user_ready() => {
            publish.push(DP_CATEGORY_MUSIC);
            m.music_heading = heading(m, env);
            publish.push(DP_SCREEN);
            m.state = 1;
        }
        _ => {}
    }
    let busy = m.state >= 3;
    busy_icon(fe, env, busy);
    fe.publish.extend(publish.into_iter().map(String::from));
}

fn name_at(table: &[&str], i: i32, env: &mut FeEnv) -> String {
    usize::try_from(i).ok().and_then(|i| table.get(i)).map_or_else(String::new, |k| env.loc.text(k))
}

fn heading(m: &mut Music, env: &mut FeEnv) -> String {
    m.source_name = name_at(&SOURCE_NAMES, m.source, env);
    env.loc.format("TXT_PERSONALIZE_MUSIC_HEADING", &[m.source_name.as_str(), m.category_name.as_str()])
}

pub fn action(fe: &mut FrontEnd, env: &mut FeEnv, action: &str, param: &ActionParam) {
    let screen = fe.music.last_screen;
    match action {
        "A_ActivityFeedMusicSelectItem" => match param.number as i64 {
            0 => {
                fe.music.last_screen = TRACK_LISTING;
                env.nav_events.push("go_to_activity_feed_track_listing".into());
            }
            1 => {
                fe.music.last_screen = PERSONALIZE;
                env.nav_events.push("go_to_activity_feed_personalize_music".into());
                fe.music.state = 0;
            }
            _ => {}
        },
        "A_ActivityFeedMusicNextPage" | "A_ActivityFeedMusicPrevPage" => {
            let next = action == "A_ActivityFeedMusicNextPage";
            let moved = match screen {
                PERSONALIZE => {
                    fe.publish.push(DP_CATEGORY_MUSIC.into());
                    fe.play_sound(1);
                    return;
                }
                TRACK_LISTING if next => fe.music.service_next(),
                TRACK_LISTING => fe.music.service_prev(),
                _ => return,
            };
            if !moved {
                fe.publish.push(DP_SSX_MUSIC.into());
                fe.play_sound(1);
                return;
            }
            let m = &mut fe.music;
            if m.preview_row != NONE {
                m.preview_row += if next { -1 } else { 1 };
            }
            fe.play_sound(0);
        }
        "A_ActivityFeedMusicSaveChanges" => {
            fe.music.exit_after_save = param.number != 0.0;
            fe.requests.push(crate::game::fe::pause::GameRequest::Save);
            let event = if fe.music.exit_after_save { "close_activity_feed" } else { "go_to_activity_feed_music_menu" };
            env.nav_events.push(event.into());
        }
        _ => {}
    }
}

pub fn data_action(fe: &mut FrontEnd, env: &mut FeEnv, dp: &str, action: &str, param: &ActionParam) {
    let n = param.number as i32;
    let m = &mut fe.music;
    match (dp, action) {
        (DP_CATEGORIES, "A_ActivityFeedMusicSelectItem") => {
            m.category = n + 1;
            m.state = 2;
        }
        (DP_SORT_TYPES, "A_ActivityFeedMusicSelectItem") => {
            m.source = n + 1;
            if m.source == 1 {
                let c = m.category;
                m.category_default(c);
                m.state = 3;
            } else {
                m.state = 4;
            }
            fe.music.placeholder = true;
            fe.publish.push(DP_CATEGORY_MUSIC.into());
            let m = &mut fe.music;
            m.music_heading = heading(m, env);
            fe.publish.push(DP_SCREEN.into());
        }
        (DP_CATEGORY_MUSIC, "A_ActivityFeedMusicSelectItem") => m.state = 5,
        (DP_SSX_MUSIC, "A_ActivityFeedMusicSelectItem") => {
            if let Some(on) = m.page.rows.get(n.max(0) as usize).map(|r| r.on) {
                m.service_toggle(n.max(0) as usize, !on);
            }
        }
        (DP_SSX_MUSIC, "A_ActivityFeedMusicPlaySong") => {
            m.preview_row = if m.preview_row == n { NONE } else { n };
        }
        (DP_SSX_MUSIC | DP_SCREEN, "A_ActivityFeedMusicResetToDefault") => {
            env.nav_events.push("go_to_reset_music_selection_popup".into());
        }
        _ => {}
    }
}

pub fn do_reset(fe: &mut FrontEnd) {
    if fe.music.last_screen == TRACK_LISTING {
        fe.music.service_reset();
        fe.publish.push(DP_SSX_MUSIC.into());
    }
}

fn object(cx: &mut Ctx, members: Vec<(&str, Value)>) -> Value {
    let o = Value::Object(cx.new_object());
    for (k, v) in members {
        let _ = cx.set_member(&o, k, v);
    }
    o
}

fn row(cx: &mut Ctx, item: &str, description: Option<&str>, param: usize) -> Value {
    let mut m = vec![("strItem", Value::str(item))];
    if let Some(d) = description {
        m.push(("strDescription", Value::str(d)));
    }
    m.push(("eventAdvance", Value::str("A_ActivityFeedMusicSelectItem")));
    m.push(("eventAdvanceParam", Value::Number(param as f64)));
    object(cx, m)
}

pub fn build(fe: &mut FrontEnd, cx: &mut Ctx, loc: &mut Localization, dp: &str) -> Option<Value> {
    let m = &mut fe.music;
    let list = |cx: &mut Ctx, rows: Vec<Value>| Some(Value::Object(cx.new_array(rows)));
    match dp {
        DP_MENU => {
            let mut rows = Vec::new();
            for (i, key) in ["TXT_SSX_MUSIC", "TXT_PERSONALIZE_YOUR_MUSIC"].iter().enumerate() {
                rows.push(object(
                    cx,
                    vec![
                        ("strItem", Value::str(&loc.text(key))),
                        ("bEnabled", Value::Bool(true)),
                        ("bFocusable", Value::Bool(true)),
                        ("eventAdvance", Value::str("A_ActivityFeedMusicSelectItem")),
                        ("eventAdvanceParam", Value::Number(i as f64)),
                    ],
                ));
            }
            list(cx, rows)
        }
        DP_MENU_INDEX => Some(Value::Number(if m.last_screen == TRACK_LISTING { 0.0 } else { 1.0 })),
        DP_CATEGORIES => {
            let default = loc.text(SOURCE_NAMES[0]);
            let mut rows = Vec::new();
            for (i, slot) in m.custom.iter().enumerate().take(CATEGORY_NAMES.len()) {
                let description = if slot.item_type == 0 { default.clone() } else { slot.name.clone() };
                rows.push(row(cx, &loc.text(CATEGORY_NAMES[i]), Some(&description), i));
            }
            list(cx, rows)
        }
        DP_SORT_TYPES => {
            let rows = vec![row(cx, &loc.text(SOURCE_NAMES[0]), None, 0), row(cx, &loc.text(SOURCE_NAMES[2]), None, 1)];
            list(cx, rows)
        }
        DP_CATEGORY_MUSIC => {
            let rows = if std::mem::take(&mut m.placeholder) || m.wait > 0 {
                vec![row(cx, "", Some(""), 0)]
            } else {
                Vec::new()
            };
            list(cx, rows)
        }
        DP_SSX_MUSIC => {
            let (preview, total) = (m.preview_row, m.total);
            let mut rows = Vec::new();
            for (i, s) in m.page.rows.iter().enumerate().take(ROWS) {
                if s.title.is_empty() {
                    break;
                }
                rows.push(object(
                    cx,
                    vec![
                        ("strSong", Value::str(&s.title)),
                        ("strArtist", Value::str(&s.artist)),
                        ("isChecked", Value::Bool(s.on)),
                        ("eventSelect", Value::str("A_ActivityFeedMusicSelectItem")),
                        ("eventSelectParam", Value::Number(i as f64)),
                        ("eventPlay", Value::str("A_ActivityFeedMusicPlaySong")),
                        ("eventPlayParam", Value::Number(i as f64)),
                        ("isPlaying", Value::Number(f64::from(u8::from(preview == i as i32)))),
                        ("numTotalSongs", Value::Number(f64::from(total))),
                    ],
                ));
            }
            list(cx, rows)
        }
        DP_SCREEN => {
            let personalize = loc.text("TXT_PERSONALIZE_YOUR_MUSIC");
            let mut members = vec![
                ("strSelectCategoryHeading", Value::str(&personalize)),
                ("strSelectSortTypeHeading", Value::str(&personalize)),
                ("strNoMusicDescription", Value::str(&loc.text("TXT_MUSIC_NO_MUSIC_AVAILABLE_WARNING"))),
            ];
            if !m.music_heading.is_empty() {
                members.push(("strSelectMusicHeading", Value::str(&m.music_heading)));
            }
            Some(object(cx, members))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn music(n: usize) -> Music {
        let songs = (0..n)
            .map(|i| LicensedSong { id: 100 + i as u32, title: format!("song {i}"), on: true, ..Default::default() })
            .collect();
        Music::with_songs(songs)
    }

    fn ready(m: &mut Music) {
        m.service_preview(NONE);
        for _ in 0..3 {
            m.service_update();
        }
        assert_eq!(m.browser.state, 5);
    }

    #[test]
    fn the_window_moves_one_row_and_stops_at_the_ends() {
        let mut m = music(10);
        ready(&mut m);
        assert_eq!(m.page.rows.len(), ROWS);
        assert!(!m.service_prev(), "no row above the first");
        assert!(m.service_next());
        assert_eq!((m.browser.first, m.browser.state), (1, 2));
        assert!(!m.service_next(), "a browser still reading does not move");
        ready_again(&mut m);
        assert!(m.service_next());
        ready_again(&mut m);
        assert_eq!(m.page.rows[0].title, "song 2");
        assert!(!m.has_next() && !m.service_next(), "ten songs: the last window starts at 2");
        m.service_end();
        assert_eq!((m.browser.first, m.browser.state), (0, 0));
    }

    fn ready_again(m: &mut Music) {
        for _ in 0..3 {
            m.service_update();
        }
    }

    #[test]
    fn a_toggle_and_the_reset_change_the_song_and_the_page() {
        let mut m = music(10);
        ready(&mut m);
        assert!(m.service_next());
        ready_again(&mut m);
        m.service_toggle(2, false);
        assert!(!m.songs[3].on && !m.page.rows[2].on);
        assert_eq!(m.flags()[3], (103, false));
        m.service_reset();
        assert!(m.songs.iter().all(|s| s.on) && m.page.rows.iter().all(|r| r.on));
        let mut other = music(10);
        other.set_flags(&[(105, false), (999, false)]);
        assert!(!other.songs[5].on && other.songs.iter().filter(|s| !s.on).count() == 1);
    }

    #[test]
    fn a_preview_is_asked_once_and_held_ten_updates() {
        let mut m = music(10);
        ready(&mut m);
        m.service_preview(1);
        m.service_preview(1);
        assert_eq!(m.requests, vec![MusicRequest::Preview(101)]);
        assert_eq!(m.browser.preview, Some(1));
        m.service_preview(NONE);
        assert_eq!(m.browser.preview, None);
        for _ in 0..PREVIEW_HOLD - 1 {
            m.service_update();
        }
        assert_eq!(m.requests.len(), 1);
        m.service_update();
        assert_eq!(m.requests.last(), Some(&MusicRequest::StopPreview));
    }

    #[test]
    fn a_category_goes_back_to_the_ssx_music() {
        let mut m = music(1);
        m.custom[1] = CustomSlot { override_type: 2, name: "mine".into(), item_type: 2 };
        m.category_default(2);
        assert_eq!(m.custom[1], CustomSlot::default());
        assert!(m.custom_written);
        m.category_default(0);
        assert_eq!(m.custom.len(), CUSTOM_SLOTS);
    }
}
