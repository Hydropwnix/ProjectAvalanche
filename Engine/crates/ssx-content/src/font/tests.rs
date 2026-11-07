use super::write::{FontDef, GlyphDef, PairSubtable, rect, ttf};
use super::*;

fn sample() -> FontDef {
    let mut f = FontDef::new("Sample Sans", 1000);
    f.glyph(' ', GlyphDef::Empty, 250);
    let a = f.glyph('A', GlyphDef::Simple(vec![rect(0, 0, 500, 700)]), 600);
    let mut hole = rect(100, 100, 300, 500);
    hole.reverse();
    let o = f.glyph('Ω', GlyphDef::Simple(vec![rect(0, 0, 500, 700), hole]), 600);
    f.glyph(
        'Ж',
        GlyphDef::Simple(vec![vec![
            (0, 0, true),
            (250, 700, false),
            (500, 0, true),
            (250, -100, false),
            (100, -50, false),
        ]]),
        550,
    );
    f.glyph('Ł', GlyphDef::Composite(vec![(a, 0, 0), (o, 600, 0)]), 1200);
    f.kern.push((a, o, -50));
    f
}

#[test]
fn a_font_converts_to_the_players_glyphs() {
    let c = convert(&ttf(&sample()), "pack:font/sample", &Limits::default()).unwrap();
    let f = &c.font;
    assert_eq!((c.info.family.as_str(), c.info.units_per_em, c.info.outlines), ("Sample Sans", 1000, "glyf"));
    assert_eq!(f.codes, [0x20, 0x41, 0x141, 0x3A9, 0x416]);
    assert_eq!((f.version, f.name.as_str()), (3, "pack:font/sample"));
    let l = f.layout.as_ref().unwrap();
    assert_eq!((l.ascent, l.descent), (16384, 4096));
    assert_eq!(l.advances, [5120, 12288, 24576, 12288, 11264]);
    assert_eq!(l.kerning, [(0x41, 0x3A9, -1024)]);
    let a = &f.glyphs[1].records;
    assert_eq!(a.len(), 5);
    assert!(matches!(
        a[0],
        ssx_formats::gfx::shape::ShapeRecord::StyleChange { move_to: Some((0, 0)), fill1: Some(1), .. }
    ));
    assert_eq!(l.bounds[1], ssx_formats::gfx::types::Rect { x_min: 0, x_max: 10240, y_min: -14336, y_max: 0 });
    let paths = f.glyphs[2]
        .records
        .iter()
        .filter(|r| matches!(r, ssx_formats::gfx::shape::ShapeRecord::StyleChange { .. }))
        .count();
    assert_eq!(paths, 3);
    assert!(f.glyphs[4].records.iter().any(|r| matches!(r, ssx_formats::gfx::shape::ShapeRecord::Curved { .. })));
    assert_eq!(c.info.mapped_glyphs, 5);
}

#[test]
fn composite_cycles_depths_and_fans_are_refused() {
    let mut f = FontDef::new("Cycle", 1000);
    f.glyphs.push((GlyphDef::Composite(vec![(2, 0, 0)]), 500));
    f.glyphs.push((GlyphDef::Composite(vec![(1, 0, 0)]), 500));
    f.cmap.push((0x41, 1));
    assert!(convert(&ttf(&f), "x", &Limits::default()).unwrap_err().contains("cycle"));
    let mut f = FontDef::new("Deep", 1000);
    f.glyphs.push((GlyphDef::Simple(vec![rect(0, 0, 10, 10)]), 500));
    for i in 1..40u16 {
        f.glyphs.push((GlyphDef::Composite(vec![(i, 0, 0)]), 500));
    }
    f.cmap.push((0x41, 40));
    assert!(convert(&ttf(&f), "x", &Limits::default()).unwrap_err().contains("deeper"));
    let mut f = FontDef::new("Fan", 1000);
    f.glyphs.push((GlyphDef::Simple(vec![rect(0, 0, 10, 10)]), 500));
    for i in 1..31u16 {
        f.glyphs.push((GlyphDef::Composite(vec![(i, 0, 0), (i, 5, 0)]), 500));
    }
    f.cmap.push((0x41, 31));
    let e = convert(&ttf(&f), "x", &Limits::default()).unwrap_err();
    assert!(e.contains("components or"), "{e}");
}

#[test]
fn counts_and_sizes_are_capped() {
    let mut f = FontDef::new("Bomb", 1000);
    let pts: Vec<(i16, i16, bool)> = (0..20_000).map(|i| ((i % 1000) as i16, (i / 1000) as i16, true)).collect();
    f.glyph('A', GlyphDef::Simple(vec![pts]), 500);
    assert!(convert(&ttf(&f), "x", &Limits::default()).unwrap_err().contains("points"));
    let mut f = FontDef::new("Many", 1000);
    let g = f.glyph('A', GlyphDef::Simple(vec![rect(0, 0, 10, 10)]), 500);
    for c in 0x100u16..0x100 + 300 {
        f.cmap.push((c, g));
    }
    let caps = Limits { max_codes: 200, ..Limits::default() };
    assert!(convert(&ttf(&f), "x", &caps).unwrap_err().contains("characters mapped"));
    let mut f = FontDef::new("Ranges", 1000);
    f.glyph('A', GlyphDef::Simple(vec![rect(0, 0, 10, 10)]), 500);
    f.cmap12.push((0, 0x10_FFFF, 1));
    let c = convert(&ttf(&f), "x", &Limits::default()).unwrap();
    assert_eq!(c.font.codes, [0]);
    let mut f = sample();
    f.claimed_glyphs = Some(60_000);
    let c = convert(&ttf(&f), "x", &Limits::default()).unwrap();
    assert_eq!(c.font.codes, [0x20, 0x41, 0x141, 0x3A9, 0x416]);
    let caps = Limits { max_file_bytes: 100, ..Limits::default() };
    assert!(convert(&ttf(&sample()), "x", &caps).unwrap_err().contains("bytes"));
    assert!(convert(b"ttcf\0\x01\0\0", "x", &Limits::default()).unwrap_err().contains("collection"));
    assert!(convert(&[0x55; 64], "x", &Limits::default()).is_err());
}

#[test]
fn damaged_tables_fail_without_a_panic() {
    let good = ttf(&sample());
    for i in 0..good.len() {
        let mut b = good.clone();
        b[i] ^= 0xFF;
        let _ = convert(&b, "x", &Limits::default());
        let _ = convert(&good[..i], "x", &Limits::default());
    }
}

#[test]
fn gpos_kerning_follows_the_shapers_rules() {
    let mut f = FontDef::new("Kerned", 1000);
    let a = f.glyph('A', GlyphDef::Simple(vec![rect(0, 0, 500, 700)]), 600);
    let b = f.glyph('B', GlyphDef::Simple(vec![rect(0, 0, 500, 700)]), 600);
    let c = f.glyph('C', GlyphDef::Simple(vec![rect(0, 0, 500, 700)]), 600);
    f.kern.push((a, c, -400));
    let classes = PairSubtable::Classes {
        covered: vec![a, c],
        class1: vec![],
        class2: vec![(b, 1), (c, 2)],
        values: vec![vec![0, -10, -20]],
    };
    f.gpos_kern = vec![
        vec![PairSubtable::Pairs(vec![(a, vec![(b, -50)])]), classes],
        vec![PairSubtable::Pairs(vec![(a, vec![(b, -5)])])],
    ];
    let cv = convert(&ttf(&f), "pack:font/kerned", &Limits::default()).unwrap();
    assert_eq!(cv.info.kerning_source, "GPOS");
    let k = |v: f64| (v * 20.48).round() as i16;
    let l = cv.font.layout.unwrap();
    assert_eq!(
        l.kerning,
        [(0x41, 0x42, k(-55.0)), (0x41, 0x43, k(-20.0)), (0x43, 0x42, k(-10.0)), (0x43, 0x43, k(-20.0))]
    );
    let few = Limits { max_kerning_pairs: 3, ..Limits::default() };
    let cv = convert(&ttf(&f), "pack:font/kerned", &few).unwrap();
    assert!(cv.font.layout.unwrap().kerning.is_empty());
    assert_eq!(cv.info.kerning_dropped.as_deref(), Some("more than 3 kerning pairs between mapped characters"));
    let slow = Limits { max_kerning_work: 2, ..Limits::default() };
    let why = convert(&ttf(&f), "pack:font/kerned", &slow).unwrap().info.kerning_dropped.unwrap();
    assert!(why.contains("GPOS kerning: more than 2 steps"), "{why}");
}
