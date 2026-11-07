use serde::Deserialize;

use super::{BASES, DISC_INPUTS, Hand, INPUTS, Input, MAX_POINTS, MIN_POINTS, ROLES, TrickToml};
use crate::id::{ContentId, Kind};
use crate::manifest::line_col;
use crate::material::Problem;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawClips {
    pub(crate) enter: Option<toml::Spanned<String>>,
    #[serde(rename = "loop")]
    pub(crate) loop_: Option<toml::Spanned<String>>,
    pub(crate) tweak: Option<toml::Spanned<String>>,
    pub(crate) tweak_loop: Option<toml::Spanned<String>>,
    pub(crate) uber: Option<toml::Spanned<String>>,
    pub(crate) uber_loop: Option<toml::Spanned<String>>,
    pub(crate) uber_tweak: Option<toml::Spanned<String>>,
    pub(crate) uber_tweak_loop: Option<toml::Spanned<String>>,
    pub(crate) super_uber: Option<toml::Spanned<String>>,
    pub(crate) super_uber_loop: Option<toml::Spanned<String>>,
    pub(crate) super_uber_tweak: Option<toml::Spanned<String>>,
    pub(crate) super_uber_tweak_loop: Option<toml::Spanned<String>>,
}

impl RawClips {
    fn all(&self) -> [&Option<toml::Spanned<String>>; 12] {
        [
            &self.enter,
            &self.loop_,
            &self.tweak,
            &self.tweak_loop,
            &self.uber,
            &self.uber_loop,
            &self.uber_tweak,
            &self.uber_tweak_loop,
            &self.super_uber,
            &self.super_uber_loop,
            &self.super_uber_tweak,
            &self.super_uber_tweak_loop,
        ]
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawTrickTable {
    pub(crate) base: Option<toml::Spanned<String>>,
    pub(crate) input: Option<toml::Spanned<String>>,
    pub(crate) hand: Option<toml::Spanned<String>>,
    pub(crate) points: Option<toml::Spanned<i64>>,
    pub(crate) name: Option<toml::Spanned<String>>,
    pub(crate) name_uber: Option<toml::Spanned<String>>,
    pub(crate) name_super_uber: Option<toml::Spanned<String>>,
    pub(crate) clips: Option<toml::Spanned<RawClips>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawTrick {
    pub(crate) trick: Option<toml::Spanned<RawTrickTable>>,
}

const MAX_KEY_CHARS: usize = 64;

fn named(field: Option<String>, m: &str) -> String {
    match field {
        Some(f) => format!("{f}: {}", ssx_core::text::message(m)),
        None => ssx_core::text::message(m).into_string(),
    }
}

fn line_start(text: &str, line: usize) -> Option<usize> {
    if line == 0 {
        return None;
    }
    let mut at = 0;
    for _ in 1..line {
        at += text.get(at..)?.find('\n')? + 1;
    }
    Some(at)
}

fn field_at(text: &str, at: usize) -> Option<String> {
    let before = text.get(..at)?;
    let start = before.rfind('\n').map_or(0, |i| i + 1);
    let line = text.get(start..)?.split('\n').next()?;
    let key = line.split_once('=')?.0.trim();
    let bare = |k: &str| k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if key.is_empty() || key.len() > MAX_KEY_CHARS || !bare(key) {
        return None;
    }
    let table = before.get(..start)?.lines().rev().find(|l| l.trim_start().starts_with('['));
    let clips = table.is_some_and(|t| t.trim() == "[trick.clips]");
    Some(if clips { format!("clips.{key}") } else { key.to_string() })
}

fn id_of(s: &str, kind: Kind, what: &str) -> Result<ContentId, String> {
    let id: ContentId = s.parse().map_err(|e| format!("{what}: {e}"))?;
    if id.kind() != kind {
        return Err(format!("{what}: a `{kind}` id (`<namespace>:{kind}/…`), not `<namespace>:{}/…`", id.kind()));
    }
    Ok(id)
}

pub fn parse_toml(bytes: &[u8]) -> Result<TrickToml, Vec<Problem>> {
    let text = std::str::from_utf8(bytes).map_err(|e| vec![(0, 0, format!("not UTF-8: {e}"))])?;
    crate::package::parse_record_at(bytes).map_err(|(line, column, m)| {
        let field = line_start(text, line).and_then(|at| field_at(text, at));
        vec![(line, column, named(field, &m))]
    })?;
    let raw: RawTrick = ::toml::from_str(text).map_err(|e| {
        let (line, column) = e.span().map_or((0, 0), |s| line_col(text, s.start));
        vec![(line, column, named(e.span().and_then(|s| field_at(text, s.start)), e.message()))]
    })?;
    let mut problems = Vec::new();
    let mut at = |span: std::ops::Range<usize>, m: String| {
        let (line, column) = line_col(text, span.start);
        problems.push((line, column, m));
    };
    let Some(table) = raw.trick else {
        return Err(vec![(0, 0, "a trick.toml has a [trick] table".into())]);
    };
    let span = table.span();
    let t = table.into_inner();
    let names: Vec<&str> = BASES.iter().map(|b| b.0).collect();
    let base = match &t.base {
        None => {
            at(span.clone(), format!("no base (the disc grab the trick plays in: {})", names.join(", ")));
            None
        }
        Some(b) => {
            let found = BASES.iter().position(|x| x.0 == b.get_ref());
            if found.is_none() {
                at(b.span(), format!("base: one of {}", names.join(", ")));
            }
            found
        }
    };
    let free: Vec<&str> = INPUTS.iter().map(|i| i.name()).collect();
    let input = match &t.input {
        None => {
            at(span.clone(), format!("no input (one the disc leaves free: {})", free.join(", ")));
            None
        }
        Some(i) => match Input::parse(i.get_ref()) {
            Some(x) => Some(x),
            None => {
                let m = match DISC_INPUTS.iter().find(|d| d.0 == i.get_ref()) {
                    Some((n, regular, goofy)) => format!(
                        "input: {n} is the disc's {regular} (goofy {goofy}); an added trick takes one of {}",
                        free.join(", ")
                    ),
                    None => format!("input: one of {}", free.join(", ")),
                };
                at(i.span(), m);
                None
            }
        },
    };
    let hand = match &t.hand {
        None => None,
        Some(h) => {
            let x = Hand::parse(h.get_ref());
            if x.is_none() {
                at(h.span(), "hand: rear, front or both".into());
            }
            x
        }
    };
    let points = match &t.points {
        None => {
            at(span.clone(), format!("no points (TRICK_POINTS: {MIN_POINTS} to {MAX_POINTS})"));
            None
        }
        Some(p) => match u16::try_from(*p.get_ref()) {
            Ok(v) if (MIN_POINTS..=MAX_POINTS).contains(&v) => Some(v),
            _ => {
                let m = format!(
                    "points: {MIN_POINTS} to {MAX_POINTS} (the disc table's largest, the signature's), not {}",
                    p.get_ref()
                );
                at(p.span(), m);
                None
            }
        },
    };
    let mut names = Vec::new();
    for (field, what) in [(&t.name, "name"), (&t.name_uber, "name_uber"), (&t.name_super_uber, "name_super_uber")] {
        match field {
            None => at(span.clone(), format!("no {what} (a string id `<namespace>:string/<key>`)")),
            Some(s) => match id_of(s.get_ref(), Kind::String, what) {
                Ok(id) => names.push(id),
                Err(m) => at(s.span(), m),
            },
        }
    }
    let mut clips = Vec::new();
    match &t.clips {
        None => at(span, "no [trick.clips] table (a clip id for each of the base's 12 clips)".into()),
        Some(c) => {
            let cspan = c.span();
            for (field, (role, _)) in c.get_ref().all().into_iter().zip(ROLES) {
                match field {
                    None => at(cspan.clone(), format!("clips: no {role} (a clip id `<namespace>:anim/…`)")),
                    Some(s) => match id_of(s.get_ref(), Kind::Anim, &format!("clips.{role}")) {
                        Ok(id) => clips.push(id),
                        Err(m) => at(s.span(), m),
                    },
                }
            }
        }
    }
    if !problems.is_empty() {
        problems.sort();
        return Err(problems);
    }
    let (Some(base), Some(input), Some(points)) = (base, input, points) else {
        return Err(vec![(0, 0, "an incomplete [trick] table".into())]);
    };
    let (Ok(names), Ok(clips)) = (<[ContentId; 3]>::try_from(names), <[ContentId; 12]>::try_from(clips)) else {
        return Err(vec![(0, 0, "an incomplete [trick] table".into())]);
    };
    let hand = hand.unwrap_or_else(|| Hand::of_base(BASES[base].1));
    Ok(TrickToml { base, input, hand, points, names, clips })
}
