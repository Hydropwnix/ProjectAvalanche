use std::sync::Arc;

use ssx_formats::gfx::shape::ShapeRecord;
use ssx_formats::gfx::text::{EditText, Font};

use crate::geom::RectF;
use crate::library::{Library, MovieId};

pub const GUTTER: f32 = 40.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
    Center,
    Justify,
}

impl Align {
    pub fn from_swf(a: u8) -> Align {
        match a {
            1 => Align::Right,
            2 => Align::Center,
            3 => Align::Justify,
            _ => Align::Left,
        }
    }

    fn from_html(s: &str) -> Option<Align> {
        Some(match s.to_ascii_lowercase().as_str() {
            "left" => Align::Left,
            "right" => Align::Right,
            "center" => Align::Center,
            "justify" => Align::Justify,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CharFormat {
    pub face: Option<String>,
    pub size: f32,
    pub color: [f32; 4],
    pub letter_spacing: f32,
    pub kerning: bool,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParaFormat {
    pub align: Align,
    pub left_margin: f32,
    pub right_margin: f32,
    pub indent: f32,
    pub leading: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub text: String,
    pub format: CharFormat,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Paragraph {
    pub format: ParaFormat,
    pub runs: Vec<Run>,
}

pub fn default_formats(e: &EditText) -> (CharFormat, ParaFormat) {
    let [r, g, b, a] = e.color.map_or([0.0, 0.0, 0.0, 1.0], |c| [c.r, c.g, c.b, c.a].map(|v| f32::from(v) / 255.0));
    let ch = CharFormat {
        face: None,
        size: f32::from(e.font_height.unwrap_or(240)),
        color: [r, g, b, a],
        letter_spacing: 0.0,
        kerning: false,
        bold: false,
        italic: false,
        underline: false,
    };
    let para = ParaFormat {
        align: Align::from_swf(e.layout.map_or(0, |l| l.align)),
        left_margin: e.layout.map_or(0.0, |l| f32::from(l.left_margin)),
        right_margin: e.layout.map_or(0.0, |l| f32::from(l.right_margin)),
        indent: e.layout.map_or(0.0, |l| f32::from(l.indent)),
        leading: e.layout.map_or(0.0, |l| f32::from(l.leading)),
    };
    (ch, para)
}

pub fn plain(text: &str, ch: &CharFormat, para: &ParaFormat) -> Vec<Paragraph> {
    text.replace("\r\n", "\n")
        .split(['\r', '\n'])
        .map(|line| Paragraph { format: *para, runs: vec![Run { text: line.to_string(), format: ch.clone() }] })
        .collect()
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(name) {
        let at = from + i;
        let before_ok = at == 0 || lower.as_bytes()[at - 1].is_ascii_whitespace();
        let rest = lower[at + name.len()..].trim_start();
        if before_ok && rest.starts_with('=') {
            let vstart = tag.len() - rest.len() + 1;
            let v = tag[vstart..].trim_start();
            let q = v.chars().next()?;
            return if q == '"' || q == '\'' {
                v[1..].split(q).next().map(str::to_string)
            } else {
                v.split_whitespace().next().map(str::to_string)
            };
        }
        from = at + name.len();
    }
    None
}

pub fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';').filter(|&e| e <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..end];
        let ch = match ent {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            _ => ent
                .strip_prefix("#x")
                .or_else(|| ent.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn parse_hex(v: &str) -> Option<u32> {
    let h = v.strip_prefix('#')?;
    if h.is_empty() {
        return None;
    }
    h.chars().try_fold(0u32, |acc, c| Some(acc.wrapping_shl(4) | c.to_digit(16)?))
}

fn parse_color(v: &str, current: [f32; 4]) -> Option<[f32; 4]> {
    let c = parse_hex(v)?;
    let [r, g, b] = [(c >> 16) & 255, (c >> 8) & 255, c & 255].map(|v| v as f32 / 255.0);
    Some([r, g, b, current[3]])
}

pub fn parse_html(src: &str, ch: &CharFormat, para: &ParaFormat) -> Vec<Paragraph> {
    let mut paras: Vec<Paragraph> = Vec::new();
    let mut open = false;
    let mut fmt: Vec<CharFormat> = vec![ch.clone()];
    let mut pfmt = *para;
    let mut rest = src;
    let start = |paras: &mut Vec<Paragraph>, f: ParaFormat| paras.push(Paragraph { format: f, runs: Vec::new() });
    while !rest.is_empty() {
        if let Some(stripped) = rest.strip_prefix('<') {
            let end = stripped.find('>').unwrap_or(stripped.len());
            let tag = &stripped[..end];
            rest = stripped.get(end + 1..).unwrap_or("");
            let name = tag.split_whitespace().next().unwrap_or("").trim_end_matches('/').to_ascii_lowercase();
            let top = fmt.last().cloned().unwrap_or_else(|| ch.clone());
            match name.as_str() {
                "p" => {
                    pfmt.align = attr(tag, "align").and_then(|a| Align::from_html(&a)).unwrap_or(para.align);
                    start(&mut paras, pfmt);
                    open = true;
                }
                "/p" => open = false,
                "br" => {
                    if !open {
                        start(&mut paras, pfmt);
                    }
                    start(&mut paras, pfmt);
                    open = true;
                }
                "font" => {
                    let mut f = top;
                    if let Some(face) = attr(tag, "face") {
                        f.face = Some(face);
                    }
                    if let Some(size) = attr(tag, "size").and_then(|s| s.trim().parse::<f32>().ok()) {
                        f.size = size * 20.0;
                    }
                    if let Some(c) = attr(tag, "color").and_then(|c| parse_color(&c, f.color)) {
                        f.color = c;
                    }
                    if let Some(a) = attr(tag, "alpha").and_then(|a| parse_hex(&a)) {
                        f.color[3] = (a & 255) as f32 / 255.0;
                    }
                    if let Some(ls) = attr(tag, "letterspacing").and_then(|s| s.trim().parse::<f32>().ok()) {
                        f.letter_spacing = ls * 20.0;
                    }
                    if let Some(k) = attr(tag, "kerning") {
                        f.kerning = k.trim() != "0";
                    }
                    fmt.push(f);
                }
                "b" | "i" | "u" => {
                    let mut f = top;
                    match name.as_str() {
                        "b" => f.bold = true,
                        "i" => f.italic = true,
                        _ => f.underline = true,
                    }
                    fmt.push(f);
                }
                "/font" | "/b" | "/i" | "/u" => {
                    if fmt.len() > 1 {
                        fmt.pop();
                    }
                }
                "textformat" => {
                    let px = |n: &str| attr(tag, n).and_then(|s| s.trim().parse::<f32>().ok()).map(|v| v * 20.0);
                    pfmt.leading = px("leading").unwrap_or(pfmt.leading);
                    pfmt.left_margin = px("leftmargin").unwrap_or(pfmt.left_margin);
                    pfmt.right_margin = px("rightmargin").unwrap_or(pfmt.right_margin);
                    pfmt.indent = px("indent").unwrap_or(pfmt.indent);
                }
                "/textformat" => pfmt = ParaFormat { align: pfmt.align, ..*para },
                _ => {}
            }
            continue;
        }
        let end = rest.find('<').unwrap_or(rest.len());
        let text = decode_entities(&rest[..end]);
        rest = &rest[end..];
        if !open {
            start(&mut paras, pfmt);
            open = true;
        }
        for (i, piece) in text.replace("\r\n", "\n").split(['\r', '\n']).enumerate() {
            if i > 0 {
                start(&mut paras, pfmt);
            }
            if !piece.is_empty() {
                let f = fmt.last().cloned().unwrap_or_else(|| ch.clone());
                paras.last_mut().expect("a paragraph").runs.push(Run { text: piece.to_string(), format: f });
            }
        }
    }
    if paras.is_empty() {
        paras.push(Paragraph { format: *para, runs: Vec::new() });
    }
    paras
}

#[derive(Clone, Debug)]
pub struct PlacedGlyph {
    pub font: Arc<Font>,
    pub index: usize,
    pub x: f32,
    pub y: f32,
    pub scale: f32,
    pub color: [f32; 4],
    pub line: u32,
    pub ink_right: f32,
}

#[derive(Clone, Debug, Default)]
pub struct TextLayout {
    pub glyphs: Vec<PlacedGlyph>,
    pub text_width: f32,
    pub text_height: f32,
    pub lines: usize,
    pub line_boxes: Vec<(f32, f32)>,
}

pub fn glyph_ink_right(font: &Font, index: usize) -> f32 {
    let Some(shape) = font.glyphs.get(index) else { return 0.0 };
    let (mut x, mut right) = (0.0f32, f32::NEG_INFINITY);
    for r in &shape.records {
        match *r {
            ShapeRecord::StyleChange { move_to: Some((mx, _)), .. } => x = mx as f32,
            ShapeRecord::StyleChange { .. } => {}
            ShapeRecord::Straight { dx, .. } => {
                right = right.max(x).max(x + dx as f32);
                x += dx as f32;
            }
            ShapeRecord::Curved { control_dx, anchor_dx, .. } => {
                let (p0, p1, p2) = (x, x + control_dx as f32, x + (control_dx + anchor_dx) as f32);
                right = right.max(p0).max(p2);
                let d = p0 - 2.0 * p1 + p2;
                if d != 0.0 {
                    let t = (p0 - p1) / d;
                    if (0.0..=1.0).contains(&t) {
                        right = right.max((1.0 - t) * (1.0 - t) * p0 + 2.0 * t * (1.0 - t) * p1 + t * t * p2);
                    }
                }
                x = p2;
            }
        }
    }
    if right.is_finite() { right } else { 0.0 }
}

pub fn em_units(font: &Font) -> f32 {
    if font.version == 3 { 20480.0 } else { 1024.0 }
}

pub fn glyph_index(font: &Font, ch: char) -> Option<usize> {
    let code = u32::from(ch);
    if code > 0xFFFF {
        return None;
    }
    let code = code as u16;
    font.codes.binary_search(&code).ok().or_else(|| font.codes.iter().position(|&c| c == code))
}

struct Item {
    font: Arc<Font>,
    glyph: Option<usize>,
    advance: f32,
    scale: f32,
    color: [f32; 4],
    ascent: f32,
    descent: f32,
    space: bool,
}

fn metrics(font: &Font, size: f32) -> (f32, f32) {
    let k = size / em_units(font);
    // TODO(guess): a font without a layout record gets 0.8 / 0.2 EM (every font the disc's text uses has one); settle: Scaleform's metrics fallback.
    font.layout.as_ref().map_or((size * 0.8, size * 0.2), |l| (f32::from(l.ascent) * k, f32::from(l.descent) * k))
}

// TODO(guess): Flash Player's layout rules (line heights, word wrap at spaces, trailing spaces, kerning with `kerning="1"`; no faux bold / italic / underline); settle: Scaleform's text engine (document view / line buffer).
pub fn layout(
    lib: &Library,
    movie: MovieId,
    font_id: Option<u16>,
    paras: &[Paragraph],
    default: &CharFormat,
    bounds: RectF,
    word_wrap: bool,
) -> TextLayout {
    let mut out = TextLayout::default();
    let origin = bounds.y0 + GUTTER;
    let mut top = origin;
    let field_w = bounds.width() - 2.0 * GUTTER;
    let default_metrics = lib
        .font_for(movie, font_id, default.face.as_deref())
        .map_or((default.size * 0.8, default.size * 0.2), |f| metrics(&f, default.size));
    let mut last_leading = 0.0;
    for para in paras {
        let mut items: Vec<Item> = Vec::new();
        let mut para_metrics = None;
        for run in &para.runs {
            let f = &run.format;
            let Some(font) = lib.font_for(movie, font_id, f.face.as_deref()) else { continue };
            let scale = f.size / em_units(&font);
            let (ascent, descent) = metrics(&font, f.size);
            para_metrics.get_or_insert((ascent, descent));
            let mut prev: Option<u16> = None;
            for ch in run.text.chars() {
                let glyph =
                    glyph_index(&font, ch).or_else(|| if ch == '\u{a0}' { glyph_index(&font, ' ') } else { None });
                let adv = glyph
                    .and_then(|g| font.layout.as_ref().and_then(|l| l.advances.get(g)))
                    .map_or(0.0, |&a| f32::from(a) * scale);
                let code = u32::from(ch).min(0xFFFF) as u16;
                if f.kerning
                    && let (Some(p), Some(l), Some(last)) = (prev, font.layout.as_ref(), items.last_mut())
                    && let Some(k) = l.kerning.iter().find(|k| k.0 == p && k.1 == code)
                {
                    last.advance += f32::from(k.2) * scale;
                }
                prev = Some(code);
                let space = ch == ' ' || ch == '\u{a0}' || ch == '\t';
                items.push(Item {
                    font: font.clone(),
                    glyph,
                    advance: adv + f.letter_spacing,
                    scale,
                    color: f.color,
                    ascent,
                    descent,
                    space,
                });
            }
        }
        let pf = &para.format;
        let mut start = 0;
        let mut first = true;
        loop {
            let indent = if first { pf.indent } else { 0.0 };
            let avail = field_w - pf.left_margin - pf.right_margin - indent;
            let (mut x, mut end, mut brk) = (0.0f32, start, None);
            while end < items.len() {
                let it = &items[end];
                if word_wrap && end > start && !it.space && x + it.advance > avail {
                    if let Some(b) = brk {
                        end = b;
                    }
                    break;
                }
                x += it.advance;
                end += 1;
                if it.space {
                    brk = Some(end);
                }
            }
            let line = &items[start..end];
            let trimmed = line.iter().rposition(|i| !i.space).map_or(0, |p| p + 1);
            let width: f32 = line[..trimmed].iter().map(|i| i.advance).sum();
            let (ascent, descent) = if line.is_empty() {
                para_metrics.unwrap_or(default_metrics)
            } else {
                line.iter().fold((0.0f32, 0.0f32), |m, i| (m.0.max(i.ascent), m.1.max(i.descent)))
            };
            let left = bounds.x0 + GUTTER + pf.left_margin + indent;
            let right = bounds.x1 - GUTTER - pf.right_margin;
            let x0 = match pf.align {
                Align::Right => right - width,
                Align::Center => left + (right - left - width) / 2.0,
                Align::Left | Align::Justify => left,
            };
            let baseline = top + ascent;
            let mut pen = x0;
            for it in line {
                if let Some(g) = it.glyph.filter(|_| !it.space) {
                    out.glyphs.push(PlacedGlyph {
                        font: it.font.clone(),
                        index: g,
                        x: pen,
                        y: baseline,
                        scale: it.scale,
                        color: it.color,
                        line: out.lines as u32,
                        ink_right: glyph_ink_right(&it.font, g) * it.scale,
                    });
                }
                pen += it.advance;
            }
            out.text_width = out.text_width.max(width);
            out.line_boxes.push((top - origin, ascent + descent));
            out.lines += 1;
            top = baseline + descent + pf.leading;
            last_leading = pf.leading;
            first = false;
            start = end;
            if start >= items.len() {
                break;
            }
        }
    }
    out.text_height = (top - last_leading - origin).max(0.0);
    out
}

#[cfg(test)]
mod tests {
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

    fn test_font() -> Font {
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
}
