use std::collections::HashMap;
use std::sync::Arc;

use ssx_formats::gfx::shape::Shape;
use ssx_formats::gfx::text::Font;

use super::json::{self, Pos};
use super::*;
use crate::geom::RectF;
use crate::library::{Character, FrameDef, Library, MovieDef, TimelineDef};

const GOOD: &str = r##"{
  "format": 1,
  "view": "ava_test",
  "layer": "popup",
  "stage": [1280, 720],
  "frame_rate": 30,
  "imports": [
    {"name": "Panel", "url": "components/Panel.swf", "symbol": "components.Panel"},
    {"name": "Grid", "url": "components/GridMenu.swf", "symbol": "components.GridMenu"},
    {"name": "Cell", "url": "components/ListMenuCellSmall.swf", "symbol": "components.ListMenuCellSmall"}
  ],
  "symbols": [
    {"name": "AvaTestCell", "export": true, "cell": "mcCell", "children": [{"instance": "mcCell", "symbol": "Cell"}]},
    {"name": "AvaTest", "export": true, "children": [
      {"instance": "mcPanel", "symbol": "Panel", "x": 10, "y": 20},
      {"instance": "mcBack", "rect": {"width": 100, "height": 50, "color": "#102030c0"}, "alpha": 50},
      {"instance": "txtEntry", "text": {"font": "Test Font", "size": 20, "color": "#ffffff", "width": 300,
        "height": 30, "html": true, "text": "A<B"}},
      {"instance": "mcKeys", "symbol": "Grid", "x": 40, "y": 60, "visible": false}
    ]}
  ],
  "view_symbol": {"symbol": "AvaTest"},
  "logic": {
    "input_priority": "popup",
    "consume_unmapped": true,
    "data": ["ava_test", "ava_test_keys", "ava_test_index"],
    "texts": [{"target": "txtEntry", "data": "ava_test", "field": "text"}],
    "menus": [{"target": "mcKeys", "columns": 4, "rows": 2, "cell_width": 60, "cell_height": 40,
      "data": "ava_test_keys", "index": "ava_test_index", "cell": "AvaTestCell"}],
    "panels": [{"target": "mcPanel", "width": 400, "height": 200, "dimmer": 50}],
    "inputs": [
      {"key": "DPAD_UP", "menu": "mcKeys", "do": "up"},
      {"key": "START", "action": "A_AvaTestDone", "sound": "menu_select"}
    ]
  }
}"##;

fn err(text: &str) -> LayoutError {
    match Layout::parse("t.json", text) {
        Ok(_) => panic!("expected an error"),
        Err(e) => e,
    }
}

fn with(from: &str, to: &str) -> String {
    assert!(GOOD.contains(from), "{from}");
    GOOD.replacen(from, to, 1)
}

#[test]
fn the_good_layout_reads() {
    let l = Layout::parse("t.json", GOOD).unwrap();
    assert_eq!(l.view, "ava_test");
    assert_eq!(l.layer, "LAYER_POPUP");
    assert_eq!(l.logic.input_priority, Some("INPUT_PRIORITY_POPUP"));
    assert_eq!(l.symbols.len(), 2);
    assert_eq!(
        l.logic.menus[0].kind,
        MenuKind::Grid {
            columns: 4,
            rows: 2,
            cell_width: 60.0,
            cell_height: 40.0,
            spacing_x: 0.0,
            spacing_y: 0.0,
            wrap: false
        }
    );
    assert_eq!(l.resolve_path("txtEntry").map(|c| c.instance.as_str()), Some("txtEntry"));
}

#[test]
fn unknown_members_are_errors_at_their_key() {
    let e = err(&with(r#""frame_rate": 30,"#, r#""frame_rate": 30, "colour": 3,"#));
    assert_eq!((e.pos.line, e.message.as_str()), (6, "layout: unknown member `colour`"));
    let e = err(&with(r#""x": 10, "y": 20}"#, r#""x": 10, "y": 20, "z": 1}"#));
    assert_eq!(e.pos.line, 15);
    assert!(e.message.contains("unknown member `z`"), "{e}");
    assert!(e.to_string().starts_with("t.json:15:"), "{e}");
}

#[test]
fn bad_references_are_errors_with_their_line() {
    let e = err(&with(r#""symbol": "Panel", "x""#, r#""symbol": "Pannel", "x""#));
    assert_eq!(e.pos.line, 15);
    assert!(e.message.contains("no import or symbol is named \"Pannel\""), "{e}");
    let e = err(&with(r#"{"target": "txtEntry""#, r#"{"target": "txtEntri""#));
    assert_eq!(e.pos.line, 27);
    let e = err(&with(r#""data": "ava_test", "field""#, r#""data": "ava_tset", "field""#));
    assert!(e.message.contains("not in the logic's `data`"), "{e}");
    let e = err(&with(r#""cell": "AvaTestCell""#, r#""cell": "AvaTest2""#));
    assert!(e.message.contains("not a cell symbol"), "{e}");
    let e = err(&with(r#""menu": "mcKeys", "do": "up""#, r#""menu": "mcPanel", "do": "up""#));
    assert!(e.message.contains("not one of the logic's `menus`"), "{e}");
    let e = err(&with(r#"{"target": "mcKeys", "columns""#, r#"{"target": "mcPanel", "columns""#));
    assert!(e.message.contains("GridMenu"), "{e}");
    let e = err(&with(r#"{"target": "mcPanel", "width""#, r#"{"target": "mcKeys", "width""#));
    assert!(e.message.contains("components.Panel"), "{e}");
    let e = err(&with(r#""view_symbol": {"symbol": "AvaTest"}"#, r#""view_symbol": {"symbol": "AvaNone"}"#));
    assert_eq!(e.pos.line, 22);
}

#[test]
fn names_are_checked() {
    for (from, to, want) in [
        (r#""view": "ava_test""#, r#""view": "test""#, "starting with `ava_`"),
        (r#""view": "ava_test""#, r#""view": "ava_Test""#, "lower case"),
        (r#""view": "ava_test""#, r#""view": "ava_../x""#, "not a name"),
        (r#""name": "AvaTestCell""#, r#""name": "TestCell""#, "starts with `Ava`"),
        (r#""instance": "mcBack""#, r#""instance": "mcPanel""#, "given twice"),
        (r#""name": "Grid""#, r#""name": "Panel""#, "given twice"),
        (r#""layer": "popup""#, r#""layer": "LAYER_POPUP""#, "not one of"),
        (r#""key": "DPAD_UP""#, r#""key": "START""#, "mapped twice"),
        (r#""action": "A_AvaTestDone""#, r#""action": "AvaTestDone""#, "starting with `A_`"),
        (r#""sound": "menu_select""#, r#""sound": "explode""#, "not one of"),
        (r#""font": "Test Font""#, r#""font": 3"#, "expected a string"),
        (r##""color": "#ffffff""##, r#""color": "white""#, "#rrggbb"),
        (r#""frame_rate": 30"#, r#""frame_rate": 0"#, "outside"),
        (r#""rows": 2"#, r#""rows": 2.5"#, "not a whole number"),
        (r#""format": 1"#, r#""format": 2"#, "outside"),
    ] {
        let e = err(&with(from, to));
        assert!(e.message.contains(want), "{to}: {e}");
    }
}

#[test]
fn import_urls_stay_among_the_components() {
    for url in [
        "components/../views/hud.swf",
        "/components/Panel.swf",
        "components/Panel.gfx",
        "views/hud.swf",
        "components//Panel.swf",
        "components/Pa nel.swf",
        "components/Panel.swf\u{0}",
        "components\\Panel.swf",
    ] {
        assert!(!valid_component_url(url), "{url}");
        let text =
            with(r#""url": "components/Panel.swf""#, &format!(r#""url": {}"#, serde_json::to_string(url).unwrap()));
        assert!(err(&text).message.contains("import url"), "{url}");
    }
    assert!(valid_component_url("components/HUD/Leaderboard.swf"));
}

#[test]
fn symbols_cannot_contain_themselves_or_explode() {
    let e = err(&with(
        r#""symbols": ["#,
        r#""symbols": [{"name": "Loop", "children": [{"instance": "a", "symbol": "Loop2"}]},
          {"name": "Loop2", "children": [{"instance": "b", "symbol": "Loop"}]},"#,
    ));
    assert!(e.message.contains("contains itself"), "{e}");
    let mut symbols = String::new();
    for k in 0..6 {
        let next = if k == 5 { "Panel".to_string() } else { format!("AvaL{}", k + 1) };
        let kids: Vec<String> = (0..16).map(|i| format!(r#"{{"instance": "c{i}", "symbol": "{next}"}}"#)).collect();
        symbols.push_str(&format!(r#"{{"name": "AvaL{k}", "export": true, "children": [{}]}},"#, kids.join(",")));
    }
    let text = with(r#""symbols": ["#, &format!(r#""symbols": [{symbols}"#));
    let text = text.replacen(
        r#"{"instance": "mcPanel", "symbol": "Panel", "x": 10"#,
        r#"{"instance": "mcPanel", "symbol": "AvaL0", "x": 10"#,
        1,
    );
    let e = err(&text);
    assert!(e.message.contains("instances, at most"), "{e}");
}

#[test]
fn json_syntax_and_limits() {
    let lim = limits::JSON;
    let bad = [
        ("{\"a\": 1,}", "member name"),
        ("[1, 2,]", "unexpected"),
        ("{\"a\": 1, \"a\": 2}", "given twice"),
        ("{\"a\": 1e400}", "out of range"),
        ("{\"a\": NaN}", "unexpected"),
        ("{\"a\": 01}", "after a member"),
        ("{\"a\": \"\\x\"}", "unknown escape"),
        ("{\"a\": \"\\ud800\"}", "surrogate"),
        ("{\"a\": \"line\nbreak\"}", "control character"),
        ("{\"a\": 1} 2", "after the document"),
        ("// c\n{}", "unexpected"),
        ("", "unexpected end"),
        ("\"", "unterminated"),
    ];
    for (text, want) in bad {
        let e = json::parse(text, &lim).unwrap_err();
        assert!(e.message.contains(want), "{text:?}: {}", e.message);
    }
    let deep = "[".repeat(100) + &"]".repeat(100);
    assert!(json::parse(&deep, &lim).unwrap_err().message.contains("deeper"));
    let long = format!("\"{}\"", "x".repeat(5000));
    assert!(json::parse(&long, &lim).unwrap_err().message.contains("longer"));
    let many = format!("[{}]", vec!["0"; 30_000].join(","));
    assert!(json::parse(&many, &lim).unwrap_err().message.contains("more than"));
    let big = " ".repeat(300 * 1024);
    assert!(json::parse(&big, &lim).unwrap_err().message.contains("bytes"));
    let n = json::parse("{\n  \"k\": [true, null, -1.5e2, \"\\u00e9\\ud83d\\ude00\"]\n}", &lim).unwrap();
    let json::Value::Object(m) = &n.value else { panic!() };
    assert_eq!(m[0].0.pos, Pos { line: 2, col: 3 });
    let json::Value::Array(a) = &m[0].1.value else { panic!() };
    assert_eq!(a[2].value, json::Value::Number(-150.0));
    assert_eq!(a[3].value, json::Value::String("é😀".into()));
    assert_eq!(a[1].pos, Pos { line: 2, col: 15 });
}

#[test]
fn hostile_texts_never_panic() {
    let bytes = GOOD.as_bytes();
    for cut in 0..bytes.len() {
        if let Ok(s) = std::str::from_utf8(&bytes[..cut]) {
            let _ = Layout::parse("t.json", s);
        }
    }
    let mut x: u32 = 0x9E37_79B9;
    let alphabet = b"{}[]\":,0123456789.-+eE \n\\uAvamcxnt";
    for _ in 0..20_000 {
        let mut b = bytes.to_vec();
        for _ in 0..1 + (x % 3) {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let at = x as usize % b.len();
            b[at] = alphabet[(x >> 8) as usize % alphabet.len()];
        }
        if let Ok(s) = std::str::from_utf8(&b) {
            let _ = Layout::parse("t.json", s);
        }
    }
}

fn library() -> Library {
    let mut lib = crate::display::tests::empty_lib();
    let movie = |lib: &mut Library, path: &str, exports: &[(&str, u16)], chars: Vec<(u16, Character)>| {
        let id = lib.movies().len();
        let mut map = HashMap::new();
        for (k, v) in chars {
            map.insert(k, v);
        }
        for (_, cid) in exports {
            map.entry(*cid).or_insert_with(|| {
                Character::Sprite(Arc::new(TimelineDef { movie: id, id: *cid, frames: vec![FrameDef::default()] }))
            });
        }
        lib.insert_movie(MovieDef {
            path: path.to_string(),
            dir: String::new(),
            version: 8,
            frame_rate: 30.0,
            stage: RectF::new(0.0, 0.0, 100.0, 100.0),
            background: None,
            swf_name: path.to_string(),
            root: Arc::new(TimelineDef { movie: id, id: 0, frames: vec![FrameDef::default()] }),
            chars: map,
            exports: exports.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            imports: HashMap::new(),
            scaling_grids: HashMap::new(),
        })
    };
    movie(&mut lib, "components/panel.gfx", &[("components.Panel", 1)], Vec::new());
    movie(&mut lib, "components/gridmenu.gfx", &[("components.GridMenu", 1)], Vec::new());
    movie(&mut lib, "components/listmenucellsmall.gfx", &[("components.ListMenuCellSmall", 1)], Vec::new());
    movie(&mut lib, "components/listmenu.gfx", &[("components.ListMenu", 1)], Vec::new());
    movie(&mut lib, "components/hud/billboardcontainer.gfx", &[("components.HUD.BillboardContainer", 1)], Vec::new());
    let font = Font {
        version: 3,
        id: 1,
        flags: Font::WIDE_CODES,
        language: 0,
        name: "Test Font".into(),
        glyphs: vec![Shape::default(); 3],
        codes: vec![u16::from(b'A'), u16::from(b'B'), u16::from(b'<')],
        layout: None,
        unk_trailing_layout: None,
    };
    movie(&mut lib, "gfxfontlib/na/gfxfontlib.gfx", &[], vec![(1, Character::Font(Arc::new(font)))]);
    lib.load_font_library("na").unwrap();
    lib
}

#[test]
fn a_layout_builds_a_movie() {
    let mut lib = library();
    let l = Layout::parse("t.json", GOOD).unwrap();
    let id = l.build(&mut lib).unwrap();
    let m = lib.movie(id);
    assert_eq!(m.path, "views/ava_test.gfx");
    assert_eq!(lib.movie_by_path("views/ava_test.swf"), Some(id));
    assert_eq!(m.imports.len(), 3);
    assert!(m.imports.values().all(|i| i.target.is_some()));
    let view = lib.export(id, "AvaTest").unwrap();
    let Some((_, Character::Sprite(tl))) = lib.resolve(view) else { panic!() };
    assert_eq!(tl.frames[0].tags.len(), 4);
    let FrameTag::Place(root) = &m.root.frames[0].tags[0] else { panic!() };
    assert_eq!(root.name.as_deref(), Some("mView"));
    let text = m.chars.values().find_map(|c| match c {
        Character::EditText(e) => Some(e.clone()),
        _ => None,
    });
    let text = text.unwrap();
    assert_eq!(
        text.initial_text.as_deref(),
        Some("<p align=\"left\"><font face=\"Test Font\" size=\"20\" color=\"#ffffff\">A&lt;B</font></p>")
    );
    let font = lib.font_for(id, text.font_id, None).unwrap();
    assert_eq!(font.glyphs.len(), 3);
    assert!(l.build(&mut lib).unwrap_err().message.contains("exists already"));
}

#[test]
fn building_checks_what_the_library_holds() {
    let cases = [
        (r#""symbol": "components.ListMenuCellSmall""#, r#""symbol": "components.ListMenuCel""#, "exports no"),
        (r#""url": "components/Panel.swf""#, r#""url": "components/Nothing.swf""#, "cannot be loaded"),
        (r#""font": "Test Font""#, r#""font": "Arial""#, "not in the font library"),
        (r#""text": "A<B""#, r#""text": "AC""#, "no glyph for 'C'"),
    ];
    for (from, to, want) in cases {
        let mut lib = library();
        let l = Layout::parse("t.json", &with(from, to)).unwrap();
        let e = l.build(&mut lib).unwrap_err();
        assert!(e.message.contains(want), "{to}: {e}");
        assert!(e.pos.line > 1, "{e}");
    }
}

use crate::library::FrameTag;

const HUB: &str = r##"{
  "format": 1, "view": "ava_hubtest", "layer": "screen", "stage": [1280, 720], "frame_rate": 30,
  "imports": [
    {"name": "Panel", "url": "components/Panel.swf", "symbol": "components.Panel"},
    {"name": "List", "url": "components/ListMenu.swf", "symbol": "components.ListMenu"},
    {"name": "Bill", "url": "components/HUD/BillboardContainer.swf", "symbol": "components.HUD.BillboardContainer"},
    {"name": "Cell", "url": "components/ListMenuCellSmall.swf", "symbol": "components.ListMenuCellSmall"}
  ],
  "symbols": [
    {"name": "AvaHubTestCell", "export": true, "cell": "mcCell", "children": [{"instance": "mcCell", "symbol": "Cell"}]},
    {"name": "AvaHubTest", "export": true, "children": [
      {"instance": "mcBill", "symbol": "Bill"},
      {"instance": "mcPanel", "symbol": "Panel", "scale_x": 4.53125},
      {"instance": "mcMenu", "symbol": "List"}
    ]}
  ],
  "view_symbol": {"symbol": "AvaHubTest", "x": 12, "y": 48},
  "logic": {
    "base": "fe_view",
    "data": ["ava_m", "ava_i"],
    "menus": [{"target": "mcMenu", "rows": 5, "cell_width": 303, "cell_height": 80, "spacing": -10,
      "panel_gaps": [5, -14], "data": "ava_m", "index": "ava_i", "cell": "AvaHubTestCell"}],
    "panels": [{"target": "mcPanel", "width": 303, "fit_menu": "mcMenu", "dimmer": 50}],
    "billboards": [{"target": "mcBill", "data": "globe_data", "symbol": "SocialBillboardFeedWrapper"}],
    "on_enter": {"clear_inputs": true, "event": "ACTIVITY_FEED_NORMAL_INTRO", "sound": "eg_ready_select"},
    "inputs": [
      {"key": "ADVANCE", "menu": "mcMenu", "do": "advance", "lock_ms": 200},
      {"key": "DPAD_DOWN", "menu": "mcMenu", "do": "down", "blocked_sound": "menu_unhighlightable"},
      {"key": "DIGITAL_THREE", "take": true}
    ]
  }
}"##;

#[test]
fn the_hubs_members_read_and_are_checked() {
    let l = Layout::parse("h.json", HUB).unwrap();
    let g = &l.logic;
    assert_eq!((g.base, g.input_priority), (Base::FeView, None));
    assert!(matches!(g.menus[0].kind, MenuKind::List { panel_gaps: Some((5.0, -14.0)), rows: 5, .. }));
    assert_eq!(g.panels[0].height, PanelHeight::FitMenu("mcMenu".into()));
    assert_eq!(
        (g.billboards[0].data.as_str(), g.billboards[0].symbol.as_str()),
        ("globe_data", "SocialBillboardFeedWrapper")
    );
    assert_eq!(
        g.on_enter,
        OnEnter { clear_inputs: true, event: Some("ACTIVITY_FEED_NORMAL_INTRO"), sound: Some("eg_ready_select") }
    );
    assert_eq!((g.inputs[0].lock_ms, &g.inputs[2].what), (Some(200.0), &InputDo::Take));
    let bad = |from: &str, to: &str, want: &str| {
        assert!(HUB.contains(from), "{from}");
        let e = match Layout::parse("h.json", &HUB.replacen(from, to, 1)) {
            Ok(_) => panic!("{to}: expected an error"),
            Err(e) => e,
        };
        assert!(e.message.contains(want), "{to}: {e}");
    };
    bad(r#""base": "fe_view","#, "", "`ClearInputs` is `ssx.FEView`'s");
    bad(r#""fit_menu": "mcMenu""#, r#""fit_menu": "mcPanel""#, "not one of the logic's list menus");
    bad(r#""fit_menu": "mcMenu""#, r#""fit_menu": "mcMenu", "height": 9"#, "exactly one of `height` and `fit_menu`");
    bad(r#""event": "ACTIVITY_FEED_NORMAL_INTRO""#, r#""event": "kNavChangeView""#, "is not one of");
    bad(r#""take": true"#, r#""take": false"#, "`take` can only be true");
    bad(r#""take": true"#, r#""take": true, "action": "A_X""#, "exactly one of");
    bad(r#""do": "advance", "lock_ms": 200"#, r#""do": "advance", "lock_ms": 0"#, "outside");
    bad(
        r#""do": "down", "blocked_sound""#,
        r#""do": "advance", "blocked_sound""#,
        "only for a list menu's up and down",
    );
    bad(r#""target": "mcBill""#, r#""target": "mcPanel""#, "components.HUD.BillboardContainer");
    bad(r#""panel_gaps": [5, -14]"#, r#""panel_gaps": [5]"#, "expected [top, bottom]");
}

#[test]
fn a_generated_movie_is_built_when_first_loaded() {
    let mut lib = library();
    let l = Layout::parse("t.json", GOOD).unwrap();
    let before = lib.movies().len();
    l.movie(&mut lib).unwrap();
    assert_eq!(lib.movies().len(), before);
    let layout = l.clone();
    lib.add_builder("views/ava_test.swf", Arc::new(move |lib| layout.build(lib).map_err(anyhow::Error::from))).unwrap();
    assert!(lib.movie_by_path("views/ava_test.gfx").is_none());
    let again = l.clone();
    assert!(
        lib.add_builder("views/ava_test.gfx", Arc::new(move |lib| again.build(lib).map_err(anyhow::Error::from)))
            .is_err()
    );
    let id = lib.load_movie("views/ava_test.swf").unwrap();
    assert_eq!((id, lib.movie(id).path.as_str()), (before, "views/ava_test.gfx"));
    assert_eq!(lib.load_movie("views/ava_test.gfx").unwrap(), id);
    lib.add_builder("views/ava_other.swf", Arc::new(|_: &mut Library| Ok(0))).unwrap();
    assert!(lib.load_movie("views/ava_other.swf").is_err());
}

const BROWSER: &str = r##"{
  "format": 1,
  "view": "ava_list",
  "layer": "screen",
  "stage": [1280, 720],
  "frame_rate": 30,
  "imports": [
    {"name": "ListMenu", "url": "components/ListMenu.swf", "symbol": "components.ListMenu"},
    {"name": "Highlight", "url": "components/Highlight.swf", "symbol": "components.Highlight"},
    {"name": "ImageLoader", "url": "components/ImageLoader.swf", "symbol": "components.ImageLoader"},
    {"name": "DropSelector", "url": "components/DropSelector.swf", "symbol": "components.DropSelector"}
  ],
  "symbols": [
    {"name": "AvaListRow", "export": true,
      "cell": {"texts": [{"text": "txtName", "field": "strName", "fit": "shrink"}, {"text": "txtPing", "field": "p"}],
        "images": [{"loader": "mcLock", "field": "strLock"}], "highlight": "mcHighlight", "width": 400, "height": 30},
      "children": [
        {"instance": "mcHighlight", "symbol": "Highlight"},
        {"instance": "mcLock", "symbol": "ImageLoader"},
        {"instance": "txtName", "text": {"font": "Test Font", "size": 18, "color": "#ffffff", "width": 200,
          "height": 26}},
        {"instance": "txtPing", "x": 300, "text": {"font": "Test Font", "size": 18, "color": "#ffffff", "width": 90,
          "height": 26}}
      ]},
    {"name": "AvaList", "export": true, "children": [
      {"instance": "mcRows", "symbol": "ListMenu"},
      {"instance": "mcDrop", "symbol": "DropSelector"},
      {"instance": "txtStatus", "text": {"font": "Test Font", "size": 15, "color": "#ffffff", "width": 400,
        "height": 22}},
      {"instance": "txtPlayers", "text": {"font": "Test Font", "size": 15, "color": "#ffffff", "width": 300,
        "height": 104, "multiline": true, "wrap": true}}
    ]}
  ],
  "view_symbol": {"symbol": "AvaList"},
  "logic": {
    "base": "fe_view",
    "data": ["ava_list", "ava_list_rows", "ava_list_index", "ava_list_drop"],
    "texts": [
      {"target": "txtStatus", "data": "ava_list", "field": "status", "fit": "shrink"},
      {"target": "txtPlayers", "data": "ava_list", "field": "players",
        "fit": {"items": "playerNames", "more": "playersMore", "separator": ", "}}
    ],
    "visibles": [{"target": "txtPlayers", "data": "ava_list", "field": "bShown"}],
    "menus": [{"target": "mcRows", "rows": 9, "cell_width": 400, "cell_height": 30, "spacing": 2, "wrap": false,
      "data": "ava_list_rows", "index": "ava_list_index", "cell": "AvaListRow"}],
    "paging": {"menu": "mcRows", "setup": "A_AvaListSetup"},
    "drop_panels": [{"target": "mcDrop", "data": "ava_list_drop"}],
    "inputs": [{"key": "DPAD_DOWN", "action": "A_AvaListRequestData", "param": 1}]
  }
}"##;

#[test]
fn the_browsers_members_read_and_are_checked() {
    let l = Layout::parse("b.json", BROWSER).unwrap();
    let g = &l.logic;
    assert_eq!((g.texts[0].fit.clone(), g.texts[1].html), (TextFit::Shrink, false));
    assert_eq!(
        g.texts[1].fit,
        TextFit::List { items: "playerNames".into(), more: "playersMore".into(), separator: ", ".into() }
    );
    assert_eq!(g.paging.as_ref().map(|p| (p.menu.as_str(), p.setup.as_str())), Some(("mcRows", "A_AvaListSetup")));
    assert_eq!((g.drop_panels[0].target.as_str(), g.visibles[0].field.as_str()), ("mcDrop", "bShown"));
    assert_eq!(
        g.inputs[0].what,
        InputDo::Action { name: "A_AvaListRequestData".into(), sound: None, param: Some(1.0) }
    );
    let cell = l.symbols.iter().find_map(|s| s.cell.clone());
    let Some(CellDef::Own { texts, images, shrink, .. }) = cell else { panic!("an own cell: {cell:?}") };
    assert_eq!((texts.len(), images.len(), shrink), (2, 1, vec!["txtName".to_string()]));
    let bad = |from: &str, to: &str, want: &str| {
        assert!(BROWSER.contains(from), "{from}");
        let e = match Layout::parse("b.json", &BROWSER.replacen(from, to, 1)) {
            Ok(_) => panic!("{to}: expected an error"),
            Err(e) => e,
        };
        assert!(e.message.contains(want), "{to}: {e}");
    };
    bad(r#""field": "status", "fit": "shrink""#, r#""field": "status", "fit": "grow""#, "not one of");
    bad(r#""separator": ", ""#, r#""separator": """#, "empty, or a control character");
    bad(r#""separator": ", ""#, r#""separator": "\u0007""#, "empty, or a control character");
    bad(r#""separator": ", ""#, r#""separator": ",,,,,,,,,""#, "longer than 8 characters");
    bad(r#""field": "players","#, r#""field": "players", "html": true,"#, "a list is plain text");
    bad(r#""more": "playersMore", "#, "", "missing");
    bad(r#""field": "strLock"}"#, r#""field": "strLock", "fit": "shrink"}"#, "unknown member `fit`");
    bad(r#""field": "strName", "fit": "shrink""#, r#""field": "strName", "fit": true"#, "expected a string");
    bad(r#""paging": {"menu": "mcRows""#, r#""paging": {"menu": "mcDrop""#, "not one of the logic's list menus");
    bad(r#"{"target": "mcDrop", "data""#, r#"{"target": "mcRows", "data""#, "components.DropSelector");
    bad(r#""field": "bShown""#, r#""field": "b Shown""#, "not a name");
    bad(r#""data": "ava_list", "field": "bShown""#, r#""data": "ava_lsit", "field": "bShown""#, "not in the logic");
}

#[test]
fn a_views_image_bindings_read_and_are_checked() {
    let with = BROWSER
        .replacen(
            r#"{"instance": "mcDrop", "symbol": "DropSelector"},"#,
            r#"{"instance": "mcDrop", "symbol": "DropSelector"}, {"instance": "mcIcon", "symbol": "ImageLoader"},"#,
            1,
        )
        .replacen(
            r#""visibles": ["#,
            r#""images": [{"target": "mcIcon", "data": "ava_list", "field": "strIcon"}], "visibles": ["#,
            1,
        );
    let l = Layout::parse("b.json", &with).unwrap();
    let i = &l.logic.images;
    assert_eq!(i.len(), 1);
    assert_eq!((i[0].target.as_str(), i[0].data.as_str(), i[0].field.as_str()), ("mcIcon", "ava_list", "strIcon"));
    let bad = |from: &str, to: &str, want: &str| {
        assert!(with.contains(from), "{from}");
        match Layout::parse("b.json", &with.replacen(from, to, 1)) {
            Ok(_) => panic!("{to}: expected an error"),
            Err(e) => assert!(e.message.contains(want), "{to}: {e}"),
        }
    };
    bad(r#"{"target": "mcIcon", "data""#, r#"{"target": "txtStatus", "data""#, "components.ImageLoader");
    bad(r#""data": "ava_list", "field": "strIcon""#, r#""data": "ava_lst", "field": "strIcon""#, "not in the logic");
    bad(r#""field": "strIcon"}"#, r#""field": "strIcon", "fit": "shrink"}"#, "unknown member `fit`");
    bad(r#""field": "strIcon""#, r#""field": "str Icon""#, "not a name");
}

const SCROLL: &str = r##"{
  "format": 1, "view": "ava_scrolltest", "layer": "activityfeed", "stage": [1280, 720], "frame_rate": 30,
  "imports": [
    {"name": "List", "url": "components/ListMenu.swf", "symbol": "components.ListMenu"},
    {"name": "Bar", "url": "components/ScrollBar.swf", "symbol": "components.ScrollBar"},
    {"name": "Cell", "url": "components/ListMenuCellLarge.swf", "symbol": "components.ListMenuCellLarge"}
  ],
  "symbols": [
    {"name": "AvaScrollTestCell", "export": true, "cell": "mcCell", "children": [{"instance": "mcCell", "symbol": "Cell"}]},
    {"name": "AvaScrollTest", "export": true, "children": [
      {"instance": "mcMenu", "symbol": "List", "x": 90, "y": 190},
      {"instance": "mcBar", "symbol": "Bar", "x": 441, "y": 190},
      {"instance": "mcRows", "symbol": "List", "x": 500, "y": 190},
      {"instance": "mcRowsBar", "symbol": "Bar", "x": 900, "y": 190}
    ]}
  ],
  "view_symbol": {"symbol": "AvaScrollTest"},
  "logic": {
    "base": "activity_feed_view",
    "data": ["ava_m", "ava_i", "ava_r", "ava_ri", "ava_w"],
    "menus": [
      {"target": "mcMenu", "rows": 5, "cell_width": 350, "cell_height": 60, "spacing": -10, "panel_gaps": [5, -14],
        "wrap": true, "use_panel": true, "data": "ava_m", "index": "ava_i", "cell": "AvaScrollTestCell"},
      {"target": "mcRows", "rows": 9, "cell_width": 300, "cell_height": 36, "spacing": 2,
        "data": "ava_r", "index": "ava_ri", "cell": "AvaScrollTestCell"}
    ],
    "on_enter": {"clear_inputs": true},
    "paging": {"menu": "mcRows", "setup": "A_AvaScrollTestSetup"},
    "scrollbars": [
      {"target": "mcBar", "menu": "mcMenu", "length": 251},
      {"target": "mcRowsBar", "menu": "mcRows", "length": 340, "window": "ava_w"}
    ],
    "inputs": [{"key": "BACK", "menu": "mcMenu", "do": "back", "sound": "button_press_back"}]
  }
}"##;

#[test]
fn scroll_bars_read_and_are_checked() {
    let l = Layout::parse("s.json", SCROLL).unwrap();
    let g = &l.logic;
    assert_eq!((g.base, g.base.class(), g.base.is_fe_view()), (Base::ActivityFeedView, "ssx.FEActivityFeedView", true));
    assert!(matches!(g.menus[0].kind, MenuKind::List { use_panel: true, wrap: true, .. }));
    assert!(matches!(g.menus[1].kind, MenuKind::List { use_panel: false, wrap: false, .. }));
    let bars: Vec<_> =
        g.scrollbars.iter().map(|b| (b.target.as_str(), b.menu.as_str(), b.length, b.window.as_deref())).collect();
    assert_eq!(bars, [("mcBar", "mcMenu", 251.0, None), ("mcRowsBar", "mcRows", 340.0, Some("ava_w"))]);
    let back = InputDo::Menu {
        target: "mcMenu".into(),
        op: MenuOp::Back,
        blocked_sound: None,
        sound: Some("button_press_back"),
    };
    assert_eq!(g.inputs[0].what, back);
    let bad = |from: &str, to: &str, want: &str| {
        assert!(SCROLL.contains(from), "{from}");
        let e = match Layout::parse("s.json", &SCROLL.replacen(from, to, 1)) {
            Ok(_) => panic!("{to}: expected an error"),
            Err(e) => e,
        };
        assert!(e.message.contains(want), "{to}: {e}");
        assert!(e.pos.line > 1, "{e}");
    };
    bad(r#""target": "mcBar""#, r#""target": "mcRows""#, "components.ScrollBar");
    bad(r#""menu": "mcMenu", "length""#, r#""menu": "mcNone", "length""#, "not one of the logic's list menus");
    bad(r#""length": 251"#, r#""length": 0"#, "outside");
    bad(r#""window": "ava_w""#, r#""window": "ava_x""#, "not in the logic's `data`");
    bad(r#""target": "mcRowsBar""#, r#""target": "mcBar""#, "given twice");
    bad(r#""paging": {"menu": "mcRows""#, r#""paging": {"menu": "mcMenu""#, "is not the paging's");
    bad(r#""length": 251}"#, r#""length": 251, "rows": 5}"#, "rows");
    bad(r#""use_panel": true"#, r#""use_panel": 1"#, "use_panel");
    bad(r#""do": "back", "sound""#, r#""do": "up", "blocked_sound": "menu_back", "sound""#, "not with `blocked_sound`");
    bad(r#""base": "activity_feed_view""#, r#""base": "view""#, "`ClearInputs` is `ssx.FEView`'s");
}

#[test]
fn the_4_3_section_reads_and_is_checked() {
    let sd = |s: &str| with(r#""consume_unmapped": true,"#, &format!(r#""consume_unmapped": true, "sd": {s},"#));
    let l = Layout::parse("t.json", &sd(r#"{"left": ["mcPanel"], "right": ["txtEntry"], "scale": 0.8}"#)).unwrap();
    let d = l.logic.sd.unwrap();
    assert_eq!(d.moves, vec![("mcPanel".to_string(), true), ("txtEntry".to_string(), false)]);
    assert_eq!((d.scale, d.centre), (Some(0.8), (640.0, 360.0)));
    assert!(Layout::parse("t.json", GOOD).unwrap().logic.sd.is_none());
    let bad = |s: &str, want: &str| {
        let e = match Layout::parse("t.json", &sd(s)) {
            Ok(_) => panic!("{s}: expected an error"),
            Err(e) => e,
        };
        assert!(e.message.contains(want), "{s}: {e}");
    };
    bad(r#"{"left": ["mcPanel"], "right": ["mcPanel"]}"#, "given twice");
    bad(r#"{"left": ["mcNothing"]}"#, "mcNothing");
    bad(r#"{"scale": 0.3}"#, "outside");
    bad(r#"{"scale": 1.5}"#, "outside");
    bad(r#"{"middle": ["mcPanel"]}"#, "middle");
}
