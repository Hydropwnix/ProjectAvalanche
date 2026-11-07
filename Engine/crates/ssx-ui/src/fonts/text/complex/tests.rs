use std::str::FromStr;
use std::sync::Arc;

use ssx_formats::gfx::text::Font;

use super::itemize::{self, Choice};
use super::shape::{self, Src, Text};
use super::{CharFormat, lines};
use crate::bidi::class;
use crate::display::tests::empty_lib;
use crate::text::isolate;

fn format() -> CharFormat {
    CharFormat {
        face: None,
        size: 400.0,
        color: [1.0; 4],
        letter_spacing: 0.0,
        kerning: true,
        bold: false,
        italic: false,
        underline: false,
    }
}

#[test]
fn a_word_forced_across_two_lines_is_shaped_per_piece_with_its_joining_forms() {
    let lib = empty_lib();
    let word = "\u{633}\u{633}\u{633}\u{633}\u{628}\u{64a}\u{627}";
    let text: Vec<char> = isolate::wrap(word).chars().collect();
    let classes: Vec<_> = text.iter().map(|&c| class::of(c)).collect();
    let (levels, _) = itemize::levels(&text, &classes);
    let run_of = vec![0; text.len()];
    let f = format();
    let formats = [&f];
    let run_fonts: [Option<Arc<Font>>; 1] = [Some(Arc::new(super::super::tests::test_font()))];
    let outside = isolate::outside(text.iter().copied());
    let clusters = itemize::itemize(&lib, &text, &levels, &run_of, &run_fonts, &outside);
    let Some(Choice::Ot(font)) = clusters.iter().map(|c| c.font.clone()).find(|c| matches!(c, Choice::Ot(_))) else {
        panic!("the word has no OpenType font");
    };
    let t = Text {
        chars: &text,
        levels: &levels,
        run_of: &run_of,
        formats: &formats,
        run_fonts: &run_fonts,
        language: None,
    };
    let mut units = shape::units(&t, &clusters);
    let whole: f32 = units.iter().map(|u| u.advance).sum();
    let gid_at = |units: &[shape::Unit]| -> Vec<(usize, Vec<u32>)> {
        units
            .iter()
            .filter(|u| !u.glyphs.is_empty())
            .map(|u| {
                let gids = u.glyphs.iter().filter_map(|g| if let Src::Ot(_, gid) = g.src { Some(gid) } else { None });
                (u.start, gids.collect())
            })
            .collect()
    };
    let whole_forms = gid_at(&units);
    assert!(whole_forms.len() >= 6, "{whole_forms:?}");
    let field: f32 = units.iter().filter(|u| (1..5).contains(&u.start)).map(|u| u.advance).sum::<f32>() * 1.02;
    assert!(field < whole);
    let mut brk = lines::breaks(&text, &classes, &units);
    let got = lines::fill(&t, &clusters, &classes, &mut units, &mut brk, true, |_| field);
    let drawn: Vec<_> = got.iter().filter(|(s, e)| units[*s..*e].iter().any(|u| !u.glyphs.is_empty())).collect();
    assert_eq!(drawn.len(), 2, "{got:?}");
    let (a, b) = (drawn[0], drawn[1]);
    let parts = |(s, e): (usize, usize)| -> Vec<u32> {
        let mut p: Vec<u32> = units[s..e].iter().filter(|u| u.shaped && !u.glyphs.is_empty()).map(|u| u.part).collect();
        p.dedup();
        p
    };
    let (pa, pb) = (parts(*a), parts(*b));
    assert_eq!((pa.len(), pb.len()), (1, 1), "{pa:?} {pb:?}");
    assert_ne!(pa, pb);
    let width = |(s, e): (usize, usize)| -> f32 { units[s..e].iter().map(|u| u.advance).sum() };
    assert!(width(*a) <= field, "{} of {field}", width(*a));
    let form = |c: char, before: bool, after: bool| -> Vec<u32> {
        let mut s: Vec<char> = Vec::new();
        if before {
            s.push('\u{640}');
        }
        s.push(c);
        if after {
            s.push('\u{640}');
        }
        let run = crate::ot::ShapeRun {
            text: &s,
            before: &[],
            after: &[],
            rtl: true,
            script: harfrust::Script::from_str("Arab").expect("Arab"),
            language: None,
            kerning: true,
        };
        let shaped = font.shape(&run).expect("shaped");
        shaped.iter().filter(|g| g.cluster == u32::from(before)).map(|g| g.gid).collect()
    };
    let glyph_of = |at: usize| gid_at(&units).into_iter().find(|(s, _)| *s == at).map(|(_, g)| g).expect("a glyph");
    let (end_a, start_b) = (units[a.1 - 1].start, units[b.0..].iter().find(|u| u.shaped).expect("a unit").start);
    let (ca, cb) = (text[end_a], text[start_b]);
    for not in [form(ca, true, false), form(ca, false, false)] {
        assert_ne!(glyph_of(end_a), not, "the first line's last letter {ca:?} does not join on");
    }
    for not in [form(cb, false, true), form(cb, false, false)] {
        assert_ne!(glyph_of(start_b), not, "the second line's first letter {cb:?} does not join back");
    }
    for (at, gid) in &whole_forms {
        if (*at + 1 < end_a) || (*at > start_b) {
            assert_eq!(&glyph_of(*at), gid, "the letter at {at}");
        }
    }
}

#[test]
fn an_outside_han_run_takes_one_font() {
    let lib = empty_lib();
    let mut font = super::super::tests::test_font();
    font.codes.extend([u16::from(b'B'), 0x5927, 0x5BB6]);
    font.glyphs.extend(std::iter::repeat_n(font.glyphs[1].clone(), 3));
    if let Some(l) = font.layout.as_mut() {
        l.advances.extend([600, 1024, 1024]);
        let b = l.bounds[1];
        l.bounds.extend([b, b, b]);
    }
    let run_fonts: [Option<Arc<Font>>; 1] = [Some(Arc::new(font))];
    let cjk = crate::fallback::for_cluster("Hani", &[]).expect("the shipped CJK font");
    let pick = |s: &str| {
        let text: Vec<char> = s.chars().collect();
        let classes: Vec<_> = text.iter().map(|&c| class::of(c)).collect();
        let (levels, _) = itemize::levels(&text, &classes);
        let run_of = vec![0; text.len()];
        let outside = isolate::outside(text.iter().copied());
        let clusters = itemize::itemize(&lib, &text, &levels, &run_of, &run_fonts, &outside);
        clusters
            .iter()
            .map(|c| {
                let f = match &c.font {
                    Choice::None => "-".to_string(),
                    Choice::Df3 { .. } => "disc".to_string(),
                    Choice::Ot(f) if Arc::ptr_eq(f, &cjk) => "cjk".to_string(),
                    Choice::Ot(_) => "ot".to_string(),
                };
                (text[c.start], f)
            })
            .collect::<Vec<_>>()
    };
    let line = "\u{5927}\u{5BB6}\u{597D}\u{FF0C}AB";
    let got = pick(&isolate::wrap(line));
    let fonts: Vec<&str> = got.iter().map(|(_, f)| f.as_str()).collect();
    assert_eq!(fonts, ["-", "cjk", "cjk", "cjk", "cjk", "disc", "disc", "-"], "{got:?}");
    let got = pick(line);
    let fonts: Vec<&str> = got.iter().map(|(_, f)| f.as_str()).collect();
    assert_eq!(fonts, ["disc", "disc", "-", "-", "disc", "disc"], "{got:?}");
}
