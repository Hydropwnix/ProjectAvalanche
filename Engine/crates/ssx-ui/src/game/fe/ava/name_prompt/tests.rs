use super::*;

fn due() -> NamePrompt {
    NamePrompt { stage: Stage::Due }
}

#[test]
fn unset_asks_once_the_menu_is_idle() {
    let mut p = due();
    assert!(p.waiting());
    assert_eq!(p.step(false, false, None), Do::Nothing, "the menu not idle yet");
    assert_eq!(p.step(true, false, None), Do::Read);
    assert_eq!(p.found(None), Do::Ask(String::new()));
    assert_eq!(p.stage, Stage::Asking);
    assert!(p.waiting());
    assert_eq!(p.step(true, false, None), Do::Nothing, "no answer yet: nothing asked twice");
}

#[test]
fn a_stored_name_or_an_unarmed_prompt_asks_nothing() {
    let mut p = due();
    assert_eq!(p.step(true, false, None), Do::Read);
    assert_eq!(p.found(Some("Elise".into())), Do::Nothing);
    assert_eq!((p.stage.clone(), p.waiting()), (Stage::Named("Elise".into()), false));
    let mut off = NamePrompt::default();
    assert!(!off.waiting());
    assert_eq!(off.step(true, false, Some(Outcome::Done("Zoe".into()))), Do::Nothing);
    assert_eq!(off.stage, Stage::Off);
}

#[test]
fn a_confirmed_name_is_saved() {
    let mut p = NamePrompt { stage: Stage::Asking };
    assert_eq!(p.step(true, false, Some(Outcome::Done("  Zoe   Payne ".into()))), Do::Save("Zoe Payne".into()));
    assert_eq!((p.stage.clone(), p.waiting()), (Stage::Named("Zoe Payne".into()), false));
    assert_eq!(p.step(true, false, None), Do::Nothing, "answered: nothing more");
}

#[test]
fn a_cancel_saves_nothing() {
    let mut p = NamePrompt { stage: Stage::Asking };
    assert_eq!(p.step(true, false, Some(Outcome::Cancelled)), Do::Nothing);
    assert_eq!((p.stage.clone(), p.waiting()), (Stage::Cancelled, false));
    assert_eq!(name_for_join(None, None), DEFAULT_PLAYER_NAME);
}

#[test]
fn a_refused_name_shows_the_refusal_then_asks_again() {
    let mut p = NamePrompt { stage: Stage::Asking };
    let why = ssx_net_proto::name::player_name("   ").unwrap_err().to_string();
    assert_eq!(p.step(true, false, Some(Outcome::Done("   ".into()))), Do::Refuse(why));
    assert!(p.waiting());
    assert_eq!(p.step(true, false, None), Do::Nothing, "the popup not seen yet: not asked over it");
    assert_eq!(p.step(false, true, None), Do::Nothing);
    assert_eq!(p.stage, Stage::Refused { text: "   ".into(), shown: true });
    assert_eq!(p.step(false, false, None), Do::Nothing, "gone, but the menu not idle");
    assert_eq!(p.step(true, false, None), Do::Ask("   ".into()));
    assert_eq!(p.stage, Stage::Asking);
    let long = "W".repeat(ssx_net_proto::name::MAX_PLAYER_NAME_CHARS + 1);
    assert!(matches!(p.step(true, false, Some(Outcome::Done(long))), Do::Refuse(_)), "too long");
    let mut p = NamePrompt { stage: Stage::Asking };
    assert!(matches!(p.step(true, false, Some(Outcome::Done("a\u{7}b".into()))), Do::Refuse(_)), "a control");
}

#[test]
fn a_join_takes_the_confirmed_name_then_the_files() {
    assert_eq!(name_for_join(Some("Zoe"), Some("Elise")), "Zoe");
    assert_eq!(name_for_join(Some("Zoe"), None), "Zoe");
    assert_eq!(name_for_join(None, Some("Elise")), "Elise");
    assert_eq!(name_for_join(None, None), DEFAULT_PLAYER_NAME);
}

#[test]
fn the_popups_events_are_the_menus_alone() {
    let overlay: serde_json::Value =
        serde_json::from_str(include_str!("../../../../../../../assets/fe/nav/overlay_net.json")).expect("overlay");
    let transitions = overlay["transitions"].as_array().expect("transitions");
    let hub = |event: &str| {
        transitions.iter().filter(|t| t["transition"]["event"] == event).map(|t| t["state"].clone()).collect::<Vec<_>>()
    };
    for popup in [REFUSED, BROKEN] {
        assert_eq!(hub(popup), [serde_json::json!("AvaHub")], "{popup}");
    }
    let events = [
        ("ava_name_refused_ok", include_str!("../../../../../../../assets/fe/popups/ava_name_refused.pop")),
        (
            "ava_name_settings_broken_yes",
            include_str!("../../../../../../../assets/fe/popups/ava_name_settings_broken.pop"),
        ),
        (
            "ava_name_settings_broken_no",
            include_str!("../../../../../../../assets/fe/popups/ava_name_settings_broken.pop"),
        ),
    ];
    for (event, pop) in events {
        assert!(pop.contains(&format!("\"event\": \"{event}\"")), "{event}");
        assert_eq!(hub(event), [serde_json::json!("AvaHub")], "{event}");
    }
    let base: serde_json::Value =
        serde_json::from_str(include_str!("../../../../../../../assets/fe/nav/overlay.json")).expect("overlay");
    let text = base.to_string();
    for event in [REFUSED, BROKEN, "ava_name_refused_ok", "ava_name_settings_broken_yes"] {
        assert!(!text.contains(&format!("\"{event}\"")), "{event} only in the net overlay");
    }
    let strings: std::collections::BTreeMap<String, String> =
        serde_json::from_str(include_str!("../../../../../../../assets/fe/strings/eng_us.json")).expect("strings");
    for k in [TITLE, NOTE, "TXT_AVA_PLAYER_NAME_REFUSED", "TXT_AVA_OPTIONS_PLAYER_NAME_TITLE"] {
        assert!(strings.contains_key(k), "{k}");
    }
}

#[test]
fn the_profile_name_is_written_where_the_dock_sees_it() {
    let mut v = crate::game::values::UiValues::new();
    v.set_str(USER_GAMERTAG, "Player");
    v.clear_dirty();
    assert!(!v.is_dirty(USER_GAMERTAG));
    v.set_str(USER_GAMERTAG, "Elise");
    assert!(v.is_dirty(USER_GAMERTAG) && v.str(USER_GAMERTAG) == "Elise");
    v.clear_dirty();
    assert!(!v.is_dirty(USER_GAMERTAG), "a write inside the update is not seen by the next one");
    let named = crate::game::value_names::UI_VALUE_NAMES.iter().find(|(id, _)| *id == USER_GAMERTAG);
    assert_eq!(named.map(|n| n.1), Some("USER_gamertag"));
}
