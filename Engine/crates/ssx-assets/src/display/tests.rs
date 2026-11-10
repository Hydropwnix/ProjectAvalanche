use serde_json::json;

use super::*;

fn read(v: serde_json::Value) -> (DisplaySettings, Vec<String>) {
    DisplaySettings::from_json(Some(&v))
}

#[test]
fn a_hostile_display_member_falls_back_member_by_member() {
    let (s, problems) = read(json!({
        "mode": "Fullscreen",
        "fullscreen": "Exclusive",
        "monitor": "DP-1\u{1b}[31m",
        "video_mode": {"width": 1920, "height": 1080, "refresh_mhz": 60_000},
        "window": {"width": 100_000, "height": 900},
    }));
    assert_eq!(s.mode, WindowMode::Windowed);
    assert_eq!(s.fullscreen, WindowMode::Exclusive);
    assert_eq!(s.monitor, None);
    assert_eq!(s.video_mode, Some(VideoMode { width: 1920, height: 1080, refresh_mhz: 60_000 }));
    assert_eq!(s.window, None);
    assert_eq!(problems.len(), 3, "{problems:?}");
    for k in ["display.mode", "display.monitor", "display.window"] {
        assert!(problems.iter().any(|p| p.contains(k)), "{k}: {problems:?}");
    }
    for v in [json!("Borderless"), json!([1, 2]), json!(7)] {
        let (s, p) = read(v);
        assert_eq!(s, DisplaySettings::default());
        assert_eq!(p.len(), 1);
    }
    let (s, p) = read(json!({
        "mode": 3, "fullscreen": "Windowed", "monitor": "x".repeat(MAX_NAME + 1),
        "video_mode": {"width": 1920, "height": 1080, "refresh_mhz": 59.94},
        "window": {"width": "1600", "height": 900},
    }));
    assert_eq!(s, DisplaySettings::default());
    assert_eq!(p.len(), 5, "{p:?}");
    let (_, p) = read(json!({"video_mode": {"width": 0, "height": 1080, "refresh_mhz": 60_000}}));
    assert_eq!(p.len(), 1);
    let (_, p) = read(json!({"video_mode": {"width": 1920, "height": 1080, "refresh_mhz": 2_000_000}}));
    assert_eq!(p.len(), 1);
    let (_, p) = read(json!({"window": {"width": 319, "height": 180}}));
    assert_eq!(p.len(), 1);
    for bad in [json!({"render_height": 0}), json!({"render_height": 239}), json!({"render_height": "1080"})] {
        let (s, p) = read(bad.clone());
        assert_eq!((s.render_height, p.len()), (None, 1), "{bad}");
    }
    let (s, _) = read(json!({"render_height": 1080}));
    assert_eq!(s.render_height, Some(1080));
    assert_eq!(DisplaySettings::from_json(None), (DisplaySettings::default(), vec![]));
    let (s, p) = read(json!({"monitor": null, "video_mode": null, "window": null, "render_height": null}));
    assert_eq!((s, p.len()), (DisplaySettings::default(), 0));
}

#[test]
fn the_render_size_is_the_height_chosen_with_the_windows_shape() {
    assert_eq!(render_size((3840, 2160), Some(1080)), Some((1920, 1080)));
    assert_eq!(render_size((3440, 1440), Some(1080)), Some((2580, 1080)));
    assert_eq!(render_size((5120, 1440), Some(1080)), Some((3840, 1080)));
    assert_eq!(render_size((2560, 1080), Some(1080)), None);
    assert_eq!(render_size((1280, 720), Some(2160)), None);
    assert_eq!(render_size((3840, 2160), None), None);
    assert_eq!(render_size((3840, 2160), Some(239)), None);
    assert_eq!(render_size((0, 2160), Some(1080)), None);
    assert_eq!(render_size((1600, 900), Some(540)), Some((960, 540)));
}

#[test]
fn the_toggle_goes_to_the_kept_fullscreen_and_back() {
    let (s, _) = read(json!({"mode": "Exclusive", "fullscreen": "Borderless"}));
    assert_eq!((s.mode, s.fullscreen), (WindowMode::Exclusive, WindowMode::Exclusive));
    let w = s.toggled();
    assert_eq!((w.mode, w.fullscreen), (WindowMode::Windowed, WindowMode::Exclusive));
    assert_eq!(w.toggled().mode, WindowMode::Exclusive);
    assert_eq!(DisplaySettings::default().toggled().mode, WindowMode::Borderless);
    let x = DisplaySettings::default().with_mode(WindowMode::Exclusive);
    assert_eq!((x.mode, x.fullscreen), (WindowMode::Exclusive, WindowMode::Exclusive));
    let w = x.with_mode(WindowMode::Windowed);
    assert_eq!((w.mode, w.fullscreen), (WindowMode::Windowed, WindowMode::Exclusive));
}

#[test]
fn the_member_round_trips() {
    let s = DisplaySettings {
        mode: WindowMode::Borderless,
        fullscreen: WindowMode::Borderless,
        monitor: Some("HDMI-A-1".into()),
        video_mode: Some(VideoMode { width: 2560, height: 1080, refresh_mhz: 74_991 }),
        window: Some((1600, 900)),
        render_height: Some(1080),
    };
    assert_eq!(DisplaySettings::from_json(Some(&s.to_json())), (s, vec![]));
}

#[test]
fn the_games_options_are_one_argument_each() {
    let mut s = DisplaySettings { monitor: Some("--connect=evil.example:1".into()), ..Default::default() };
    s.window = Some((2560, 1080));
    s.video_mode = Some(VideoMode { width: 3440, height: 1440, refresh_mhz: 59_940 });
    s.render_height = Some(1080);
    let args = s.args();
    assert_eq!(
        args,
        [
            "--window-mode=windowed",
            "--fullscreen-mode=borderless",
            "--monitor=--connect=evil.example:1",
            "--video-mode=3440x1440@59.94",
            "--window-size=2560x1080",
            "--render-height=1080",
        ]
    );
    s.monitor = Some("a\nb".into());
    s.window = Some((1, 1));
    s.render_height = Some(239);
    assert!(
        !s.args()
            .iter()
            .any(|a| a.starts_with("--monitor") || a.starts_with("--window-size") || a.starts_with("--render-height"))
    );
}

#[test]
fn video_modes_parse_exactly_in_millihertz() {
    let m = |s: &str| VideoMode::parse(s);
    assert_eq!(m("1920x1080@59.94"), Some(VideoMode { width: 1920, height: 1080, refresh_mhz: 59_940 }));
    assert_eq!(m("5120x1440@240"), Some(VideoMode { width: 5120, height: 1440, refresh_mhz: 240_000 }));
    assert_eq!(m("1024x768@60.001").map(|v| v.refresh_mhz), Some(60_001));
    for bad in [
        "1920x1080",
        "1920x1080@",
        "1920x1080@59.9401",
        "1920x1080@-60",
        "1920x1080@6e1",
        "0x1080@60",
        "9000x1080@60",
        "1920x1080@0.5",
        "1920x1080@1001",
        "1920X1080@60",
        "x@60",
        "1920x1080@60.",
        "1920x1080@.5",
    ] {
        assert_eq!(m(bad), None, "{bad}");
    }
    for s in ["1920x1080@59.94", "2560x1080@75", "3440x1440@143.999"] {
        assert_eq!(m(s).unwrap().to_string(), s);
    }
}

#[test]
fn the_display_list_round_trips_and_is_read_strictly() {
    let mode = |w, h, r| VideoMode { width: w, height: h, refresh_mhz: r };
    let list = DisplayList {
        backend: Backend::X11,
        monitors: vec![
            MonitorInfo {
                name: Some("DP-1".into()),
                position: (0, 0),
                size: (3440, 1440),
                scale_permille: 1000,
                refresh_mhz: Some(144_000),
                primary: true,
                modes: vec![mode(3440, 1440, 144_000), mode(2560, 1080, 60_000)],
            },
            MonitorInfo {
                name: None,
                position: (-1920, 200),
                size: (1920, 1080),
                scale_permille: 1250,
                refresh_mhz: None,
                primary: false,
                modes: vec![],
            },
        ],
    };
    let text = list.to_json();
    assert_eq!(DisplayList::parse(&text), Ok(list.clone()));
    assert_eq!(list.find(None).and_then(|m| m.name.as_deref()), Some("DP-1"));
    assert_eq!(list.find(Some("HDMI-1")), None);
    assert!(list.monitors[0].label().contains("3440 × 1440, 144 Hz (primary)"));
    let v = json!({"backend": "wayland", "monitors": [{"name": "eDP-1", "x": 0, "y": 0, "width": 1280, "height": 800,
        "scale_permille": 1000, "refresh_mhz": 60_000, "primary": false, "modes": [
            {"width": 1024, "height": 768, "refresh_mhz": 60_000}, {"width": 1280, "height": 800, "refresh_mhz": 60_000},
            {"width": 1280, "height": 800, "refresh_mhz": 60_000}, {"width": 99_999, "height": 800, "refresh_mhz": 60_000}]}]});
    let l = DisplayList::parse(&v.to_string()).unwrap();
    assert!(!l.backend.exclusive());
    assert_eq!(l.monitors[0].modes, [mode(1280, 800, 60_000), mode(1024, 768, 60_000)]);
    let monitor = |scale: u32| {
        json!({"name": "A", "x": 0, "y": 0, "width": 10, "height": 10, "scale_permille": scale,
        "refresh_mhz": null, "primary": true, "modes": []})
    };
    for bad in [
        json!({"backend": "dos", "monitors": []}),
        json!({"backend": "x11", "monitors": [monitor(0)]}),
        json!({"backend": "x11", "monitors": [monitor(8_001)]}),
        json!({"backend": "x11", "monitors": serde_json::Value::Array(vec![monitor(1000); MAX_MONITORS + 1])}),
        json!({"backend": "x11", "monitors": [{"name": "A\u{7}"}]}),
    ] {
        assert!(DisplayList::parse(&bad.to_string()).is_err(), "{bad}");
    }
    assert!(DisplayList::parse(&" ".repeat(MAX_LIST_BYTES + 1)).is_err());
}

#[test]
fn the_largest_list_is_printable_ascii_and_fits() {
    let name = format!("{}\u{202e}", "\u{1f600}".repeat((MAX_NAME - 3) / 4));
    assert!(monitor_name_valid(&name) && name.len() <= MAX_NAME);
    let monitor = MonitorInfo {
        name: Some(name),
        position: (i32::MIN, i32::MIN),
        size: (65_535, 65_535),
        scale_permille: *SCALE_PERMILLE.end(),
        refresh_mhz: Some(999_999),
        primary: false,
        modes: (0..MAX_MODES as u32)
            .map(|i| VideoMode { width: MAX_SIDE - i, height: MAX_SIDE, refresh_mhz: 999_999 })
            .collect(),
    };
    let list = DisplayList { backend: Backend::Wayland, monitors: vec![monitor; MAX_MONITORS] };
    let text = list.to_json();
    assert!(text.bytes().all(|b| (b' '..=b'~').contains(&b)), "printable ASCII only");
    assert!(text.len() <= MAX_LIST_BYTES, "{} bytes", text.len());
    assert_eq!(DisplayList::parse(&text), Ok(list));
}

#[test]
fn the_member_is_written_into_the_file_and_never_over_an_unreadable_one() {
    let dir = std::env::temp_dir().join(format!("ssx-display-file-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("settings.json");
    let base = DisplaySettings::default();
    let s = DisplaySettings { mode: WindowMode::Borderless, ..Default::default() };
    write_member(&path, &base, &s).unwrap();
    assert_eq!(read_member(&path), (s.clone(), vec![]));
    let text =
        r#"{"vsync": false, "player_name": "Elise", "display": {"extra": 1, "monitor": "DP-2", "mode": "Windowed"}}"#;
    std::fs::write(&path, text).unwrap();
    let sized = DisplaySettings { window: Some((1600, 900)), ..s.clone() };
    write_member(&path, &s, &sized).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!((v["vsync"].clone(), v["player_name"].clone()), (json!(false), json!("Elise")));
    let d = &v["display"];
    assert_eq!((d["extra"].clone(), d["monitor"].clone()), (json!(1), json!("DP-2")), "theirs kept: {d}");
    assert_eq!((d["mode"].clone(), d["window"].clone()), (json!("Windowed"), json!({"width": 1600, "height": 900})));
    std::fs::write(&path, r#"{"display": "fullscreen please"}"#).unwrap();
    write_member(&path, &base, &s).unwrap();
    assert_eq!(read_member(&path), (s.clone(), vec![]));
    for unreadable in [&b"[1, 2]"[..], b"{\"vsync\": tru", &vec![b' '; crate::settings_file::MAX_FILE as usize + 1][..]]
    {
        std::fs::write(&path, unreadable).unwrap();
        assert!(write_member(&path, &base, &s).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), unreadable, "left as it was");
        let (got, problems) = read_member(&path);
        assert_eq!((got, problems.len()), (DisplaySettings::default(), 1));
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_scale_factor_is_kept_in_thousandths() {
    for (factor, permille) in [
        (1.0, 1000),
        (1.022_222_222_222_222, 1022),
        (1.25, 1250),
        (0.1, 250),
        (100.0, 8000),
        (f64::NAN, 1000),
        (f64::INFINITY, 8000),
        (f64::NEG_INFINITY, 250),
    ] {
        assert_eq!(scale_permille(factor), permille, "{factor}");
    }
}
