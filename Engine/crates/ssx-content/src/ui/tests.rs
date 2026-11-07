use super::asm::{Asm, Push, op};
use super::write::{Tags, TextField, movie};
use super::*;

fn screen() -> Tags {
    let mut t = Tags::new();
    t.background([0, 0, 0]);
    t.font_placeholder(1, "DIN Engschrift Std");
    t.rect_shape(2, 2000, 400, [255, 0, 0, 255]);
    t.edit_text(&TextField {
        id: 3,
        bounds: (0, 4000, 0, 600),
        font_id: 1,
        height: 480,
        rgba: [255, 255, 255, 255],
        text: "HELLO",
        html: false,
        align: 0,
    });
    let mut inner = Tags::new();
    inner.place(1, 2, Some("mcBack"), 0, 0).place(2, 3, Some("txtLabel"), 100, 100).show_frame();
    t.sprite(4, &inner);
    t.export(&[(4, "ModView")]);
    let code = Asm::new()
        .pool(&["trace", "hello"])
        .push(&[Push::Const(1)])
        .op(op::TRACE)
        .push(&[Push::Int(0)])
        .label("loop")
        .op(op::INCREMENT)
        .op(op::PUSH_DUPLICATE)
        .push(&[Push::Int(3)])
        .op(op::LESS2)
        .if_true("loop")
        .op(op::POP)
        .finish();
    t.do_init_action(4, &code);
    t.place(1, 4, Some("mView"), 0, 0);
    t.do_action(&Asm::new().op(op::STOP).finish());
    t.show_frame();
    t
}

#[test]
fn our_movies_parse_in_every_container() {
    for zlib in [false, true] {
        let m = movie(8, 1280, 720, 30, &screen(), zlib);
        let info = check_movie(&m, &Limits::default()).unwrap();
        assert_eq!(info.container, Some(if zlib { Container::Cws } else { Container::Fws }));
        assert_eq!(info.version, 8);
        assert_eq!(info.blocks, 2);
        assert!(info.has_actionscript());
        assert_eq!(info.exports, ["ModView"]);
        assert_eq!(info.fonts, ["DIN Engschrift Std"]);
        let mut g = m.clone();
        g[..3].copy_from_slice(if zlib { b"CFX" } else { b"GFX" });
        assert!(check_movie(&g, &Limits::default()).is_ok());
    }
}

#[test]
fn lzma_versions_and_actionscript_3_are_refused() {
    let mut m = movie(8, 640, 480, 30, &screen(), false);
    m[..3].copy_from_slice(b"ZWS");
    assert!(check_movie(&m, &Limits::default()).unwrap_err().contains("LZMA"));
    for v in [6u8, 11, 40] {
        let m = movie(v, 640, 480, 30, &screen(), false);
        assert!(check_movie(&m, &Limits::default()).unwrap_err().contains("SWF version"));
    }
    let mut t = Tags::new();
    t.tag(69, &0x08u32.to_le_bytes()).show_frame();
    assert!(check_movie(&movie(10, 64, 64, 30, &t, false), &Limits::default()).unwrap_err().contains("ActionScript 3"));
    let mut t = Tags::new();
    t.tag(82, &[0; 8]).show_frame();
    assert!(check_movie(&movie(10, 64, 64, 30, &t, false), &Limits::default()).unwrap_err().contains("DoABC"));
    assert!(check_movie(b"PK\x03\x04", &Limits::default()).unwrap_err().contains("not a movie"));
}

#[test]
fn a_zlib_bomb_stops_at_the_cap() {
    let body = vec![0u8; 64 << 20];
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    std::io::Write::write_all(&mut z, &body).unwrap();
    let mut m = b"CWS\x08".to_vec();
    m.extend_from_slice(&1000u32.to_le_bytes());
    m.extend(z.finish().unwrap());
    let e = check_movie(&m, &Limits::default()).unwrap_err();
    assert!(e.contains("inflates past"), "{e}");
    let mut m = movie(8, 64, 64, 30, &screen(), true);
    m[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(check_movie(&m, &Limits::default()).unwrap_err().contains("header states"));
}

#[test]
fn wild_jumps_and_caps_are_refused() {
    let mut t = Tags::new();
    t.do_action(&[0x99, 2, 0, 0xFE, 0xFF, 0x00]).show_frame();
    assert!(check_movie(&movie(8, 64, 64, 30, &t, false), &Limits::default()).unwrap_err().contains("branch"));
    let mut t = Tags::new();
    t.do_action(&[0x9B, 5, 0, 0, 0, 0, 0xFF, 0x7F, 0x00]).show_frame();
    assert!(check_movie(&movie(8, 64, 64, 30, &t, false), &Limits::default()).is_err());
    let mut t = Tags::new();
    let mut code = vec![op::STOP; 5000];
    code.push(op::END);
    t.do_action(&code).show_frame();
    let caps = Limits { max_actions: 1000, ..Limits::default() };
    assert!(check_movie(&movie(8, 64, 64, 30, &t, false), &caps).unwrap_err().contains("action records"));
    let mut t = Tags::new();
    let mut d = 7u16.to_le_bytes().to_vec();
    d.extend_from_slice(&u16::MAX.to_le_bytes());
    d.extend_from_slice(&[0, 0]);
    t.tag(39, &d).show_frame();
    let caps = Limits { max_frames: 1000, ..Limits::default() };
    assert!(check_movie(&movie(8, 64, 64, 30, &t, false), &caps).unwrap_err().contains("timeline frames"));
}

#[test]
fn unplayed_tags_are_listed() {
    let mut t = Tags::new();
    t.tag(20, &[1, 0, 5, 1, 0, 1, 0]).tag(14, &[2, 0, 0]).show_frame();
    let info = check_movie(&movie(8, 64, 64, 30, &t, false), &Limits::default()).unwrap();
    let names: Vec<&str> = info.unplayed.iter().map(|u| u.0).collect();
    assert_eq!(names, ["DefineSound", "DefineBitsLossless"]);
    assert!(!info.has_actionscript());
}

#[test]
fn ids_and_paths() {
    let id: ContentId = "ssx:ui/views/mainmenu".parse().unwrap();
    assert_eq!(movie_path(&id).unwrap(), "data/common/fe/views/mainmenu.gfx");
    assert_eq!(movie_id("data/common/fe/views/mainmenu.gfx", false), Some(id));
    let added: ContentId = "neon:ui/screens/menu".parse().unwrap();
    assert_eq!(movie_path(&added).unwrap(), "data/common/fe/mods/neon/screens/menu.gfx");
    assert_eq!(movie_id("data/common/fe/mods/neon/screens/menu.swf", false), Some(added));
    assert_eq!(movie_id("data/common/fe/views/ava_hub.gfx", true).unwrap().as_str(), "ava:ui/views/ava_hub");
    for bad in [
        "data/common/fe/../../etc/passwd.gfx",
        "data/common/fe/mods/ssx/views/x.gfx",
        "data/common/fe/mods/ava/x.gfx",
        "data/common/fe/Views/X.gfx",
        "data/common/fe/views/x.dds",
        "/etc/x.gfx",
        "data/common/fe/mods/neon.gfx",
    ] {
        assert_eq!(movie_id(bad, false), None, "{bad}");
    }
}
