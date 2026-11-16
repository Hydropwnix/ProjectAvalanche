use ssx_physics::event::{EventResult, Medal};

use crate::fonts::text::{Align, CharFormat, ParaFormat, parse_html};
use crate::game::fe::globe::{TUTORIAL_DROPS, mode_name_key, panel};
use crate::game::fe::{FrontEnd, Launch, screens};
use crate::game::hud::results::{EVENT_TYPE_NAMES, measure, value_text};
use crate::game::loc::Localization;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Doing {
    Starting,
    Menu(Menu),
    Ride(Ride),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Menu {
    Main,
    Settings,
    Globe(i64),
    Equipment(i64),
    Maps,
    Servers,
    Joining,
    OnlineEquipment,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ride {
    pub launch: u32,
    pub run: u32,
    pub part: Part,
    pub online: Option<Round>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Loading,
    Riding,
    Results,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Round {
    pub mode: Option<&'static str>,
    pub competing: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Timer {
    Game,
    Run,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lines {
    pub details: String,
    pub state: Option<String>,
    pub competing: bool,
    pub timer: Timer,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RideNames {
    pub title: String,
    pub place: String,
}

const OPTIONS_STATES: [&str; 4] = ["AvaOptions", "AvaOptionsPage", "AvaOptionsGame", "AvaOptionsCalibration"];

pub fn menu(screen: u32, game_mode: i64, active: &dyn Fn(&str) -> bool) -> Menu {
    if active("AvaServerBrowser") || active("AvaBrowserFilters") {
        return Menu::Servers;
    }
    if active("AvaMaps") {
        return Menu::Maps;
    }
    if OPTIONS_STATES.iter().any(|n| active(n)) {
        return Menu::Settings;
    }
    match screen {
        7..=10 => Menu::Settings,
        screens::id::EXPLORE_RANGE..=screens::id::GLOBAL_DROP
        | screens::id::CAMPAIGN_RANGE..=screens::id::CAMPAIGN_DROP => Menu::Globe(game_mode),
        screens::id::EQUIPMENT | 0x31 => Menu::Equipment(game_mode),
        _ => Menu::Main,
    }
}

pub fn starting(screen: u32) -> bool {
    screen == 0 || screen == screens::id::BOOT
}

pub fn plain(text: &str) -> String {
    let ch = CharFormat {
        face: None,
        size: 240.0,
        color: [1.0; 4],
        letter_spacing: 0.0,
        kerning: false,
        bold: false,
        italic: false,
        underline: false,
    };
    let para = ParaFormat { align: Align::Left, left_margin: 0.0, right_margin: 0.0, indent: 0.0, leading: 0.0 };
    let paragraphs: Vec<String> = parse_html(text, &ch, &para)
        .iter()
        .map(|p| p.runs.iter().map(|r| r.text.as_str()).collect::<String>())
        .filter(|p| !p.trim().is_empty())
        .collect();
    paragraphs.join(" ")
}

fn event_key(launch: &Launch) -> Option<&'static str> {
    if launch.free_ride {
        Some("TXT_EVENTTYPE_FREERIDE")
    } else if TUTORIAL_DROPS.contains(&launch.drop) {
        Some("TXT_EVENTTYPE_TUTORIAL")
    } else {
        usize::try_from(launch.event_type).ok().and_then(|t| EVENT_TYPE_NAMES.get(t).copied())
    }
}

fn mode_text(loc: &mut Localization, name: &str) -> Option<String> {
    let key = match name {
        "freeride" => "TXT_AVA_BROWSER_MODE_FREERIDE",
        "race" => "TXT_AVA_BROWSER_MODE_RACE",
        "trick" => "TXT_AVA_BROWSER_MODE_TRICK",
        "survive" => "TXT_AVA_BROWSER_MODE_SURVIVE",
        "death" => "TXT_AVA_BROWSER_MODE_DEATH",
        _ => return None,
    };
    Some(loc.text(key))
}

fn pair(loc: &mut Localization, key: &str, a: String, b: String) -> String {
    match (a.is_empty(), b.is_empty()) {
        (false, false) => loc.format(key, &[&a, &b]),
        (false, true) => a,
        (true, _) => b,
    }
}

pub fn ride_names(fe: Option<&FrontEnd>, loc: &mut Localization, launch: &Launch) -> RideNames {
    if let Some(map) = &launch.map {
        let mode = mode_text(loc, &map.mode).or_else(|| event_key(launch).map(|k| plain(&loc.text(k))));
        let title = match mode {
            Some(m) if !m.is_empty() => loc.format("TXT_AVA_PRESENCE_MAP", &[&m]),
            _ => loc.text("TXT_AVA_PRESENCE_MAP_PLAIN"),
        };
        return RideNames { title, place: map.title.clone() };
    }
    let mode = mode_name_key(launch.game_mode).map(|k| plain(&loc.text(k))).unwrap_or_default();
    let event = event_key(launch).map(|k| plain(&loc.text(k))).unwrap_or_default();
    let mut title = pair(loc, "TXT_AVA_PRESENCE_TITLE", mode, event);
    if title.is_empty() {
        title = loc.text("TXT_AVA_PRESENCE_RIDING");
    }
    let place = match fe.and_then(|fe| panel::drop_fields(fe, loc, launch.drop)) {
        Some(p) => pair(loc, "TXT_AVA_PRESENCE_PLACE", plain(&p.drop_name), plain(&p.peak_name)),
        None => String::new(),
    };
    RideNames { title, place }
}

// PORT(choice): the standing is the medal where the run has one and the place where the event is against bots, else
// none (a World Tour event's success is the results screen's own); a failed run is "Did not finish". The console had
// no Discord.
pub fn outcome(loc: &mut Localization, event_type: i64, r: &EventResult) -> String {
    if r.failed {
        return loc.text("TXT_AVA_PRESENCE_FAILED");
    }
    let t = i32::try_from(event_type).unwrap_or(-1);
    let (label, value) = measure(t, r.points, r.time_ms, r.descent_mm);
    let (label, value) = (plain(&loc.text(label)), value_text(loc, t, value));
    let shown = pair(loc, "TXT_AVA_PRESENCE_MEASURE", label, value);
    let standing = match (r.medal, r.rank) {
        (Some(m), _) => plain(&loc.text(match m {
            Medal::Gold => "TXT_RESULTS_MEDAL_GOLD",
            Medal::Silver => "TXT_RESULTS_MEDAL_SILVER",
            Medal::Bronze => "TXT_RESULTS_MEDAL_BRONZE",
        })),
        (None, Some(rank)) if matches!(t, 1 | 7) => loc.ordinal(i64::try_from(rank).map_or(i64::MAX, |k| k + 1)),
        _ => String::new(),
    };
    pair(loc, "TXT_AVA_PRESENCE_RESULT", shown, standing)
}

pub fn lines(doing: &Doing, ride: &RideNames, outcome: Option<&str>, loc: &mut Localization) -> Lines {
    let (details, state) = match *doing {
        Doing::Starting => (loc.text("TXT_AVA_PRESENCE_STARTING"), None),
        Doing::Menu(m) => match m {
            Menu::Main => (loc.text("TXT_AVA_PRESENCE_MENUS"), None),
            Menu::Settings => (loc.text("TXT_AVA_PRESENCE_SETTINGS"), None),
            Menu::Globe(mode) => (game_mode(loc, mode), Some(loc.text("TXT_AVA_PRESENCE_CHOOSING_DROP"))),
            Menu::Equipment(mode) => (game_mode(loc, mode), Some(loc.text("TXT_AVA_PRESENCE_CHOOSING_GEAR"))),
            Menu::Maps => (loc.text("TXT_AVA_PRESENCE_CHOOSING_MAP"), None),
            Menu::Servers => (loc.text("TXT_AVA_PRESENCE_MULTIPLAYER"), Some(loc.text("TXT_AVA_PRESENCE_BROWSING"))),
            Menu::Joining => (loc.text("TXT_AVA_PRESENCE_ONLINE"), Some(loc.text("TXT_AVA_PRESENCE_JOINING"))),
            Menu::OnlineEquipment => {
                (loc.text("TXT_AVA_PRESENCE_ONLINE"), Some(loc.text("TXT_AVA_PRESENCE_CHOOSING_GEAR")))
            }
        },
        Doing::Ride(r) => {
            let details = match r.online {
                None => ride.title.clone(),
                Some(round) => match round.mode.and_then(|m| mode_text(loc, m)) {
                    Some(m) => loc.format("TXT_AVA_PRESENCE_ONLINE_MODE", &[&m]),
                    None => loc.text("TXT_AVA_PRESENCE_ONLINE"),
                },
            };
            let state = match r.part {
                Part::Loading if ride.place.is_empty() => loc.text("TXT_AVA_PRESENCE_LOADING"),
                Part::Loading => loc.format("TXT_AVA_PRESENCE_LOADING_PLACE", &[&ride.place]),
                Part::Riding => ride.place.clone(),
                Part::Results if r.online.is_some() => loc.text("TXT_AVA_PRESENCE_ROUND_RESULTS"),
                Part::Results => outcome.unwrap_or_default().to_string(),
            };
            let competing = r.online.is_some_and(|o| o.competing) && r.part != Part::Results;
            let timer = if r.part == Part::Riding { Timer::Run } else { Timer::Game };
            return Lines { details, state: Some(state).filter(|s| !s.is_empty()), competing, timer };
        }
    };
    Lines { details, state, competing: false, timer: Timer::Game }
}

fn game_mode(loc: &mut Localization, mode: i64) -> String {
    match mode_name_key(mode) {
        Some(k) => plain(&loc.text(k)),
        None => loc.text("TXT_AVA_PRESENCE_MENUS"),
    }
}

#[cfg(test)]
mod tests;
