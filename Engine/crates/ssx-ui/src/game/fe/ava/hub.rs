use crate::avm1::Ctx;
use crate::avm1::value::Value;
use crate::game::fe::menu::{self, MenuItem};
use crate::game::fe::{FeEnv, FrontEnd, pause::GameRequest, screens};

pub const DP_MENU: &str = "dp_ava_hub_menu";
pub const DP_INDEX: &str = "dp_ava_hub_menu_index";
pub const OPEN: &str = "go_to_ava_hub";
pub const MAIN_MENU_BACK: &str = "A_AvaMainMenuBack";
pub const EXIT_POPUP: &str = "ava_exit";
pub const EXIT_ACTION: &str = "do_ava_exit";

#[derive(Clone, Debug, Default)]
pub struct Hub {
    pub on: bool,
    pub active: bool,
    pub items: Vec<MenuItem>,
    pub index: i32,
}

pub fn activate(fe: &mut FrontEnd, env: &mut FeEnv) {
    fe.game_mode = -1;
    fe.screen = screens::id::MAIN_MENU;
    let title = env.loc.text("TXT_AVA_HUB_TITLE");
    env.values.set_str(0x32, title);
    env.values.set_str(0x33, "");
    menu::write_progress(fe, env);
    let v = &*env.values;
    let (dd, medals, total) = (v.int(0x58), v.int(0x6F), v.int(0x70));
    let loc = &mut *env.loc;
    let item = |loc: &mut crate::game::loc::Localization, key: &str, desc: String, enabled: bool, advance| MenuItem {
        item: loc.text(key),
        desc,
        enabled,
        focusable: enabled,
        advance,
        highlight: "A_AvaHubHighlight",
        rider_net: false,
        back: "",
    };
    let sp = loc.format("TXT_AVA_HUB_SINGLEPLAYER_DESC", &[&dd.to_string(), &medals.to_string(), &total.to_string()]);
    let mp = loc.text(if cfg!(feature = "net") {
        "TXT_AVA_HUB_MULTIPLAYER_DESC"
    } else {
        "TXT_AVA_HUB_MULTIPLAYER_DESC_OFF"
    });
    let maps = loc.text("TXT_AVA_HUB_MAPS_DESC");
    let options = loc.text("TXT_AVA_HUB_OPTIONS_DESC");
    let credits = loc.text("TXT_AVA_HUB_CREDITS_DESC");
    let exit = loc.text("TXT_AVA_HUB_EXIT_DESC");
    fe.ava.hub.items = vec![
        item(loc, "TXT_AVA_HUB_SINGLEPLAYER", sp, true, "A_AvaSingleplayer"),
        item(loc, "TXT_AVA_HUB_MULTIPLAYER", mp, cfg!(feature = "net"), "A_AvaMultiplayer"),
        item(loc, "TXT_AVA_HUB_MAPS", maps, true, "A_AvaMaps"),
        item(loc, "TXT_AVA_HUB_OPTIONS", options, true, "A_AvaOptions"),
        item(loc, "TXT_AVA_HUB_CREDITS", credits, true, "A_AvaCredits"),
        item(loc, "TXT_AVA_HUB_EXIT", exit, true, "A_AvaExit"),
    ];
    fe.ava.hub.index = 0;
    fe.ava.hub.active = true;
    fe.publish.extend([DP_MENU.into(), DP_INDEX.into()]);
    env.nav_events.push("backgroundcontainer_disable".into());
}

pub fn deactivate(fe: &mut FrontEnd) {
    fe.ava.hub.active = false;
}

pub fn action(fe: &mut FrontEnd, env: &mut FeEnv, action: &str, param: f64) {
    match action {
        "A_AvaSingleplayer" => env.nav_events.push("go_to_menu".into()),
        #[cfg(feature = "net")]
        "A_AvaMultiplayer" => env.nav_events.push(crate::game::fe::ava::browser::OPEN.into()),
        "A_AvaMaps" => env.nav_events.push(crate::game::fe::ava::maps::OPEN.into()),
        "A_AvaOptions" => crate::game::fe::ava::options::open(fe, env),
        "A_AvaCredits" => env.nav_events.push("go_to_ava_credits".into()),
        "A_AvaExit" => env.nav_events.push(EXIT_POPUP.into()),
        "A_AvaHubHighlight" => {
            fe.ava.hub.index = param as i32;
            fe.note(format!("hub: item {param} highlighted"));
        }
        _ => {}
    }
}

pub fn main_menu_action(fe: &mut FrontEnd, env: &mut FeEnv, action: &str) {
    if action == MAIN_MENU_BACK {
        fe.note("hub: the main menu's back");
        fe.menu.active = false;
        env.nav_events.push(OPEN.into());
    }
}

pub fn exit(fe: &mut FrontEnd) {
    fe.note("hub: exit asked");
    fe.requests.push(GameRequest::ExitGame);
}

pub fn build(fe: &FrontEnd, cx: &mut Ctx, dp: &str) -> Option<Value> {
    match dp {
        DP_MENU => Some(menu::items_value(cx, &fe.ava.hub.items)),
        DP_INDEX => Some(Value::Number(f64::from(fe.ava.hub.index))),
        _ => None,
    }
}
