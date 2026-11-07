use std::path::Path;

use anyhow::Result;
use ssx_content::font::write::{FontDef, GlyphDef, rect, ttf};
use ssx_content::ui::asm::{Asm, Push, op, path};
use ssx_content::ui::write::{Tags, TextField, movie};

fn manifest(name: &str, body: &str) -> String {
    format!(
        r#"# The manifest of the package "{name}" (Project Avalanche; every key: Docs/architecture/content.md §2, §18).
# Check it with `ssx-assets mod check <folder>`, pack it with `ssx-assets mod pack <folder>`.

[resource]
name = "{name}"
version = "0.1.0"
title = "{name}"
authors = []
description = ""
api = 1
type = "mod"
priority = 0
{body}"#
    )
}

const ROWS: u16 = 4;

fn reg(r: u8) -> Push {
    Push::Reg(r)
}

fn s(x: &str) -> Push {
    Push::Str(x.into())
}

fn provider_manager() -> Asm {
    Asm::new().push(&[Push::Int(0), reg(1), s("GetProviderManager")]).op(op::CALL_METHOD)
}

fn item_action(field: &str, param: Asm, label: &str) -> Asm {
    let item = || {
        Asm::new()
            .push(&[reg(1), s("items")])
            .op(op::GET_MEMBER)
            .push(&[reg(1), s("sel")])
            .op(op::GET_MEMBER)
            .op(op::GET_MEMBER)
    };
    Asm::new()
        .then(item())
        .push(&[s(field)])
        .op(op::GET_MEMBER)
        .push(&[Push::Undefined])
        .op(op::EQUALS2)
        .if_true(label)
        .then(param)
        .then(item())
        .push(&[s(field)])
        .op(op::GET_MEMBER)
        .push(&[Push::Int(2)])
        .then(provider_manager())
        .push(&[s("Action")])
        .op(op::CALL_METHOD)
        .op(op::POP)
        .label(label)
}

fn class_code() -> Vec<u8> {
    const THIS: u16 = 0x0100;
    const SUPER: u16 = 0x1000;
    let ctor = Asm::new().push(&[Push::Int(0), reg(1), Push::Undefined]).op(op::CALL_METHOD).op(op::POP);
    let mut init = Asm::new()
        .push(&[reg(3), Push::Int(1), reg(2), s("Initialize")])
        .op(op::CALL_METHOD)
        .op(op::POP)
        .push(&[reg(1), s("sel"), Push::Int(0)])
        .op(op::SET_MEMBER)
        .push(&[reg(1), s("items"), Push::Int(0), s("Array")])
        .op(op::NEW_OBJECT)
        .op(op::SET_MEMBER);
    for data in ["debug_menu", "debug_menu_index"] {
        init = init
            .push(&[s(data), Push::Int(1)])
            .then(provider_manager())
            .push(&[s("AddDataProvider")])
            .op(op::CALL_METHOD)
            .op(op::POP);
    }
    let mut publish = Asm::new();
    for (data, member, l) in [("debug_menu", "items", "not_items"), ("debug_menu_index", "sel", "not_index")] {
        publish = publish
            .push(&[reg(2), s(data)])
            .op(op::EQUALS2)
            .op(op::NOT)
            .if_true(l)
            .push(&[reg(1), s(member), reg(3)])
            .op(op::SET_MEMBER)
            .push(&[Push::Int(0), reg(1), s("Redraw")])
            .op(op::CALL_METHOD)
            .op(op::POP)
            .label(l);
    }
    let mut redraw = Asm::new();
    for k in 0..ROWS {
        let k = i32::from(k);
        let (empty, mark, name, set) = (format!("e{k}"), format!("m{k}"), format!("n{k}"), format!("s{k}"));
        redraw = redraw
            .push(&[reg(1), s("items")])
            .op(op::GET_MEMBER)
            .push(&[Push::Int(k)])
            .op(op::GET_MEMBER)
            .store(2)
            .op(op::POP)
            .push(&[reg(1), Push::Str(format!("txtItem{k}"))])
            .op(op::GET_MEMBER)
            .push(&[s("text"), reg(2), Push::Undefined])
            .op(op::EQUALS2)
            .if_true(&empty)
            .push(&[reg(1), s("sel")])
            .op(op::GET_MEMBER)
            .push(&[Push::Int(k)])
            .op(op::EQUALS2)
            .if_true(&mark)
            .push(&[s("   ")])
            .jump(&name)
            .label(&mark)
            .push(&[s("> ")])
            .label(&name)
            .push(&[reg(2), s("strItem")])
            .op(op::GET_MEMBER)
            .op(op::ADD2)
            .jump(&set)
            .label(&empty)
            .push(&[s("")])
            .label(&set)
            .op(op::SET_MEMBER);
    }
    let code_is = |via: Asm, label: &str| Asm::new().push(&[reg(3)]).then(via).op(op::STRICT_EQUALS).if_true(label);
    let util = |method: &str| {
        Asm::new().push(&[Push::Int(0)]).then(path("utility.ViewUtil")).push(&[s(method)]).op(op::CALL_METHOD)
    };
    let key = |name: &str| path(&format!("ux.input.InputCode.{name}"));
    let step = |dir: u8, wrap: &str, done: &str| {
        let mut a =
            Asm::new().push(&[reg(1), s("sel"), reg(1), s("sel")]).op(op::GET_MEMBER).op(dir).op(op::SET_MEMBER);
        if dir == op::INCREMENT {
            a = a
                .push(&[reg(1), s("sel")])
                .op(op::GET_MEMBER)
                .push(&[reg(1), s("items")])
                .op(op::GET_MEMBER)
                .push(&[s("length")])
                .op(op::GET_MEMBER)
                .op(op::LESS2)
                .if_true(wrap)
                .push(&[reg(1), s("sel"), Push::Int(0)])
                .op(op::SET_MEMBER);
        } else {
            a = a
                .push(&[reg(1), s("sel")])
                .op(op::GET_MEMBER)
                .push(&[Push::Int(0)])
                .op(op::LESS2)
                .op(op::NOT)
                .if_true(wrap)
                .push(&[reg(1), s("sel"), reg(1), s("items")])
                .op(op::GET_MEMBER)
                .push(&[s("length")])
                .op(op::GET_MEMBER)
                .op(op::DECREMENT)
                .op(op::SET_MEMBER);
        }
        let param = Asm::new()
            .push(&[reg(1), s("items")])
            .op(op::GET_MEMBER)
            .push(&[reg(1), s("sel")])
            .op(op::GET_MEMBER)
            .op(op::GET_MEMBER)
            .push(&[s("eventHighlightParam")])
            .op(op::GET_MEMBER);
        a.label(wrap)
            .push(&[Push::Int(0), reg(1), s("Redraw")])
            .op(op::CALL_METHOD)
            .op(op::POP)
            .then(item_action("eventHighlight", param, done))
            .push(&[Push::Bool(true)])
            .op(op::RETURN)
    };
    let sel = Asm::new().push(&[reg(1), s("sel")]).op(op::GET_MEMBER);
    let input = Asm::new()
        .push(&[reg(2), s("KeyCode")])
        .op(op::GET_MEMBER)
        .store(3)
        .op(op::POP)
        .then(code_is(util("GetAdvanceInputCode"), "advance"))
        .then(code_is(util("GetBackInputCode"), "back"))
        .then(code_is(key("DPAD_DOWN"), "down"))
        .then(code_is(key("DPAD_UP"), "up"))
        .push(&[Push::Bool(false)])
        .op(op::RETURN)
        .label("advance")
        .then(item_action("eventAdvance", sel, "advanced"))
        .push(&[Push::Bool(true)])
        .op(op::RETURN)
        .label("back")
        .then(item_action("eventBack", Asm::new().push(&[Push::Int(0)]), "backed"))
        .push(&[Push::Bool(true)])
        .op(op::RETURN)
        .label("down")
        .then(step(op::INCREMENT, "down_ok", "down_done"))
        .label("up")
        .then(step(op::DECREMENT, "up_ok", "up_done"));
    Asm::new()
        .then(path("_global"))
        .push(&[s("ModMainMenu")])
        .function2("", &[], 2, SUPER | 0x0800 | 0x0200, ctor)
        .store(1)
        .op(op::SET_MEMBER)
        .push(&[reg(1)])
        .then(path("ux.view.View"))
        .op(op::EXTENDS)
        .push(&[reg(1), s("prototype")])
        .op(op::GET_MEMBER)
        .store(2)
        .op(op::POP)
        .push(&[reg(2), s("Initialize")])
        .function2("", &[(3, "inputManager")], 4, THIS | SUPER, init)
        .op(op::SET_MEMBER)
        .push(&[reg(2), s("Publish")])
        .function2("", &[(2, "varName"), (3, "value")], 4, THIS, publish)
        .op(op::SET_MEMBER)
        .push(&[reg(2), s("Redraw")])
        .function2("", &[], 3, THIS, redraw)
        .op(op::SET_MEMBER)
        .push(&[reg(2), s("HandleInput")])
        .function2("", &[(2, "input")], 4, THIS, input)
        .op(op::SET_MEMBER)
        .push(&[reg(1), s("ModMainMenu"), Push::Int(2)])
        .then(path("Object"))
        .push(&[s("registerClass")])
        .op(op::CALL_METHOD)
        .op(op::POP)
        .finish()
}

fn field(id: u16, h: u16, text: &str, rgba: [u8; 4]) -> TextField<'_> {
    TextField { id, bounds: (0, 9000, 0, i32::from(h) + 200), font_id: 1, height: h, rgba, text, html: false, align: 0 }
}

pub fn main_menu_movie(package: &str) -> Vec<u8> {
    const VIEW: u16 = 10;
    let mut view = Tags::new();
    let mut t = Tags::new();
    t.font_placeholder(1, "DIN Mittelschrift Std");
    t.rect_shape(2, 9600, 6200, [8, 24, 40, 200]);
    t.rect_shape(3, 9600, 60, [120, 220, 255, 255]);
    t.edit_text(&field(4, 640, "MAIN MENU (MOD)", [255, 255, 255, 255]));
    for k in 0..ROWS {
        t.edit_text(&field(5 + k, 480, "", [195, 253, 249, 255]));
    }
    let footer = format!("A screen of the package {package}: up / down, A, B");
    t.edit_text(&field(9, 320, &footer, [150, 180, 200, 255]));
    view.place(1, 2, Some("mcBack"), 0, 0).place(2, 3, Some("mcLine"), 0, 1100).place(3, 4, Some("txtTitle"), 400, 300);
    for k in 0..ROWS {
        view.place(4 + k, 5 + k, Some(&format!("txtItem{k}")), 400, 1500 + i32::from(k) * 900);
    }
    view.place(9, 9, Some("txtFooter"), 400, 5400);
    view.show_frame();
    t.sprite(VIEW, &view);
    t.export(&[(VIEW, "ModMainMenu")]);
    t.do_init_action(VIEW, &class_code());
    t.place(1, VIEW, Some("mView"), 4800, 1600);
    t.show_frame();
    movie(8, 1280, 720, 30, &t, true)
}

type Contour = Vec<(i16, i16, bool)>;

pub fn demo_font(family: &str) -> Vec<u8> {
    let mut f = FontDef::new(family, 1000);
    f.glyph(' ', GlyphDef::Empty, 300);
    let shapes: [(char, Vec<Contour>); 8] = [
        (
            'Δ',
            vec![
                vec![(0, 0, true), (300, 700, true), (600, 0, true)],
                vec![(150, 120, true), (450, 120, true), (300, 470, true)],
            ],
        ),
        (
            'Λ',
            vec![vec![
                (0, 0, true),
                (240, 700, true),
                (360, 700, true),
                (600, 0, true),
                (480, 0, true),
                (300, 520, true),
                (120, 0, true),
            ]],
        ),
        ('Π', vec![rect(0, 0, 120, 700), rect(480, 0, 120, 700), rect(0, 580, 600, 120)]),
        (
            'Σ',
            vec![
                rect(0, 0, 600, 120),
                rect(0, 580, 600, 120),
                vec![
                    (0, 120, true),
                    (260, 350, true),
                    (0, 580, true),
                    (160, 580, true),
                    (400, 350, true),
                    (160, 120, true),
                ],
            ],
        ),
        (
            'Ж',
            vec![
                rect(240, 0, 120, 700),
                vec![
                    (0, 0, true),
                    (240, 300, true),
                    (240, 400, true),
                    (0, 700, true),
                    (130, 700, true),
                    (240, 420, true),
                    (240, 280, true),
                    (130, 0, true),
                ],
            ],
        ),
        ('Д', vec![rect(0, 0, 600, 120), rect(80, 120, 120, 580), rect(400, 120, 120, 580), rect(80, 580, 440, 120)]),
        (
            'Ł',
            vec![
                rect(100, 0, 120, 700),
                rect(100, 0, 500, 120),
                vec![(0, 300, true), (0, 400, true), (360, 520, true), (360, 420, true)],
            ],
        ),
        (
            'Ż',
            vec![
                rect(0, 0, 600, 120),
                rect(0, 580, 600, 120),
                vec![(0, 120, true), (440, 580, true), (600, 580, true), (160, 120, true)],
                rect(240, 780, 120, 120),
            ],
        ),
    ];
    for (c, contours) in shapes {
        f.glyph(c, GlyphDef::Simple(contours), 680);
    }
    ttf(&f)
}

const UI_README: &str = "A screen of your own\n====================\nui/mainmenu.swf replaces the game's main menu \
(ssx:ui/views/mainmenu): a view of the game's framework class (ux.view.View) that shows the main menu's items from the \
game's data and makes the main menu's calls on A, B and up / down. Edit it in a SWF editor (JPEXS opens it), or make \
your own; `ssx-assets content export ssx:ui/views/<screen>` writes a screen of your disc as a starting point for your \
own work (exports may not be shipped: `mod check` refuses them). A movie is a program: list every movie in \
[actionscript] files; its ActionScript runs in the game's player within the sandbox's caps (a movie that passes one \
is stopped and its screen plays the game's own from its next load).\n";

pub fn ui(root: &Path, name: &str) -> Result<()> {
    std::fs::create_dir(root.join("ui"))?;
    std::fs::write(root.join("ui").join("mainmenu.swf"), main_menu_movie(name))?;
    let body = r#"
# A screen replaced by a movie of your own (SWF or GFX; `ssx-assets content ids --kind ui` lists the screens).
[override]
"ssx:ui/views/mainmenu" = "ui/mainmenu.swf"

# Every movie is a program the game's player runs: declare each (the Mods page says "contains ActionScript"; a server
# does not run a package's movies).
[actionscript]
files = ["ui/*.swf"]
"#;
    std::fs::write(root.join(ssx_content::manifest::MANIFEST_FILE), manifest(name, body))?;
    std::fs::write(root.join("README.txt"), UI_README)?;
    Ok(())
}

pub fn strings(root: &Path, name: &str) -> Result<()> {
    std::fs::create_dir(root.join("strings"))?;
    std::fs::write(
        root.join("strings").join("en.toml"),
        "# The package's strings in English (a key of the game's: `TXT_…`, any case; any other key is the package's \
         own,\n# looked up by its id \"<package>:string/<key>\"). A language without a key falls back to English, then \
         to the game's.\n# Keep a game's string's placeholders (%1s, %2d …) and button tokens (^A^ …): mod check \
         compares them.\nTXT_M_GLOBE_EXPLORE = \"FREE RIDE\"\nTXT_MAIN_MENU_EXPLORE_PROGRESS = \"Medals on %1d of %2d \
         drops\"\nTXT_HUD_TRICK_COMBO_DEPOSIT = \"%1s COMBO BANKED\"\nTXT_REMAINING = \"to go\"\ngreeting = \"Hello from a package\"\n",
    )?;
    std::fs::write(
        root.join("strings").join("de.toml"),
        "# The package's strings in German (strings/de.toml); the game's languages: en fr de es it ja.\n\
         TXT_M_GLOBE_EXPLORE = \"FREIE FAHRT\"\nTXT_HUD_TRICK_COMBO_DEPOSIT = \"%1s KOMBO GESICHERT\"\nTXT_REMAINING = \"übrig\"\n",
    )?;
    std::fs::write(
        root.join(ssx_content::manifest::MANIFEST_FILE),
        manifest(name, "\n# The string tables are the files strings/<language>.toml: no entry names them.\n"),
    )?;
    Ok(())
}

pub fn font(root: &Path, name: &str) -> Result<()> {
    std::fs::create_dir(root.join("fonts"))?;
    std::fs::write(root.join("fonts").join("demo.ttf"), demo_font(&format!("{name} demo")))?;
    let body = r#"
# A font of your own (TrueType or OpenType, `.ttf` / `.otf`): added, its characters drawn wherever the game's fonts lack
# them (names, chat, a translation's letters), in the packages' order; or replacing a font of the game's font library by
# its id (`ssx-assets content ids --kind font`). The font must be yours to share (its licence).
[add]
"font/demo" = "fonts/demo.ttf"
"#;
    std::fs::write(root.join(ssx_content::manifest::MANIFEST_FILE), manifest(name, body))?;
    Ok(())
}
