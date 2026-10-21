use crate::avm1::Ctx;
use crate::avm1::value::Value;
use crate::game::fe::{FeEnv, FrontEnd, screens};
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
}

pub fn activate(fe: &mut FrontEnd, _env: &mut FeEnv) {
    fe.post_race = PostRaceMenu { active: true, mode: fe.game_mode, drop: fe.current_drop, panel: -1 };
    fe.note(format!("post-race menu: message 0xCEF464A4 {{10, {}, 1}}", fe.game_mode));
    for dp in ["dp_debug_menu", "dp_debug_menu_index", "dp_post_race_menu_status", "dp_post_race_quit"] {
        fe.publish.push(dp.into());
    }
}

pub fn deactivate(fe: &mut FrontEnd) {
    fe.post_race.active = false;
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
            select_drop(fe, env);
            fe.note("post-race menu: SplashScreenDisplayMessage (0x576B3006)");
            fe.go_to_screen(env, screens::id::QUIT_STAGE);
        }
        "A_TrackDropHighlighted" => {
            env.values.set_bool(RECOMMENDS_IS_RIVAL, true);
            select_drop(fe, env);
            show_panel(fe, 0);
        }
        "A_MenuQuitHighlight" => {
            select_drop(fe, env);
            fe.publish.push("dp_post_race_quit".into());
            show_panel(fe, 3);
        }
        "A_MenuOnlinePassHighlight" => {
            fe.publish.push("dp_post_race_online_pass".into());
            show_panel(fe, 2);
        }
        "A_MenuRecommendationUnhighlight" => env.values.set_bool(RECOMMENDS_IS_RIVAL, false),
        _ => {}
    }
}

fn select_drop(fe: &mut FrontEnd, env: &mut FeEnv) {
    let (mode, drop) = (fe.post_race.mode, fe.post_race.drop);
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

pub fn items(fe: &FrontEnd, values: &UiValues) -> Vec<(&'static str, &'static str, &'static str, i32)> {
    let options = values.int(OPTIONS);
    let mut out = Vec::new();
    if options & 4 != 0 {
        out.push(("TXT_LE_RESULTS_CONTINUE", "A_GameContinue", "A_MenuHighlight", 1));
    }
    if options & 1 != 0 && crate::game::fe::pause::restart_allowed(fe, values) {
        out.push(("TXT_LE_RESULTS_RESTART", "A_GameRestartStage", "A_TrackDropHighlighted", 2));
    }
    out.push(("TXT_LE_RESULTS_QUIT", "A_GameQuitStage", "A_MenuQuitHighlight", 5));
    out
}

pub fn build(fe: &FrontEnd, cx: &mut Ctx, values: &UiValues, loc: &mut Localization, dp: &str) -> Option<Value> {
    match dp {
        "dp_debug_menu" => {
            let list = items(fe, values);
            let mut out = Vec::new();
            for (key, advance, highlight, icon) in list {
                let o = Value::Object(cx.new_object());
                let text = loc.text(key);
                let _ = cx.set_member(&o, "strItem", Value::str(&text));
                let _ = cx.set_member(&o, "bEnabled", Value::Bool(true));
                let _ = cx.set_member(&o, "bFocusable", Value::Bool(true));
                let _ = cx.set_member(&o, "eventAdvance", Value::str(advance));
                let _ = cx.set_member(&o, "eventHighlight", Value::str(highlight));
                let _ = cx.set_member(&o, "iMenuIcon", Value::Number(f64::from(icon)));
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
                let _ = cx.set_member(&o, "numRecommendations", Value::Number(0.0));
                let _ = cx.set_member(&o, "recommendationIndex", Value::Number(0.0));
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
