mod engine;
#[cfg(test)]
mod tests;

pub use engine::{Candidate, Collector, Likes, PeakDrop, kind};

use ssx_assets::career::DropType;
use ssx_assets::career::profile::DropState;
use ssx_assets::career::random::GlobalRandom;

use super::{FeEnv, FrontEnd, screens};
use crate::game::loc::Localization;

pub const COUNT: usize = 10;

#[derive(Clone, Debug, Default)]
pub struct Recommend {
    pub likes: Likes,
    pub list: Vec<Entry>,
    pub flow_drop: u32,
    pub chosen: Option<Candidate>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub candidate: Candidate,
    pub title: String,
    pub body: String,
    pub title_icon: String,
}

pub fn title_icon(event_type: i64) -> &'static str {
    match event_type {
        0..=2 => "assets\\drop_selector_icons\\trick_icon.dds",
        3 | 4 => "assets\\drop_selector_icons\\survive_icon.dds",
        5..=7 => "assets\\drop_selector_icons\\race_icon.dds",
        _ => "",
    }
}

const GEAR_NAMES: [&str; engine::GEAR_TYPES] = [
    "TXT_REQUIREDEQUIPMENT_WINGSUIT",
    "TXT_REQUIREDEQUIPMENT_ICEAXE",
    "TXT_REQUIREDEQUIPMENT_OXYGEN",
    "TXT_REQUIREDEQUIPMENT_THERMALPROTECTION",
    "TXT_REQUIREDEQUIPMENT_ARMOUR",
    "TXT_REQUIREDEQUIPMENT_NIGHTVISION",
    "TXT_REQUIREDEQUIPMENT_WHITEOUTVISION",
];
const GEAR_BODIES: [&str; engine::GEAR_TYPES] = [
    "TXT_RECOMMENDATION_LIKEIT_WINGSUIT_REQUIRED",
    "TXT_RECOMMENDATION_LIKEIT_ICEAXE_REQUIRED",
    "TXT_RECOMMENDATION_LIKEIT_OXYGEN_REQUIRED",
    "TXT_RECOMMENDATION_LIKEIT_THERMALPROTECTION_REQUIRED",
    "TXT_RECOMMENDATION_LIKEIT_ARMOUR_REQUIRED",
    "TXT_RECOMMENDATION_LIKEIT_HEADLAMP_REQUIRED",
    "TXT_RECOMMENDATION_LIKEIT_PULSEGOGGLE_REQUIRED",
];

pub fn fill(fe: &FrontEnd, loc: &mut Localization, c: &Candidate) -> Entry {
    let e = fe.career.event(c.drop);
    let event_type = e.map_or(-1, |e| e.event_type);
    let title_key = e.and_then(|e| fe.career.drop(e.drop)).map_or("", |d| d.title.as_str());
    let drop_title = loc.text(title_key);
    let mut entry =
        Entry { candidate: *c, title: String::new(), body: String::new(), title_icon: title_icon(event_type).into() };
    match c.kind {
        kind::FIRST_TRACKS => {
            entry.title = loc.text("TXT_RECOMMENDATION_EXPLORE_TITLE_FIRST_TRACKS");
            let body = match event_type {
                0..=2 => "TXT_RECOMMENDATION_EXPLORE_DESCRIPTION_GLOBE_FIRST_TRACKS_TRICK",
                3 | 4 => "TXT_RECOMMENDATION_EXPLORE_DESCRIPTION_GLOBE_FIRST_TRACKS_SURVIVE",
                5..=7 => "TXT_RECOMMENDATION_EXPLORE_DESCRIPTION_GLOBE_FIRST_TRACKS_RACE",
                _ => "",
            };
            if !body.is_empty() {
                entry.body = loc.format(body, &[&drop_title]);
            }
        }
        kind::FAVORITE => {
            entry.title = loc.format("TXT_RECOMMENDATION_FAVORITEDROP_TITLE", &[&drop_title]);
            let body = match event_type {
                5..=7 => "TXT_RECOMMENDATION_FAVORITEDROP_RACE_DESCRIPTION",
                0..=2 => "TXT_RECOMMENDATION_FAVORITEDROP_TRICK_DESCRIPTION",
                3 | 4 => "TXT_RECOMMENDATION_FAVORITEDROP_SURVIVE_DESCRIPTION",
                _ => "",
            };
            if !body.is_empty() {
                entry.body = loc.format(body, &[&drop_title]);
            }
        }
        kind::GEAR => {
            let (name, body) = (GEAR_NAMES.get(c.gear), GEAR_BODIES.get(c.gear));
            let gear = name.map_or_else(String::new, |k| loc.text(k));
            entry.title = loc.format("TXT_RECOMMENDATION_LIKEIT_GEARREQUIRED_TITLE", &[&gear]);
            if let Some(body) = body {
                entry.body = loc.format(body, &[&gear, &drop_title]);
            }
        }
        _ => {}
    }
    entry
}

pub fn explore_peaks(fe: &FrontEnd) -> Vec<Vec<PeakDrop>> {
    let Some(nodes) = fe.globe_scene.as_ref().map(|s| &s.nodes) else { return Vec::new() };
    let mut out = Vec::new();
    for &range in nodes.ranges.keys() {
        for peak in nodes.peaks_of(range) {
            let drops = nodes
                .drops_of(peak.key)
                .filter_map(|d| fe.career.event(d.key))
                .filter(|e| e.drop_type == DropType::Explore)
                .map(|e| PeakDrop {
                    key: e.key,
                    cost: e.cost as u32,
                    event_type: e.event_type,
                    required: e.required.clone(),
                    open: fe.profile.drop_state(e.key) != DropState::Locked,
                    own_best: fe.profile.explore_scores.get(&e.key).is_some_and(|s| s.score != u32::MAX),
                })
                .collect();
            out.push(drops);
        }
    }
    out
}

pub fn build(fe: &mut FrontEnd, env: &mut FeEnv, count: usize, mode: i64) {
    fe.recommend.list.clear();
    let picks = if mode == 0 {
        let peaks = explore_peaks(fe);
        engine::suggestions(
            peaks.iter().map(Vec::as_slice),
            &fe.recommend.likes,
            fe.current_drop,
            count,
            &mut GlobalRandom,
        )
    } else {
        Vec::new()
    };
    let list: Vec<Entry> = picks.iter().map(|c| fill(fe, env.loc, c)).collect();
    let names: Vec<String> = list.iter().map(|e| format!("{} ({:#010x})", e.title, e.candidate.drop)).collect();
    fe.note(format!("recommendations (0xCEF464A4 {{{count}, {mode}, 1}}): {} — {}", list.len(), names.join(", ")));
    fe.recommend.list = list;
}

fn show(fe: &mut FrontEnd, env: &mut FeEnv, e: &Entry) {
    let v = &mut *env.values;
    v.set_str(0x1C6, e.title.clone());
    v.set_str(0x1C4, e.body.clone());
    v.set_bool(0x1C3, e.candidate.kind == kind::RIVAL);
    v.set_bool(0xC6, true);
    v.set_str(0xCA, e.title.clone());
    v.set_str(0xC4, e.body.clone());
    v.set_str(0xC5, String::new());
    v.set_str(0xCB, e.title_icon.clone());
    v.set_str(0xC7, String::new());
    v.set_str(0xC9, String::new());
    v.set_int(0xC8, 0);
    fe.recommend.flow_drop = e.candidate.drop;
}

pub fn highlight(fe: &mut FrontEnd, env: &mut FeEnv, index: usize) {
    let Some(entry) = fe.recommend.list.get(index).cloned() else { return };
    let c = entry.candidate;
    fe.note(format!("recommendation {index}: message 0x82549A5E → {} ({:#010x})", entry.title, c.drop));
    show(fe, env, &entry);
    super::postrace::select_drop(fe, env, c.mode, c.drop);
}

pub fn accept(fe: &mut FrontEnd, env: &mut FeEnv, c: &Candidate) {
    let entry = fill(fe, env.loc, c);
    let screen = match c.mode {
        0 => Some(screens::id::EXPLORE_DROP),
        3 => Some(screens::id::GLOBAL_DROP),
        _ => None,
    };
    if let Some(screen) = screen {
        fe.note(format!("recommendation taken: message 0x8983A20F {{{screen:#x}, {:#010x}}}", c.drop));
        if let Some(s) = fe.globe_scene.as_mut() {
            s.list.clear();
        }
        env.values.set_int(0xB3, 3);
        fe.menu_selection = if c.mode == 0 { 0 } else { 2 };
        fe.game_mode = c.mode;
        fe.fe_mode = c.mode as i32;
        super::progression::go_to_node(fe, c.drop, 2);
        fe.go_to_screen(env, screen);
    }
    fe.note(format!("recommendation taken: message 0x14649424 {{{:#010x}}}", c.drop));
    if let Some(s) = fe.globe_scene.as_mut() {
        s.select_drop(c.drop);
    }
    show(fe, env, &entry);
}

pub fn clear(fe: &mut FrontEnd) {
    fe.recommend.list.clear();
}

pub fn track_upvote(fe: &mut FrontEnd) {
    let drop = fe.current_drop;
    let required = fe.career.event(drop).map(|e| e.required.clone()).unwrap_or_default();
    fe.recommend.likes.like(drop, &required);
    fe.note(format!("A_TrackUpvote: message 0x187FBCE5 {{{drop:#010x}}} → likes {:?}", fe.recommend.likes));
}
