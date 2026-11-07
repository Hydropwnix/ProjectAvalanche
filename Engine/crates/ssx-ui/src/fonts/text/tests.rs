use super::*;
use crate::display::tests::{empty_lib, movie};
use crate::library::Character;
use ssx_formats::gfx::shape::Shape;
use ssx_formats::gfx::text::FontLayout;
use ssx_formats::gfx::types::Rect;

#[test]
fn html_subset_and_entities() {
    let ch = CharFormat {
        face: None,
        size: 240.0,
        color: [0.0, 0.0, 0.0, 1.0],
        letter_spacing: 0.0,
        kerning: false,
        bold: false,
        italic: false,
        underline: false,
    };
    let para = ParaFormat { align: Align::Left, left_margin: 0.0, right_margin: 0.0, indent: 0.0, leading: 40.0 };
    let src = r##"<p align="right"><font face="DIN Engschrift Std" size="20" color="#FF8000" letterSpacing="1.000000" kerning="1">it&apos;s&nbsp;00</font></p><p align="center">x<br>y</p>"##;
    let p = parse_html(src, &ch, &para);
    assert_eq!(p.len(), 3);
    assert_eq!(p[0].format.align, Align::Right);
    let r = &p[0].runs[0];
    assert_eq!(r.text, "it's\u{a0}00");
    assert_eq!(r.format.face.as_deref(), Some("DIN Engschrift Std"));
    assert_eq!((r.format.size, r.format.letter_spacing, r.format.kerning), (400.0, 20.0, true));
    assert_eq!(r.format.color, [1.0, 128.0 / 255.0, 0.0, 1.0]);
    assert_eq!((p[1].runs[0].text.as_str(), p[2].runs[0].text.as_str()), ("x", "y"));
    assert_eq!(p[2].format.align, Align::Center);
    assert_eq!(decode_entities("a &amp; &#65;&#x42; &bogus"), "a & AB &bogus");
}

#[test]
fn html_color_keeps_the_alpha() {
    let ch = CharFormat {
        face: None,
        size: 240.0,
        color: [0.0, 0.0, 0.0, 0.6],
        letter_spacing: 0.0,
        kerning: false,
        bold: false,
        italic: false,
        underline: false,
    };
    let para = ParaFormat { align: Align::Left, left_margin: 0.0, right_margin: 0.0, indent: 0.0, leading: 0.0 };
    let p = parse_html(r##"<font color="#dbf0f8">x0</font>"##, &ch, &para);
    assert_eq!(p[0].runs[0].format.color, [219.0 / 255.0, 240.0 / 255.0, 248.0 / 255.0, 0.6]);
    let p = parse_html(r##"<font alpha="#4C" color="#FF0000">a</font>"##, &ch, &para);
    assert_eq!(p[0].runs[0].format.color, [1.0, 0.0, 0.0, 76.0 / 255.0]);
    let p = parse_html(r##"<font color="FF0000">b</font><font color="#GG0000">c</font>"##, &ch, &para);
    assert_eq!(p[0].runs[0].format.color, ch.color);
    assert_eq!(p[0].runs[1].format.color, ch.color);
}

#[test]
fn glyph_ink_right_edge() {
    let f = test_font();
    assert_eq!(glyph_ink_right(&f, 0), 0.0);
    assert_eq!(glyph_ink_right(&f, 1), 512.0);
    assert_eq!(glyph_ink_right(&f, 7), 0.0);
}

pub(crate) fn test_font() -> Font {
    let shape = crate::display::tests::rect_shape(0, 512, 700).shape;
    Font {
        version: 2,
        id: 50,
        flags: 0x80,
        language: 0,
        name: "Test".into(),
        glyphs: vec![Shape::default(), Shape { styles: Default::default(), records: shape.records }],
        codes: vec![u16::from(b' '), u16::from(b'A')],
        layout: Some(FontLayout {
            ascent: 800,
            descent: 200,
            leading: 0,
            advances: vec![300, 600],
            bounds: vec![Rect::default(); 2],
            kerning: Vec::new(),
        }),
        unk_trailing_layout: None,
    }
}

#[test]
fn alignment_and_word_wrap() {
    let mut lib = empty_lib();
    let m = movie(&mut lib, vec![vec![]]);
    let font = Arc::new(test_font());
    let mut def = lib.movie(m).clone();
    def.path = "fonts.gfx".into();
    def.chars.insert(50, Character::Font(font.clone()));
    let m = lib.insert_movie(def);
    let ch = CharFormat {
        face: None,
        size: 1024.0,
        color: [1.0; 4],
        letter_spacing: 0.0,
        kerning: false,
        bold: false,
        italic: false,
        underline: false,
    };
    let para = |align| ParaFormat { align, left_margin: 0.0, right_margin: 0.0, indent: 0.0, leading: 100.0 };
    let bounds = RectF::new(0.0, 0.0, 2080.0, 5000.0);
    let l = layout(&lib, m, Some(50), &plain("AA", &ch, &para(Align::Right)), &ch, bounds, false);
    assert_eq!(l.glyphs.len(), 2);
    assert_eq!(l.glyphs[0].x, 2040.0 - 1200.0);
    assert_eq!(l.glyphs[0].y, 40.0 + 800.0);
    let l = layout(&lib, m, Some(50), &plain("AA", &ch, &para(Align::Center)), &ch, bounds, false);
    assert_eq!(l.glyphs[0].x, 40.0 + (2000.0 - 1200.0) / 2.0);
    let l = layout(&lib, m, Some(50), &plain("AA AA AA", &ch, &para(Align::Left)), &ch, bounds, true);
    assert_eq!(l.lines, 3);
    assert_eq!(l.glyphs[2].x, 40.0);
    assert_eq!(l.glyphs[2].y, 40.0 + 800.0 + 200.0 + 100.0 + 800.0);
    assert_eq!(l.text_width, 1200.0);
    assert_eq!(l.text_height, 3.0 * 1000.0 + 2.0 * 100.0);
}

#[test]
fn the_discs_texts_keep_the_games_layout() {
    for t in
        ["\u{f8ff}4,500", "Zoë Łódź €™ – “quoted”…", "Γεια Привет", "ラドクリフ、五輪代表", "1\u{a0}000", "\u{2009}et"]
    {
        assert!(!super::needs_complex_layout(t), "{t:?}");
    }
    for t in ["שלום", "e\u{301}", "नमस्ते", "สวัสดี", "١٢٣"] {
        assert!(super::needs_complex_layout(t), "{t:?}");
    }
}

#[test]
fn only_outside_text_takes_the_fallback_fonts() {
    let mut lib = empty_lib();
    let m = movie(&mut lib, vec![vec![]]);
    let mut def = lib.movie(m).clone();
    def.path = "fonts.gfx".into();
    def.chars.insert(50, Character::Font(Arc::new(test_font())));
    let m = lib.insert_movie(def);
    let ch = CharFormat {
        face: None,
        size: 1024.0,
        color: [1.0; 4],
        letter_spacing: 0.0,
        kerning: false,
        bold: false,
        italic: false,
        underline: false,
    };
    let para = ParaFormat { align: Align::Left, left_margin: 0.0, right_margin: 0.0, indent: 0.0, leading: 0.0 };
    let bounds = RectF::new(0.0, 0.0, 20_000.0, 5000.0);
    let l = layout(&lib, m, Some(50), &plain("A\u{416}A", &ch, &para), &ch, bounds, false);
    assert_eq!(l.glyphs.len(), 2);
    assert_eq!(l.glyphs[1].x - l.glyphs[0].x, 600.0 + 512.0);
    let l = layout(&lib, m, Some(50), &plain("A\tA", &ch, &para), &ch, bounds, false);
    assert_eq!(l.glyphs[1].x - l.glyphs[0].x, 600.0);
    let l = layout(&lib, m, Some(50), &plain("A\u{5E9}A", &ch, &para), &ch, bounds, false);
    assert_eq!(l.glyphs.len(), 2);
    assert_eq!(l.glyphs[1].x - l.glyphs[0].x, 600.0 + 512.0);
    let l = layout(&lib, m, Some(50), &plain(&isolate::wrap("A\u{416}A"), &ch, &para), &ch, bounds, false);
    assert!(!l.pending);
    assert_eq!(l.glyphs.len(), 3);
    assert!(l.glyphs[2].x - l.glyphs[0].x > 600.0);
}

#[test]
fn the_disc_fallbacks_map_answers_as_the_scan() {
    let font = |codes: Vec<u16>| Arc::new(Font { codes, ..test_font() });
    let fonts = vec![
        font(vec![0x41, 0x42, 0x416]),
        font(vec![0x5E9, 0x41, 0x30, 0x5E9]),
        font(vec![0xD800, 0x416, 0x4E00, 0x43]),
    ];
    let map = crate::library::first_glyphs(&fonts);
    let scan = |c: char| fonts.iter().enumerate().find_map(|(i, f)| super::glyph_index(f, c).map(|g| (i, g)));
    let found = |c: char| u16::try_from(u32::from(c)).ok().and_then(|code| map.get(&code).copied());
    for c in ['A', 'B', 'C', '0', '\u{416}', '\u{5E9}', '\u{4E00}', 'Z', '\u{FFFF}', '\u{1F600}', '\u{10000}'] {
        assert_eq!(found(c), scan(c), "{c:?}");
    }
    assert_eq!(found('\u{416}'), Some((0, 2)));
    assert_eq!(found('\u{5E9}'), scan('\u{5E9}'));
    assert_eq!(map.len(), 7);
}
