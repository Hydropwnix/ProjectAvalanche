use std::collections::VecDeque;

use crate::avm1::Ctx;
use crate::avm1::value::Value;
use crate::game::fe::pause::reason;
use crate::game::fe::{ActionParam, FeEnv, FeMessage, FrontEnd};
use crate::game::loc::Localization;

pub const DP_POPUP: &str = "dp_tutorial_popup";
pub const DP_MENU: &str = "dp_tutorial_pause_menu_data";
pub const DP_INDEX: &str = "dp_tutorial_pause_menu_index";
pub const MENU_TYPE: u16 = 0x219;
pub const MESSAGE: u16 = 0x21A;
pub const LOWER_MESSAGE: u16 = 0x21B;
pub const TITLE: u16 = 0x21C;
const HUD_MESSAGE_VISIBLE: u16 = 0xD1;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Popup {
    pub title: String,
    pub message: String,
    pub lower: String,
    pub menu_type: i32,
    pub enabled: bool,
}

#[derive(Clone, Debug, Default)]
pub struct TutorialPause {
    pub active: bool,
    pub queue: VecDeque<Popup>,
    pub current: Option<Popup>,
    pub shown: bool,
    pub hidden: bool,
    pub controller: i32,
    pub input_choice: bool,
}

fn popup_show(fe: &mut FrontEnd, env: &mut FeEnv, title: &str, message: &str, lower: &str) {
    if !fe.game_pause.asked(reason::TUTORIAL_POPUP) {
        fe.game_pause.set(reason::TUTORIAL_POPUP, true);
        fe.note("tutorial popup: message 0x9671760E (pause reason 0xB)");
    }
    env.values.set_bool(HUD_MESSAGE_VISIBLE, false);
    env.values.set_str(TITLE, title);
    env.values.set_str(MESSAGE, message);
    env.values.set_str(LOWER_MESSAGE, lower);
}

pub fn show(fe: &mut FrontEnd, env: &mut FeEnv, key: &str, message: &str) {
    let title = env.loc.text("TXT_TUTORIAL_POPUP_TITLE");
    popup_show(fe, env, &title, message, "");
    env.values.set_int(MENU_TYPE, 0);
    fe.note(format!("tutorial popup {key}: message 0xFCCD4EB8 {{0}}"));
    fe.popup_message(0);
}

pub fn show_input_choice(fe: &mut FrontEnd, env: &mut FeEnv) {
    fe.tutorial_pause.controller = 0;
    let title = env.loc.text("TXT_TUTORIAL_RACE_HELP_1");
    let message = env.loc.text("TXT_TUTORIAL_INPUT_CHOICE");
    let lower = env.loc.text("TXT_TUTORIAL_INPUT_TIP_CLASSIC");
    fe.tutorial_pause.input_choice = true;
    popup_show(fe, env, &title, &message, &lower);
    env.values.set_int(MENU_TYPE, 1);
    fe.note("tutorial input choice (0x824D62B8): the controller type 0, the popup pending");
}

pub fn activate(fe: &mut FrontEnd) {
    let (controller, input_choice) = (fe.tutorial_pause.controller, fe.tutorial_pause.input_choice);
    fe.tutorial_pause = TutorialPause { active: true, controller, input_choice, ..Default::default() };
    publish(fe);
}

pub fn deactivate(fe: &mut FrontEnd) {
    fe.tutorial_pause.active = false;
}

fn publish(fe: &mut FrontEnd) {
    for dp in [DP_INDEX, DP_MENU, DP_POPUP] {
        fe.publish.push(dp.into());
    }
}

fn fe_event(fe: &mut FrontEnd, event: u32) {
    let none = FeMessage::PauseMenuAudio { opened: false, leaving: false, closed: false };
    match event {
        0x12 => fe.messages.extend([FeMessage::PopupState(2), none]),
        0x13 => fe.messages.extend([FeMessage::PopupState(3), none]),
        _ => {
            if fe.game_pause.paused && fe.game_pause.active[reason::TUTORIAL_POPUP] {
                fe.game_pause.set(reason::TUTORIAL_POPUP, false);
                fe.note("tutorial popup: FE manager event 0x14 → message 0x389CAC7F (pause reason 0xB off)");
            }
        }
    }
}

fn enqueue(fe: &mut FrontEnd, env: &mut FeEnv) {
    let v = &env.values;
    let popup = Popup {
        title: v.str(TITLE).to_string(),
        message: v.str(MESSAGE).to_string(),
        lower: v.str(LOWER_MESSAGE).to_string(),
        menu_type: v.int(MENU_TYPE),
        enabled: true,
    };
    fe.tutorial_pause.queue.push_back(popup);
}

fn show_next(fe: &mut FrontEnd) {
    if fe.tutorial_pause.shown {
        return;
    }
    match fe.tutorial_pause.queue.pop_front() {
        None => {
            fe.note("tutorial popup: none left — message 0x7B2339FD {7}, FE manager events 0x14, 0x13");
            fe.messages.push(FeMessage::HintClosed);
            fe.tutorial_pause.current = None;
            publish(fe);
            fe_event(fe, 0x14);
            fe_event(fe, 0x13);
        }
        Some(p) => {
            fe.note(format!("tutorial popup shown: {}", p.message));
            fe.tutorial_pause.current = Some(p);
            publish(fe);
            fe_event(fe, 0x12);
            fe.tutorial_pause.shown = true;
        }
    }
}

fn hide(fe: &mut FrontEnd) {
    if let Some(p) = fe.tutorial_pause.current.as_mut() {
        p.enabled = false;
    }
    fe.publish.push(DP_POPUP.into());
}

pub fn action(fe: &mut FrontEnd, env: &mut FeEnv, action: &str, param: &ActionParam) {
    match action {
        "A_GameTutorialClose" => fe.tutorial_pause.shown = false,
        "A_GameTutorialDigital" | "A_GameTutorialAnalog" => {
            fe.tutorial_pause.shown = false;
            fe.tutorial_pause.controller = if action == "A_GameTutorialDigital" { 1 } else { 2 };
            fe.messages.push(FeMessage::HintClosed);
            hide(fe);
            fe_event(fe, 0x14);
            return;
        }
        "A_GameTutorialOnlineDisconnected" => {
            let tp = &mut fe.tutorial_pause;
            (tp.shown, tp.hidden, tp.current) = (false, false, None);
            tp.queue.clear();
            publish(fe);
            fe_event(fe, 0x14);
            return;
        }
        "A_GamePopupTutorial" | "PopupTutorial" => {
            if param.number as i32 == 1 {
                return;
            }
            enqueue(fe, env);
        }
        _ => return,
    }
    show_next(fe);
}

pub fn update(fe: &mut FrontEnd, env: &mut FeEnv, provider: bool) {
    if fe.tutorial_pause.input_choice && fe.tutorial_ui {
        fe.tutorial_pause.input_choice = false;
        fe.note("tutorial input choice: message 0xFCCD4EB8 {2}");
        fe.popup_message(2);
    }
    if !provider || !fe.tutorial_pause.shown {
        return;
    }
    if !fe.tutorial_pause.hidden && env.values.bool(crate::game::fe::pause::PAUSE_IS_IN_PAUSE_MENU) {
        hide(fe);
        fe.tutorial_pause.hidden = true;
    } else if env.values.bool(crate::game::fe::pause::PAUSE_IS_EXITING_BE) {
        let tp = &mut fe.tutorial_pause;
        (tp.shown, tp.hidden, tp.current) = (false, false, None);
        tp.queue.clear();
        env.values.set_bool(crate::game::fe::pause::PAUSE_IS_EXITING_BE, false);
        fe.messages.push(FeMessage::HintClosed);
        publish(fe);
    } else if fe.tutorial_pause.hidden && !env.values.bool(crate::game::fe::pause::PAUSE_IS_IN_PAUSE_MENU) {
        if let Some(p) = fe.tutorial_pause.current.as_mut() {
            p.enabled = true;
        }
        publish(fe);
        fe.tutorial_pause.hidden = false;
    }
}

pub fn build(fe: &FrontEnd, cx: &mut Ctx, loc: &mut Localization, dp: &str) -> Option<Value> {
    let menu = |cx: &mut Ctx, loc: &mut Localization, p: Option<&Popup>| {
        let items: Vec<(String, &str)> = match p {
            None => Vec::new(),
            Some(p) if p.menu_type == 1 => {
                let analog = loc.text("TXT_TUTORIAL_ANALOG");
                let digital = loc.text("TXT_TUTORIAL_DIGITAL");
                vec![(analog, "TutorialAnalog"), (digital, "TutorialDigital")]
            }
            Some(_) => vec![(loc.text("TXT_CONTINUE"), "TutorialClose")],
        };
        let out: Vec<Value> = items
            .into_iter()
            .map(|(text, advance)| {
                let o = Value::Object(cx.new_object());
                let _ = cx.set_member(&o, "strItem", Value::str(&text));
                let _ = cx.set_member(&o, "bEnabled", Value::Bool(true));
                let _ = cx.set_member(&o, "bFocusable", Value::Bool(false));
                let _ = cx.set_member(&o, "eventAdvance", Value::str(advance));
                o
            })
            .collect();
        Value::Object(cx.new_array(out))
    };
    let current = fe.tutorial_pause.current.as_ref();
    match dp {
        DP_INDEX => Some(Value::Number(0.0)),
        DP_MENU => Some(menu(cx, loc, current)),
        DP_POPUP => {
            let o = Value::Object(cx.new_object());
            let m = menu(cx, loc, current);
            let _ = cx.set_member(&o, "menu", m);
            if let Some(p) = current {
                let continue_text = loc.text("TXT_CONTINUE");
                for (k, v) in [
                    ("isEnabled", Value::Bool(p.enabled)),
                    ("title", Value::str(&p.title)),
                    ("message", Value::str(&p.message)),
                    ("lowerMessage", Value::str(&p.lower)),
                    ("menutype", Value::Number(f64::from(p.menu_type))),
                    ("continueString", Value::str(&continue_text)),
                ] {
                    let _ = cx.set_member(&o, k, v);
                }
            }
            Some(o)
        }
        _ => None,
    }
}
