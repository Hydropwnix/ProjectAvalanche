use crate::avm1::Ctx;
use crate::avm1::value::Value;
use crate::game::fe::{FeEnv, FrontEnd};
use crate::game::loc::Localization;
use crate::game::values::UiValues;

#[derive(Clone, Debug, Default)]
pub struct UserData {
    pub drop_info_shown: bool,
    pub adjust: i64,
}

pub const DP_DIMMER: &str = "dp_dimmer_state";

#[derive(Clone, Debug, Default)]
pub struct Dimmer {
    pub state: String,
    pub action: i32,
    pub queue: std::collections::VecDeque<(i32, String)>,
}

impl Dimmer {
    pub fn nav(&mut self, name: &str, arg: &str) {
        let entry = match name {
            "set_dimmer_state" => (0, arg.to_string()),
            "push_dimmer_state" => (1, arg.to_string()),
            _ => (2, String::new()),
        };
        self.queue.push_back(entry);
    }

    pub fn drain(&mut self) -> Vec<(i32, String)> {
        let out: Vec<(i32, String)> = self.queue.drain(..).collect();
        if let Some((a, s)) = out.last() {
            self.action = *a;
            self.state = s.clone();
        }
        out
    }
}

pub fn dimmer_object(cx: &mut Ctx, action: i32, state: &str) -> Value {
    let o = Value::Object(cx.new_object());
    let _ = cx.set_member(&o, "action", Value::Number(f64::from(action)));
    let _ = cx.set_member(&o, "state", Value::str(state));
    o
}

pub fn update(fe: &mut FrontEnd, env: &mut FeEnv) {
    let v = &mut *env.values;
    let adjust = v.int(0x93);
    if [0x228u16, 0x22B, 0x22F].iter().any(|&i| v.is_dirty(i)) || adjust != 0 {
        fe.publish.push("dp_user_data".into());
        fe.user.adjust = i64::from(adjust);
        if adjust != 0 {
            let bank = v.int(0x228);
            v.set_int(0x228, bank + adjust);
            v.set_int(0x93, 0);
        }
    }
}

pub fn activate_feed(fe: &mut FrontEnd, _env: &mut FeEnv) {
    fe.publish.push("dp_welcome_screen_data".into());
}

pub fn feed_action(fe: &mut FrontEnd, env: &mut FeEnv, action: &str) {
    if action == "A_GameTriggerAnimation" && fe.screen == crate::game::fe::screens::id::WELCOME_HUB {
        fe.note("FEGlobeUiTransRequestMessage (0x628144DE)");
        // TODO(guess): the globe transition's end (0x824F2210) bringing up the main menu is not traced; the port does the main-menu request's (0x8259BBD8) transition and screen 2 at once; settle: that receiver.
        env.values.set_int(0x11, 1);
        fe.go_to_screen(env, crate::game::fe::screens::id::MAIN_MENU);
    }
}

pub fn build(fe: &mut FrontEnd, cx: &mut Ctx, values: &UiValues, loc: &mut Localization, dp: &str) -> Option<Value> {
    let o = Value::Object(cx.new_object());
    if dp == "dp_drop_data" {
        let _ = cx.set_member(&o, "dropSelected", Value::Bool(fe.user.drop_info_shown));
        if fe.user.drop_info_shown {
            let panel = values.panel(crate::game::fe::globe::panel::DROP_PANEL).cloned().unwrap_or_default();
            let p = crate::game::fe::globe::panel::object(cx, &panel);
            let _ = cx.set_member(&o, "dropPanel", p);
        }
        return Some(o);
    }
    let adjust = fe.user.adjust;
    let bank = i64::from(values.int(0x228)) - adjust;
    let status = values.int(0x22F);
    for (k, v) in [
        ("imageurl", Value::str(values.str(0x22C))),
        ("username", Value::str(values.str(0x22B))),
        ("bankBalance", Value::str(&loc.currency(bank, true))),
        ("bankAdjustment", Value::str(&loc.currency(adjust, true))),
        ("newBankBalance", Value::str(&loc.currency(bank + adjust, true))),
        ("isBankAdjustmentPositive", Value::Bool(adjust >= 0)),
        ("isBankAdjustmentEnabled", Value::Bool(adjust != 0)),
        ("onlinestatus", Value::str(&loc.text(&format!("TXT_ONLINE_STATUS_{status}")))),
    ] {
        let _ = cx.set_member(&o, k, v);
    }
    Some(o)
}

pub fn build_offline(_fe: &mut FrontEnd, cx: &mut Ctx, loc: &mut Localization, dp: &str) -> Option<Value> {
    match dp {
        // TODO(guess): the panel objects' fields (the builder after the list, 0x826164F8…) are not decompiled; the port sends the fields the view reads; settle: that builder.
        "dp_welcome_screen_data" => {
            let o = Value::Object(cx.new_object());
            let _ = cx.set_member(&o, "iPanelType", Value::Number(4.0));
            let _ = cx.set_member(&o, "eventAdvance", Value::str("A_GameTriggerAnimation"));
            Some(o)
        }
        // TODO(guess): `dp_online_pass_enabled`'s fields are taken from what the main menu's and the welcome hub's `Publish` read; settle: the provider's custom object.
        _ => {
            let o = Value::Object(cx.new_object());
            for k in ["showOnlinePassButton", "showRedeemCodeButton", "showConnectToServerButton"] {
                let _ = cx.set_member(&o, k, Value::Bool(false));
            }
            for (k, key) in [
                ("getPassLabel", "TXT_ONLINE_PASS_GET_PASS"),
                ("redeemCodeLabel", "TXT_ONLINE_PASS_REDEEM_CODE"),
                ("connectToServerLabel", "TXT_CONNECT_EA_SERVER"),
                ("earningsLabel", "TXT_ONLINE_PASS_LE_EARNINGS_LABEL"),
            ] {
                let _ = cx.set_member(&o, k, Value::str(&loc.text(key)));
            }
            Some(o)
        }
    }
}
