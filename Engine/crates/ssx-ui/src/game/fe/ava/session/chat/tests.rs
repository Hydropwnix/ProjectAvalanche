use std::collections::{BTreeSet, VecDeque};

use super::*;

fn line(name: &str, text: &str, from: Option<u8>, at: f64) -> Line {
    Line { name: name.into(), text: text.into(), from: from.map(|b| [b; 8]), at }
}

#[test]
fn rows_are_the_newest_lines_of_players_not_muted_while_young() {
    let mut lines = VecDeque::new();
    for k in 0..8u8 {
        lines.push_back(line(&format!("P{k}"), &format!("line {k}"), Some(k % 3), f64::from(k)));
    }
    lines.push_back(line("Server", "restart soon", None, 8.0));
    let none = BTreeSet::new();
    let names = |rows: &[Row]| rows.iter().map(|r| r.name.clone()).collect::<Vec<_>>();
    let rows = visible_rows(&lines, &none, false, 9.0);
    assert_eq!(names(&rows), ["P3", "P4", "P5", "P6", "P7", "Server"], "the newest six, oldest first");
    let muted: BTreeSet<[u8; 8]> = [[1u8; 8]].into();
    assert_eq!(names(&visible_rows(&lines, &muted, false, 9.0)), ["P0", "P2", "P3", "P5", "P6", "Server"]);
    let rows = visible_rows(&lines, &none, false, 15.5);
    assert_eq!(rows.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), vec!["line 6", "line 7", "restart soon"]);
    assert!(rows.iter().all(|r| !r.notice), "the players' and the server's lines are no notices");
    assert!(visible_rows(&lines, &none, false, 100.0).is_empty());
    assert_eq!(visible_rows(&lines, &none, true, 100.0).len(), ROWS);
}

#[test]
fn every_quick_line_has_its_string() {
    assert_eq!(QUICK_KEYS.len(), usize::from(QUICK_LINES));
    let strings: std::collections::BTreeMap<String, String> =
        serde_json::from_str(include_str!("../../../../../../../../assets/fe/strings/eng_us.json")).expect("strings");
    for k in QUICK_KEYS.iter().chain(&["TXT_AVA_QUICK_TYPE", "TXT_AVA_QUICK_TITLE"]) {
        let text = strings.get(*k).unwrap_or_else(|| panic!("{k}"));
        assert!(chat_text(text).is_ok(), "{k}: {text:?} keeps the wire's rule");
    }
}

#[test]
fn a_notice_names_the_mods_switched_off() {
    let names: Vec<String> = ["speed", "<b>x</b>", "c", "d", "e"].iter().map(|s| s.to_string()).collect();
    let shown = |n: &str| crate::game::fe::ava::bounded(n, MOD_NAME_CHARS, None);
    let wrap = crate::text::isolate::wrap;
    assert_eq!(mods_named(&names[..1], shown, None), wrap("speed"));
    let listed = format!("{}, {}, {}, +2 more", wrap("speed"), wrap("?b?x?/b?"), wrap("c"));
    assert_eq!(mods_named(&names, shown, Some("+2 more".into())), listed);
    let long = "m".repeat(64);
    assert_eq!(mods_named(&[long], shown, None).chars().count(), MOD_NAME_CHARS + 2);
    let strings: std::collections::BTreeMap<String, String> =
        serde_json::from_str(include_str!("../../../../../../../../assets/fe/strings/eng_us.json")).expect("strings");
    for k in ["TXT_AVA_CHAT_MODS_OFF", "TXT_AVA_CHAT_MODS_SIM_OFF", "TXT_AVA_CHAT_MODS_MORE"] {
        let text = strings.get(k).unwrap_or_else(|| panic!("{k}"));
        assert_eq!(text.matches("%1s").count(), 1, "{k}: {text:?}");
        assert!(!text.contains("%2"), "{k}: {text:?}");
    }
}

#[test]
fn the_chat_layer_is_on_its_own_layer() {
    let layout = crate::author::Layout::parse(
        "ava_chat.json",
        include_str!("../../../../../../../../assets/fe/views/ava_chat.json"),
    )
    .expect("the layout");
    assert_eq!(layout.layer, LAYER);
}

#[test]
fn the_launchs_notice_is_said_once_when_the_level_shows_the_layer() {
    let mut s = crate::game::fe::ava::session::Session::default();
    s.begin("loopback", Some(4));
    s.launch_notice = Some("a 4% chance of survival".into());
    let open = false;
    assert!(visible_rows(&s.chat.lines, &s.muted, open, 12.0).is_empty(), "nothing before the level shows it");
    level_shown(&mut s, 30.0);
    let rows = visible_rows(&s.chat.lines, &s.muted, open, 39.5);
    let notice = Row { name: String::new(), text: "a 4% chance of survival".into(), notice: true };
    assert_eq!(rows, [notice], "a notice row, no sender");
    assert!(visible_rows(&s.chat.lines, &s.muted, open, 40.5).is_empty(), "gone after its ten seconds");
    level_shown(&mut s, 60.0);
    assert_eq!(s.chat.lines.len(), 1, "said once");
    s.launch_notice = Some("again".into());
    s.end();
    assert_eq!(s.launch_notice, None, "a session's end drops it");
}
