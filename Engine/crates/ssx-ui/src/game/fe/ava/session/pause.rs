use crate::avm1::Ctx;
use crate::avm1::value::Value;
use crate::game::fe::pause::POPUP_IS_ACTIVE;
use crate::game::fe::{FeEnv, FrontEnd};
use crate::game::loc::Localization;

pub const PLAYERS: &str = "A_AvaPausePlayers";
pub const LEAVE: &str = "A_AvaPauseLeave";
pub const LEAVE_POPUP: &str = "ava_leave_server";

pub fn build(cx: &mut Ctx, loc: &mut Localization) -> Value {
    let items = [
        (loc.text("TXT_PAUSE_MENU_HEADING"), "Resume"),
        (loc.text("TXT_AVA_PAUSE_PLAYERS"), PLAYERS),
        (loc.text("TXT_PAUSE_MENU_SETTINGS"), "A_GameSettings"),
        (loc.text("TXT_AVA_PAUSE_LEAVE"), LEAVE),
    ];
    let mut out = Vec::new();
    for (text, advance) in items {
        let o = Value::Object(cx.new_object());
        let _ = cx.set_member(&o, "strItem", Value::str(&text));
        let _ = cx.set_member(&o, "bEnabled", Value::Bool(true));
        let _ = cx.set_member(&o, "bFocusable", Value::Bool(false));
        let _ = cx.set_member(&o, "eventAdvance", Value::str(advance));
        out.push(o);
    }
    Value::Object(cx.new_array(out))
}

pub fn action(fe: &mut FrontEnd, env: &mut FeEnv, action: &str) {
    if !fe.network_session {
        return;
    }
    match action {
        PLAYERS => {
            fe.pause.players = true;
            fe.note("pause menu: the Players screen");
            env.nav_events.push(super::players::OPEN.into());
        }
        LEAVE => {
            env.values.set_bool(POPUP_IS_ACTIVE, true);
            let name = crate::game::fe::ava::bounded(
                &fe.ava.session.server,
                ssx_net_proto::name::MAX_SERVER_NAME_CHARS,
                fe.ava.glyphs.as_ref(),
            );
            let body = env.loc.format("TXT_AVA_LEAVE_BODY", &[&name]);
            env.values.set_str(crate::game::fe::popup::POPUP_BODY, body);
            env.nav_events.push(LEAVE_POPUP.into());
        }
        _ => {}
    }
}
