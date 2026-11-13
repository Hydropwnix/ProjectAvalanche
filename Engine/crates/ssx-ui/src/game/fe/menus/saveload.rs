use crate::game::fe::{FeEnv, FrontEnd};
use crate::game::loc::Localization;
use crate::game::values::{PopupButton, UiValue};

pub const SHOW_EVENT: &str = "go_to_saveload_popup";
pub const RESULT_EVENT: &str = "saveload_dialog_result";
pub const BUTTONS: u16 = 0x1BC;
pub const DEFAULT_BUTTON: u16 = 0x1BD;
pub const IMAGE: u16 = 0x1BE;
pub const MESSAGE: u16 = 0x1BF;
pub const TYPE: u16 = 0x1C2;
pub const GAME_TITLE: &str = "GameTitle";
pub const PROFILE_GROUP_NAME: &str = "TXT_PROF_SAVEGROUP_NAME";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadError {
    Corrupt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    Load,
    SaveNew,
    NoSaving,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Dialog {
    pub id: u32,
    pub body: String,
    pub buttons: Vec<String>,
    pub default_button: i32,
}

#[derive(Clone, Debug, Default)]
pub struct SaveLoad {
    pub load_error: Option<LoadError>,
    pub dialog: Option<u32>,
    pub signing_in: Option<i32>,
    pub requests: Vec<Request>,
}

pub fn fill(text: &str, args: &[String]) -> String {
    let mut s = text.as_bytes().to_vec();
    let mut from = 0;
    while let Some(i) = s.iter().skip(from).position(|&b| b == b'%').map(|i| i + from) {
        if i + 2 >= s.len() {
            break;
        }
        let n = usize::from(s[i + 1].wrapping_sub(b'0'));
        let mut at = i;
        if n != 0 && n < 9 && n <= args.len() {
            let a = args[n - 1].as_bytes();
            s.splice(i..i + 3, a.iter().copied());
            at = i + a.len();
        }
        from = at + 1;
    }
    String::from_utf8_lossy(&s).into_owned()
}

fn format(loc: &mut Localization, key: &str, args: &[&str], all_keys: bool) -> String {
    let args: Vec<String> = args
        .iter()
        .map(|a| if all_keys || a.eq_ignore_ascii_case(GAME_TITLE) { loc.text(a) } else { a.to_string() })
        .collect();
    fill(&loc.text(key), &args)
}

pub fn corrupt_saving(loc: &mut Localization) -> Dialog {
    let group = loc.text(PROFILE_GROUP_NAME);
    Dialog {
        id: 0x23,
        body: format(loc, "SL_CorruptSaving", &[GAME_TITLE, &group], false),
        buttons: vec![
            format(loc, "SL_RETRY", &[], true),
            format(loc, "SL_CreateANew", &[GAME_TITLE, &group], false),
            format(loc, "SL_ContinueWithoutSaving", &[], true),
        ],
        default_button: 0,
    }
}

pub fn show(fe: &mut FrontEnd, env: &mut FeEnv, d: Dialog) {
    env.values.set_str(MESSAGE, d.body);
    env.values.set_int(DEFAULT_BUTTON, d.default_button);
    env.values.set_int(TYPE, 1);
    env.values.set_str(IMAGE, "");
    let button = |label: &String| PopupButton { label: label.clone(), event: RESULT_EVENT.into(), event_data: 0 };
    env.values.set(BUTTONS, UiValue::Buttons(d.buttons.iter().take(3).map(button).collect()));
    fe.note(format!("SaveLoad dialog {:#x}: message 0x129DCB5F → A_SaveloadPopup → {SHOW_EVENT}", d.id));
    env.nav_events.push(SHOW_EVENT.into());
    fe.saveload.dialog = Some(d.id);
}

pub fn autosave_notice(loc: &mut Localization) -> Dialog {
    Dialog {
        id: 0xC,
        body: format(loc, "SL_AutoSaveAlwaysOn", &[], true),
        buttons: vec![format(loc, "SL_Okay", &[], true)],
        default_button: 0,
    }
}

fn save_icon(fe: &mut FrontEnd, env: &mut FeEnv, on: bool) {
    fe.busy.set_saving(env, on);
}

pub fn lead_profile_load(fe: &mut FrontEnd, env: &mut FeEnv, device: i32) -> bool {
    fe.saveload.signing_in = Some(device);
    let d = autosave_notice(env.loc);
    show(fe, env, d);
    save_icon(fe, env, true);
    false
}

fn after_notice(fe: &mut FrontEnd, env: &mut FeEnv) {
    match fe.saveload.load_error {
        None => flow_ended(fe, env),
        Some(LoadError::Corrupt) => {
            let d = corrupt_saving(env.loc);
            show(fe, env, d);
        }
    }
}

pub fn answer(fe: &mut FrontEnd, env: &mut FeEnv, index: i32) {
    let Some(id) = fe.saveload.dialog.take() else { return };
    fe.note(format!("SaveLoad dialog {id:#x}: answer {index} (message 0x10AD1002)"));
    match (id, index) {
        (0xC, _) => after_notice(fe, env),
        (0x23, 0) => fe.saveload.requests.push(Request::Load),
        (0x23, 1) => {
            fe.saveload.requests.push(Request::SaveNew);
            flow_ended(fe, env);
        }
        (0x23, 2) => {
            fe.saveload.requests.push(Request::NoSaving);
            flow_ended(fe, env);
        }
        _ => flow_ended(fe, env),
    }
}

pub fn loaded(fe: &mut FrontEnd, env: &mut FeEnv, error: Option<LoadError>) {
    fe.saveload.load_error = error;
    match error {
        None => {
            crate::game::fe::flow::profile_into_sign_in(fe, env);
            flow_ended(fe, env);
        }
        Some(LoadError::Corrupt) => {
            let d = corrupt_saving(env.loc);
            show(fe, env, d);
        }
    }
}

fn flow_ended(fe: &mut FrontEnd, env: &mut FeEnv) {
    save_icon(fe, env, false);
    fe.saveload.load_error = None;
    if let Some(device) = fe.saveload.signing_in.take() {
        crate::game::fe::flow::profile_job_done(fe, env, device);
    }
}

#[cfg(test)]
mod tests {
    use super::fill;

    #[test]
    fn the_formatter_fills_numbered_placeholders() {
        let args = ["SSX".to_string(), "Profile".to_string()];
        assert_eq!(fill("Your %1s %2s is corrupt. A new %2s.", &args), "Your SSX Profile is corrupt. A new Profile.");
        assert_eq!(fill("%3s %0s 5%", &args), "%3s %0s 5%");
        assert_eq!(fill("100%1", &args), "100%1");
        assert_eq!(fill("%1d!", &args), "SSX!");
        assert_eq!(fill("%1s%2s %2s", &args), "SSX%2s Profile");
        assert_eq!(fill("%1s", &["%2s".to_string(), "x".to_string()]), "%2s");
    }

    #[test]
    fn the_autosave_notice_has_one_answer() {
        let mut loc = crate::game::loc::Localization::empty(crate::game::loc::Language::English);
        let d = super::autosave_notice(&mut loc);
        assert_eq!((d.id, d.default_button), (0xC, 0));
        assert_eq!(d.buttons, vec![loc.text("SL_Okay")]);
        assert_eq!(d.body, loc.text("SL_AutoSaveAlwaysOn"));
    }
}
