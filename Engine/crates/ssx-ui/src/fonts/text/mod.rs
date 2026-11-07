use std::sync::Arc;

use ssx_formats::gfx::shape::ShapeRecord;
use ssx_formats::gfx::text::{EditText, Font};

use crate::geom::RectF;
use crate::library::{Library, MovieId};

mod complex;
mod html;
pub mod isolate;

pub use complex::MAX_PARAGRAPH_CHARS;
pub use complex::itemize::MAX_CLUSTER_CHARS;
pub use complex::shape::MAX_SHAPED_CHARS;
pub use html::{decode_entities, parse_html};

pub fn script_name(c: char) -> &'static str {
    complex::itemize::script_code(complex::itemize::script_of(c))
}

pub fn needs_complex_layout(text: &str) -> bool {
    let chars: Vec<char> = text.chars().filter(|&c| !isolate::is_mark(c)).collect();
    complex::needs_complex(&chars)
}

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
    pub advance: f32,
    pub char_index: u32,
}

#[derive(Clone, Debug, Default)]
pub struct TextLayout {
    pub glyphs: Vec<PlacedGlyph>,
    pub text_width: f32,
    pub text_height: f32,
    pub lines: usize,
    pub line_boxes: Vec<(f32, f32)>,
    pub pending: bool,
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

pub(crate) const MISSING_GLYPH_EM: f32 = 0.5;

pub fn em_units(font: &Font) -> f32 {
    if font.version == 3 { 20480.0 } else { 1024.0 }
}

fn kerning_pair(table: &[(u16, u16, i16)], a: u16, b: u16) -> Option<&(u16, u16, i16)> {
    let at = table.partition_point(|k| (k.0, k.1) < (a, b));
    table.get(at).filter(|k| (k.0, k.1) == (a, b))
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
    char_index: u32,
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
    let mut char_base = 0u32;
    let field = complex::Field { lib, movie, font_id, bounds, word_wrap, origin, field_w, default_metrics };
    for para in paras {
        let runs: Vec<(Option<Arc<Font>>, &str)> = para
            .runs
            .iter()
            .map(|r| (lib.font_for(movie, font_id, r.format.face.as_deref()), r.text.as_str()))
            .collect();
        let outside = isolate::outside(runs.iter().flat_map(|(_, t)| t.chars()));
        if !complex::game_layout_draws(lib, &runs, &outside) {
            complex::paragraph(&field, para, char_base, &mut top, &mut last_leading, &mut out);
            for r in &para.runs {
                char_base = char_base.saturating_add(r.text.chars().count().min(u32::MAX as usize) as u32);
            }
            char_base = char_base.saturating_add(1);
            continue;
        }
        let mut items: Vec<Item> = Vec::new();
        let mut para_metrics = None;
        let mut outside = outside.into_iter();
        for run in &para.runs {
            let run_base = char_base;
            char_base = char_base.saturating_add(run.text.chars().count().min(u32::MAX as usize) as u32);
            let run_outside: Vec<bool> = outside.by_ref().take(run.text.chars().count()).collect();
            let f = &run.format;
            let Some(font) = lib.font_for(movie, font_id, f.face.as_deref()) else { continue };
            let scale = f.size / em_units(&font);
            let (ascent, descent) = metrics(&font, f.size);
            para_metrics.get_or_insert((ascent, descent));
            let mut prev: Option<u16> = None;
            for (i, ch) in run.text.chars().enumerate() {
                let outside = run_outside.get(i).copied().unwrap_or(false);
                if isolate::is_mark(ch) {
                    continue;
                }
                let glyph =
                    glyph_index(&font, ch).or_else(|| if ch == '\u{a0}' { glyph_index(&font, ' ') } else { None });
                let (item_font, glyph, item_scale) = match glyph {
                    Some(g) => (font.clone(), Some(g), scale),
                    None => match outside
                        .then(|| lib.disc_glyph(ch))
                        .flatten()
                        .or_else(|| lib.fallback_fonts().iter().find_map(|fb| glyph_index(fb, ch).map(|g| (fb, g))))
                    {
                        Some((fb, g)) => (fb.clone(), Some(g), f.size / em_units(fb)),
                        None => (font.clone(), None, scale),
                    },
                };
                // TODO(guess): a character no font has is not drawn — GetGlyphShape (the font's vtable 0x820c9320
                // +0x38, 0x826b5fe0) and GetTextureGlyph (+0x28, 0x826b71c8) return null past their tables, index
                // 0xFFFF included, so the font gives it no outline; the line buffer's draw loop, which reads the glyph
                // entries the formatter writes (index 0xFFFF, no invisible flag), was not traced; settle: that loop.
                let adv = match glyph {
                    Some(g) => item_font
                        .layout
                        .as_ref()
                        .and_then(|l| l.advances.get(g))
                        .map_or(0.0, |&a| f32::from(a) * item_scale),
                    None if ch == '\t' => 0.0,
                    None => f.size * MISSING_GLYPH_EM,
                };
                let code = u32::from(ch).min(0xFFFF) as u16;
                if f.kerning
                    && let (Some(p), Some(l), Some(last)) = (prev, font.layout.as_ref(), items.last_mut())
                    && let Some(k) = kerning_pair(&l.kerning, p, code)
                {
                    last.advance += f32::from(k.2) * scale;
                }
                prev = Some(code);
                let space = ch == ' ' || ch == '\u{a0}' || ch == '\t';
                items.push(Item {
                    font: item_font,
                    glyph,
                    advance: adv + f.letter_spacing,
                    scale: item_scale,
                    color: f.color,
                    ascent,
                    descent,
                    space,
                    char_index: run_base.saturating_add(i.min(u32::MAX as usize) as u32),
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
                        advance: it.advance,
                        char_index: it.char_index,
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
        char_base = char_base.saturating_add(1);
    }
    out.text_height = (top - last_leading - origin).max(0.0);
    out
}

#[cfg(test)]
mod tests;
