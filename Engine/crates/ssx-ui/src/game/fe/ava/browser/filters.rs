use ssx_net_join::browse::{Filters, SortKey};

use crate::avm1::Ctx;
use crate::avm1::value::Value;
use crate::game::fe::{ActionParam, FeEnv, FrontEnd};
use crate::game::loc::Localization;

pub const DP_INIT: &str = "dp_ava_bf_init";
pub const DP_COMMIT: &str = "dp_ava_bf_commit";
pub const OPEN: &str = "go_to_ava_browser_filters";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    NotFull,
    NotEmpty,
    NoPassword,
    Mode,
    Compatible,
    Sort,
    Order,
}

pub const ITEMS: [Item; 7] =
    [Item::NotFull, Item::NotEmpty, Item::NoPassword, Item::Mode, Item::Compatible, Item::Sort, Item::Order];
const MODES: [Option<&str>; 5] = [None, Some("freeride"), Some("race"), Some("trick"), Some("survive")];
const SORTS: [SortKey; 5] = [SortKey::Name, SortKey::Players, SortKey::Ping, SortKey::Mode, SortKey::Drop];

impl Item {
    pub fn name(self) -> &'static str {
        match self {
            Item::NotFull => "ava_bf_not_full",
            Item::NotEmpty => "ava_bf_not_empty",
            Item::NoPassword => "ava_bf_no_password",
            Item::Mode => "ava_bf_mode",
            Item::Compatible => "ava_bf_compatible",
            Item::Sort => "ava_bf_sort",
            Item::Order => "ava_bf_order",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Item::NotFull => "TXT_AVA_BF_NOT_FULL",
            Item::NotEmpty => "TXT_AVA_BF_NOT_EMPTY",
            Item::NoPassword => "TXT_AVA_BF_NO_PASSWORD",
            Item::Mode => "TXT_AVA_BF_MODE",
            Item::Compatible => "TXT_AVA_BF_COMPATIBLE",
            Item::Sort => "TXT_AVA_BF_SORT",
            Item::Order => "TXT_AVA_BF_ORDER",
        }
    }

    fn of_dp(dp: &str) -> Option<Item> {
        ITEMS.into_iter().find(|i| dp.strip_prefix("dp_") == Some(i.name()))
    }

    fn label(self, f: &Filters, loc: &mut Localization) -> String {
        let mut on_off = |b: bool| loc.text(if b { "TXT_ON" } else { "TXT_OFF" });
        match self {
            Item::NotFull => on_off(f.not_full),
            Item::NotEmpty => on_off(f.not_empty),
            Item::NoPassword => on_off(f.no_password),
            Item::Compatible => on_off(f.compatible),
            Item::Mode => match f.mode.as_deref() {
                None => loc.text("TXT_AVA_BF_ANY"),
                Some(m) => super::rows::mode_name(loc, m),
            },
            Item::Sort => loc.text(match f.sort {
                SortKey::Name => "TXT_AVA_BROWSER_COL_NAME",
                SortKey::Players => "TXT_AVA_BROWSER_COL_PLAYERS",
                SortKey::Ping => "TXT_AVA_BROWSER_COL_PING",
                SortKey::Mode => "TXT_AVA_BROWSER_COL_MODE",
                SortKey::Drop => "TXT_AVA_BROWSER_COL_DROP",
            }),
            Item::Order => {
                loc.text(if f.descending { "TXT_AVA_BROWSER_DESCENDING" } else { "TXT_AVA_BROWSER_ASCENDING" })
            }
        }
    }

    fn step(self, f: &mut Filters, next: bool) {
        fn wrap<T: Clone + PartialEq>(list: &[T], cur: &T, next: bool) -> T {
            let i = list.iter().position(|x| x == cur).unwrap_or(0);
            let n = list.len();
            list[if next { (i + 1) % n } else { (i + n - 1) % n }].clone()
        }
        match self {
            Item::NotFull => f.not_full = !f.not_full,
            Item::NotEmpty => f.not_empty = !f.not_empty,
            Item::NoPassword => f.no_password = !f.no_password,
            Item::Compatible => f.compatible = !f.compatible,
            Item::Order => f.descending = !f.descending,
            Item::Mode => f.mode = wrap(&MODES, &f.mode.as_deref(), next).map(str::to_string),
            Item::Sort => f.sort = wrap(&SORTS, &f.sort, next),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Page {
    pub filters: Filters,
    pub changed: bool,
}

pub fn open(fe: &mut FrontEnd, env: &mut FeEnv) {
    fe.ava.filters = Page { filters: fe.ava.browser.filters.clone(), changed: false };
    env.nav_events.push(OPEN.into());
}

pub fn activate(fe: &mut FrontEnd) {
    fe.publish.extend([DP_INIT.into(), DP_COMMIT.into()]);
    fe.publish.extend(ITEMS.iter().map(|i| format!("dp_{}", i.name())));
}

fn close(fe: &mut FrontEnd, env: &mut FeEnv, apply: bool) {
    if apply && fe.ava.filters.filters != fe.ava.browser.filters {
        fe.ava.browser.filters = fe.ava.filters.filters.clone();
        fe.ava.browser.dirty = true;
        fe.ava.browser.restart = true;
        fe.note(format!("browser: filters {:?}", fe.ava.browser.filters));
    }
    env.nav_events.push("ava_browser_filters_close".into());
}

pub fn data_action(fe: &mut FrontEnd, env: &mut FeEnv, dp: &str, action: &str, _param: &ActionParam) {
    if dp == DP_COMMIT {
        match action {
            "A_GameAdvanceScreen" => close(fe, env, true),
            "A_GamePreviousScreen" if fe.ava.filters.changed => {
                env.nav_events.push("go_to_exit_settings_without_save_popup".into());
            }
            "A_GamePreviousScreen" => close(fe, env, false),
            "A_SettingsResetToDefault" => env.nav_events.push("ava_options_reset".into()),
            _ => {}
        }
        return;
    }
    let next = match action {
        "A_TogglePreviousItem" => false,
        "A_ToggleNextItem" => true,
        _ => return,
    };
    let Some(item) = Item::of_dp(dp) else { return };
    item.step(&mut fe.ava.filters.filters, next);
    fe.ava.filters.changed = true;
    fe.play_sound(7);
    fe.publish.push(format!("dp_{}", item.name()));
}

pub fn nav_action(fe: &mut FrontEnd, env: &mut FeEnv, name: &str) -> bool {
    match name {
        "do_ava_browser_filters_discard" => close(fe, env, false),
        "do_ava_browser_filters_reset" => {
            fe.ava.filters.filters = Filters::default();
            fe.ava.filters.changed = true;
            fe.publish.extend(ITEMS.iter().map(|i| format!("dp_{}", i.name())));
        }
        _ => return false,
    }
    true
}

pub fn build(fe: &FrontEnd, cx: &mut Ctx, loc: &mut Localization, dp: &str) -> Option<Value> {
    let f = &fe.ava.filters.filters;
    match dp {
        DP_INIT => {
            let v = Value::Object(cx.new_object());
            let _ = cx.set_member(&v, "doneButtonLabel", Value::str(&loc.text("TXT_AVA_BF_APPLY")));
            let _ = cx.set_member(&v, "isFromPauseMenu", Value::Bool(false));
            let names: Vec<Value> = ITEMS.iter().map(|i| Value::str(i.name())).collect();
            let names = Value::Object(cx.new_array(names));
            let _ = cx.set_member(&v, "settingsItems", names);
            let _ = cx.set_member(&v, "showDescription", Value::Bool(true));
            let _ = cx.set_member(&v, "descriptionText", Value::str(&loc.text("TXT_AVA_BF_DESCRIPTION")));
            Some(v)
        }
        DP_COMMIT => Some(Value::Object(cx.new_object())),
        _ => {
            let item = Item::of_dp(dp)?;
            let v = Value::Object(cx.new_object());
            let _ = cx.set_member(&v, "label", Value::str(&item.label(f, loc)));
            let _ = cx.set_member(&v, "bEnabled", Value::Bool(true));
            let _ = cx.set_member(&v, "title", Value::str(&loc.text(item.title())));
            Some(v)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_wrap_and_switches_toggle() {
        let mut f = Filters::default();
        Item::Mode.step(&mut f, false);
        assert_eq!(f.mode.as_deref(), Some("survive"), "any wraps to the last");
        Item::Mode.step(&mut f, true);
        assert_eq!(f.mode, None);
        Item::Sort.step(&mut f, true);
        assert_eq!(f.sort, SortKey::Players);
        Item::NotFull.step(&mut f, true);
        Item::Order.step(&mut f, false);
        assert!(f.not_full && f.descending);
        for i in ITEMS {
            assert_eq!(Item::of_dp(&format!("dp_{}", i.name())), Some(i));
        }
    }
}
