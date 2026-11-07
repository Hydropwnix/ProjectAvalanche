use std::collections::HashMap;
use std::sync::Arc;

use icu_properties::CodePointMapData;
use icu_properties::props::LineBreak;
use icu_segmenter::LineSegmenter;
use icu_segmenter::options::LineBreakOptions;

use super::itemize::Cluster;
use super::shape::{self, NO_PART, Src, Text, Unit};
use crate::bidi::{self, Class};
use crate::ot::{Converted, OtFont};

pub fn breaks(chars: &[char], classes: &[Class], units: &[Unit]) -> Vec<(bool, bool)> {
    let s: String = chars.iter().collect();
    let mut at_char = vec![false; chars.len() + 1];
    let mut byte_to_char = vec![0usize; s.len() + 1];
    let mut end = 0;
    for (ci, (b, c)) in s.char_indices().enumerate() {
        byte_to_char[b] = ci;
        end = b + c.len_utf8();
    }
    byte_to_char[end] = chars.len();
    for b in LineSegmenter::new_dictionary(LineBreakOptions::default()).segment_str(&s) {
        at_char[byte_to_char[b]] = true;
    }
    let lb = CodePointMapData::<LineBreak>::new();
    units
        .iter()
        .map(|u| {
            let forced = chars[u.start..u.end].iter().zip(&classes[u.start..u.end]).any(|(&c, &k)| {
                k == Class::B
                    || matches!(
                        lb.get(c),
                        LineBreak::MandatoryBreak
                            | LineBreak::CarriageReturn
                            | LineBreak::LineFeed
                            | LineBreak::NextLine
                    )
            });
            (at_char[u.end], forced)
        })
        .enumerate()
        .map(|(i, (uax14, forced))| {
            let game = !units[i].shaped && units.get(i + 1).is_none_or(|n| !n.shaped);
            (if game { units[i].space } else { uax14 }, forced)
        })
        .collect()
}

pub fn fill(
    t: &Text<'_>,
    clusters: &[Cluster],
    classes: &[Class],
    units: &mut Vec<Unit>,
    brk: &mut Vec<(bool, bool)>,
    word_wrap: bool,
    mut avail: impl FnMut(bool) -> f32,
) -> Vec<(usize, usize)> {
    let mut next_part = units.iter().filter(|u| u.part != NO_PART).map(|u| u.part + 1).max().unwrap_or(0);
    let mut lines = Vec::new();
    let mut start = 0;
    let mut first = true;
    loop {
        let width = avail(first);
        let mut end = line_end(units, brk, start, word_wrap, width);
        if word_wrap && cut_inside_a_call(units, brk, start, end) {
            let part = units[end].part;
            let lo = units[start..end].iter().find(|u| u.part == part).map_or(units[end].start, |u| u.start);
            let hi = units[end..].iter().take_while(|u| u.part == part).last().map_or(units[end].end, |u| u.end);
            let mut cut = units[end].start;
            loop {
                let fresh = shape::reshape(t, clusters, lo, cut, hi, &mut next_part);
                let a = units.iter().position(|u| u.start == lo).unwrap_or(start);
                let b = units.iter().rposition(|u| u.end == hi).map_or(a, |i| i + 1);
                units.splice(a..b, fresh);
                *brk = breaks(t.chars, classes, units);
                end = units.iter().position(|u| u.start == cut).unwrap_or(units.len());
                let head: f32 = units[start..end].iter().map(|u| u.advance).sum();
                if head <= width || end <= start + 1 || cut == lo {
                    break;
                }
                cut = units[end - 1].start;
            }
        }
        lines.push((start, end));
        first = false;
        start = end;
        if start >= units.len() {
            break;
        }
    }
    lines
}

fn line_end(units: &[Unit], brk: &[(bool, bool)], start: usize, word_wrap: bool, width: f32) -> usize {
    let (mut x, mut end, mut last) = (0.0f32, start, None);
    while end < units.len() {
        let u = &units[end];
        if word_wrap && end > start && !u.space && x + u.advance > width {
            if let Some(b) = last {
                end = b;
            }
            break;
        }
        x += u.advance;
        end += 1;
        if brk[end - 1].1 {
            break;
        }
        if brk[end - 1].0 {
            last = Some(end);
        }
    }
    end
}

fn cut_inside_a_call(units: &[Unit], brk: &[(bool, bool)], start: usize, end: usize) -> bool {
    end > start
        && end < units.len()
        && units[end - 1].shaped
        && units[end].shaped
        && units[end - 1].part == units[end].part
        && !brk[end - 1].0
        && !brk[end - 1].1
}

pub fn visual(
    units: &[Unit],
    line: (usize, usize),
    classes: &[Class],
    levels: &[u8],
    base: u8,
    bidi_on: bool,
) -> (Vec<usize>, f32) {
    let (s, e) = line;
    let kept = units[s..e].iter().rposition(|u| !u.space).map_or(s, |p| s + p + 1);
    let width: f32 = units[s..kept].iter().map(|u| u.advance).sum();
    if !bidi_on || kept == s {
        return ((s..kept).collect(), width);
    }
    let (c0, c1) = (units[s].start, units[e - 1].end);
    let line_levels = bidi::line_levels(classes, levels, base, c0..c1);
    let unit_levels: Vec<u8> = units[s..kept].iter().map(|u| line_levels[u.start - c0]).collect();
    (bidi::visual_order(&unit_levels).into_iter().map(|i| s + i).collect(), width)
}

type FontBatch = (Arc<OtFont>, Vec<(usize, usize, u32)>);

pub fn convert(
    units: &[Unit],
    order: &[usize],
    budget: &crate::ot::Budget,
) -> HashMap<(usize, usize), Option<Converted>> {
    let mut by_font: Vec<FontBatch> = Vec::new();
    for &u in order {
        for (gi, g) in units[u].glyphs.iter().enumerate() {
            if let Src::Ot(f, gid) = &g.src {
                match by_font.iter_mut().find(|(ff, _)| Arc::ptr_eq(ff, f)) {
                    Some((_, v)) => v.push((u, gi, *gid)),
                    None => by_font.push((f.clone(), vec![(u, gi, *gid)])),
                }
            }
        }
    }
    let mut out = HashMap::new();
    for (f, want) in by_font {
        let gids: Vec<u32> = want.iter().map(|w| w.2).collect();
        for ((u, gi, _), got) in want.into_iter().zip(f.glyphs(&gids, budget)) {
            out.insert((u, gi), got);
        }
    }
    out
}
