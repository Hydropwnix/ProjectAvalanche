use crate::avm1::Ctx;
use crate::avm1::value::Value;
use crate::game::fe::{FeEnv, FrontEnd, recommend, screens};
use crate::game::loc::Localization;
use crate::game::values::UiValues;

pub const OPTIONS: u16 = 0xA5;
pub const RECOMMENDS_IS_RIVAL: u16 = 0x1C3;
pub const EXIT_ICON: &str = "assets\\results_Icons\\exit_to_globe.dds";

#[derive(Clone, Debug, Default)]
pub struct PostRaceMenu {
    pub active: bool,
    pub mode: i64,
    pub drop: u32,
    pub panel: i32,
    pub index: usize,
    pub selected: Option<recommend::Candidate>,
    pub chosen: bool,
}

pub fn activate(fe: &mut FrontEnd, env: &mut FeEnv) {
    fe.post_race =
        PostRaceMenu { active: true, mode: fe.game_mode, drop: fe.current_drop, panel: -1, ..Default::default() };
    fe.recommend.flow_drop = 0;
    recommend::build(fe, env, recommend::COUNT, fe.game_mode);
    for dp in ["dp_debug_menu", "dp_debug_menu_index", "dp_post_race_menu_status", "dp_post_race_quit"] {
        fe.publish.push(dp.into());
    }
}

pub fn deactivate(fe: &mut FrontEnd) {
    fe.post_race.active = false;
    if !fe.post_race.chosen {
        recommend::clear(fe);
    }
}

pub fn action(fe: &mut FrontEnd, env: &mut FeEnv, action: &str) {
    match action {
        "A_GameRestartStage" => {
            fe.note("post-race menu: SplashScreenDisplayMessage (0x576B3006)");
            fe.go_to_screen(env, screens::id::RESTART_STAGE);
        }
        "A_GameContinue" => {
            fe.note("post-race menu: SplashScreenDisplayMessage (0x576B3006)");
            fe.go_to_screen(env, screens::id::CONTINUE_STAGE);
        }
        "A_GameQuitStage" => {
            ridden_drop(fe, env);
            fe.note("post-race menu: SplashScreenDisplayMessage (0x576B3006)");
            fe.go_to_screen(env, screens::id::QUIT_STAGE);
        }
        "A_TrackDropHighlighted" => {
            env.values.set_bool(RECOMMENDS_IS_RIVAL, true);
            ridden_drop(fe, env);
            show_panel(fe, 0);
        }
        "A_MenuQuitHighlight" => {
            ridden_drop(fe, env);
            fe.publish.push("dp_post_race_quit".into());
            show_panel(fe, 3);
        }
        "A_MenuOnlinePassHighlight" => {
            fe.publish.push("dp_post_race_online_pass".into());
            show_panel(fe, 2);
        }
        "A_MenuRecommendationHighlight" => suggestion(fe, env),
        "A_MenuNextRecommendation" => {
            fe.post_race.index = next_index(fe.post_race.index, fe.recommend.list.len());
            suggestion(fe, env);
        }
        "A_MenuRecommendationUnhighlight" => {
            fe.recommend.flow_drop = 0;
            env.values.set_bool(RECOMMENDS_IS_RIVAL, false);
        }
        "A_GameRecommendation" => {
            fe.post_race.chosen = true;
            // TODO(guess): with an empty list (every open Explore drop played, no like) +0x98 holds no entry's id
            // and the port's quit then returns as without a suggestion; the console passes whatever +0x98 holds to
            // +0x74 (1, id) and its quit (0x82635980) posts 0xCC4A7421 {id, 1} without writing UI 0x1B5; settle: a
            // console run with every open Explore drop played, A on Recommendation.
            fe.recommend.chosen = fe.post_race.selected;
            fe.note(format!(
                "post-race menu: the suggestion taken (FE manager +0x74 {{1, {:?}}})",
                fe.recommend.chosen
            ));
            fe.note("post-race menu: SplashScreenDisplayMessage (0x576B3006)");
            fe.go_to_screen(env, screens::id::QUIT_STAGE);
        }
        _ => {}
    }
}

pub fn next_index(index: usize, len: usize) -> usize {
    let next = index.saturating_add(1);
    if next >= len || next > 9 { 0 } else { next }
}

fn suggestion(fe: &mut FrontEnd, env: &mut FeEnv) {
    let index = fe.post_race.index;
    // TODO(guess): an empty list shows the panel with no suggestion (no 0x82549A5E); the console reads the id
    // +0x4CC of entry 0 past the vector's end (0x825D03C8 and 0x825D0468 check no length); settle: a console run with
    // every open Explore drop played, the Recommendation item highlighted.
    fe.post_race.selected = fe.recommend.list.get(index).map(|e| e.candidate);
    recommend::highlight(fe, env, index);
    show_panel(fe, 1);
}

fn ridden_drop(fe: &mut FrontEnd, env: &mut FeEnv) {
    let (mode, drop) = (fe.post_race.mode, fe.post_race.drop);
    select_drop(fe, env, mode, drop);
}

pub(crate) fn select_drop(fe: &mut FrontEnd, env: &mut FeEnv, mode: i64, drop: u32) {
    if fe.ava.maps.riding.is_some() {
        return;
    }
    fe.note(format!(
        "post-race menu: message 0x945FF5CC {{{mode}, {drop:#010x}}} → SelectedGlobeNodeMessage {{3, drop}}"
    ));
    fe.game_mode = mode;
    fe.fe_mode = mode as i32;
    crate::game::fe::globe::drop_node_selected(fe, env, drop);
}

fn show_panel(fe: &mut FrontEnd, panel: i32) {
    fe.post_race.panel = panel;
    fe.publish.push("dp_post_race_menu_status".into());
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Item {
    pub key: &'static str,
    pub advance: &'static str,
    pub highlight: &'static str,
    pub unhighlight: Option<&'static str>,
    pub next: Option<&'static str>,
    pub icon: i32,
}

impl Item {
    const fn new(key: &'static str, advance: &'static str, highlight: &'static str, icon: i32) -> Self {
        Self { key, advance, highlight, unhighlight: None, next: None, icon }
    }
}

pub const RECOMMENDATION: Item = Item {
    key: "TXT_POSTRACEMENU_RECOMMENDATION",
    advance: "A_GameRecommendation",
    highlight: "A_MenuRecommendationHighlight",
    unhighlight: Some("A_MenuRecommendationUnhighlight"),
    next: Some("A_MenuNextRecommendation"),
    icon: 3,
};

pub fn items(fe: &FrontEnd, values: &UiValues) -> Vec<Item> {
    let options = values.int(OPTIONS);
    let mut out = Vec::new();
    if options & 4 != 0 {
        out.push(Item::new("TXT_LE_RESULTS_CONTINUE", "A_GameContinue", "A_MenuHighlight", 1));
    }
    if options & 1 != 0 && crate::game::fe::pause::restart_allowed(fe, values) {
        out.push(Item::new("TXT_LE_RESULTS_RESTART", "A_GameRestartStage", "A_TrackDropHighlighted", 2));
    }
    if options & 8 != 0 {
        out.push(RECOMMENDATION);
    }
    out.push(Item::new("TXT_LE_RESULTS_QUIT", "A_GameQuitStage", "A_MenuQuitHighlight", 5));
    out
}

pub fn build(fe: &FrontEnd, cx: &mut Ctx, values: &UiValues, loc: &mut Localization, dp: &str) -> Option<Value> {
    match dp {
        "dp_debug_menu" => {
            let mut out = Vec::new();
            for item in items(fe, values) {
                let o = Value::Object(cx.new_object());
                let text = loc.text(item.key);
                let _ = cx.set_member(&o, "strItem", Value::str(&text));
                let _ = cx.set_member(&o, "bEnabled", Value::Bool(true));
                let _ = cx.set_member(&o, "bFocusable", Value::Bool(true));
                let _ = cx.set_member(&o, "eventAdvance", Value::str(item.advance));
                let _ = cx.set_member(&o, "eventHighlight", Value::str(item.highlight));
                if let Some(e) = item.unhighlight {
                    let _ = cx.set_member(&o, "eventUnhighlight", Value::str(e));
                }
                if let Some(e) = item.next {
                    let _ = cx.set_member(&o, "eventNextRecommendation", Value::str(e));
                }
                let _ = cx.set_member(&o, "iMenuIcon", Value::Number(f64::from(item.icon)));
                out.push(o);
            }
            Some(Value::Object(cx.new_array(out)))
        }
        "dp_debug_menu_index" => Some(Value::Number(0.0)),
        "dp_post_race_menu_status" => {
            let o = Value::Object(cx.new_object());
            let m = fe.post_race.mode;
            let _ = cx.set_member(&o, "isCampaignMode", Value::Bool(m == 1));
            let _ = cx.set_member(&o, "isExploreMode", Value::Bool(m == 0));
            let _ = cx.set_member(&o, "isGlobalEventsMode", Value::Bool(m == 3));
            let p = fe.post_race.panel;
            for (k, name) in [(0, "restartPanel"), (1, "recommendationPanel"), (2, "onlinePassPanel"), (3, "quitPanel")]
            {
                let _ = cx.set_member(&o, name, Value::Bool(p == k));
            }
            if p == 1 {
                let n = fe.recommend.list.len() as f64;
                let _ = cx.set_member(&o, "numRecommendations", Value::Number(n));
                let _ = cx.set_member(&o, "recommendationIndex", Value::Number(fe.post_race.index as f64));
            }
            Some(o)
        }
        "dp_post_race_quit" => Some(quit_object(cx, loc)),
        _ => None,
    }
}

fn quit_object(cx: &mut Ctx, loc: &mut Localization) -> Value {
    let o = Value::Object(cx.new_object());
    let t1 = loc.text("TXT_POST_RACE_EXIT_TITLE");
    let t2 = loc.text("TXT_POST_RACE_EXIT_SECONDARY_TITLE");
    let d = loc.text("TXT_POST_RACE_EXIT_DESCRIPTION");
    let _ = cx.set_member(&o, "title1", Value::str(&t1));
    let _ = cx.set_member(&o, "title2", Value::str(&t2));
    let _ = cx.set_member(&o, "iconPath", Value::str(EXIT_ICON));
    let _ = cx.set_member(&o, "description", Value::str(&d));
    o
}
