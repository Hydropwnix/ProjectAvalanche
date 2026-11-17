use std::time::{Duration, Instant};

use serde_json::Value as Json;

use super::super::port_settings::read_file;
use super::*;

fn folder(tag: &str, text: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("ava-settings-writer-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("settings.json");
    std::fs::write(&path, text).unwrap();
    (dir, path)
}

fn kept(path: &Path) -> PortSettings {
    PortSettings::from_json(&Json::Object(read_file(path).unwrap())).0
}

fn answers(w: &SettingsWriter) -> Vec<Done> {
    let start = Instant::now();
    while w.writing() {
        assert!(start.elapsed() < Duration::from_secs(5), "the writer did not answer in 5 s");
        std::thread::sleep(Duration::from_millis(5));
    }
    w.take_done()
}

#[test]
fn a_save_is_written_by_the_writer_and_answered() {
    let (dir, path) = folder("save", r#"{"iso": "/games/ssx.iso", "vsync": true}"#);
    let base = kept(&path);
    let w = SettingsWriter::start(path.clone()).unwrap();
    let s = PortSettings { vsync: false, ..base.clone() };
    assert!(w.ask(Ask::Save { base, settings: s.clone(), read: 1, popup: "p".into() }));
    let done = answers(&w);
    assert!(matches!(&done[..], [Done::Saved { settings, read: 1, .. }] if *settings == s), "{done:?}");
    let m = read_file(&path).unwrap();
    assert_eq!((&m["iso"], &m["vsync"]), (&Json::from("/games/ssx.iso"), &Json::Bool(false)));
    drop(w);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_change_undone_before_the_first_answer_is_written() {
    let (dir, path) = folder("undo", r#"{"vsync": true, "music_volume": 1.0}"#);
    let base = kept(&path);
    let w = SettingsWriter::start(path.clone()).unwrap();
    let first = PortSettings { vsync: false, ..base.clone() };
    w.ask(Ask::Save { base: base.clone(), settings: first.clone(), read: 1, popup: "p".into() });
    while w.writing() {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(read_file(&path).unwrap()["vsync"], Json::Bool(false));
    let second = PortSettings { music_volume: 0.5, ..base.clone() };
    w.ask(Ask::Save { base: base.clone(), settings: second.clone(), read: 1, popup: "p".into() });
    let done = answers(&w);
    assert_eq!(done.len(), 2, "{done:?}");
    let m = read_file(&path).unwrap();
    assert_eq!((&m["vsync"], &m["music_volume"]), (&Json::Bool(true), &Json::from(0.5)));
    std::fs::write(&path, r#"{"vsync": true, "music_volume": 1.0}"#).unwrap();
    let read = kept(&path);
    w.ask(Ask::Save {
        base: read.clone(),
        settings: PortSettings { vsync: false, ..read },
        read: 2,
        popup: "p".into(),
    });
    answers(&w);
    let m = read_file(&path).unwrap();
    assert_eq!((&m["vsync"], &m["music_volume"]), (&Json::Bool(false), &Json::from(1.0)));
    drop(w);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_unreadable_file_is_answered_then_replaced() {
    let (dir, path) = folder("broken", "{ not json");
    let w = SettingsWriter::start(path.clone()).unwrap();
    let s = PortSettings { vsync: false, ..PortSettings::default() };
    w.ask(Ask::Save { base: PortSettings::default(), settings: s.clone(), read: 1, popup: "ask".into() });
    let done = answers(&w);
    assert!(matches!(&done[..], [Done::Unreadable { popup, .. }] if popup == "ask"), "{done:?}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
    w.ask(Ask::Replace { base: PortSettings::default(), settings: s.clone(), read: 1 });
    let done = answers(&w);
    let [Done::Replaced { aside, .. }] = &done[..] else { panic!("{done:?}") };
    assert_eq!(std::fs::read_to_string(aside).unwrap(), "{ not json");
    assert!(!kept(&path).vsync);
    drop(w);
    std::fs::remove_dir_all(&dir).unwrap();
}
