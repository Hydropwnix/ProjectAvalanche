use serde::Deserialize;

use super::{EVENTS, GamemodeToml, MAX_MAPS, MAX_TIME_LIMIT_S, MAX_TITLE_CHARS, MapEntry, Measure, TimeUp};
use crate::id::{ContentId, Kind};
use crate::manifest::line_col;
use crate::material::Problem;
use ssx_rules::gamemode::{MAX_TARGETS, check_targets};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawMapEntry {
    pub(crate) id: Option<toml::Spanned<String>>,
    pub(crate) targets: Option<toml::Spanned<Vec<toml::Spanned<f64>>>>,
    pub(crate) time_limit_s: Option<toml::Spanned<f64>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawGamemodeTable {
    pub(crate) title: Option<toml::Spanned<String>>,
    pub(crate) event: Option<toml::Spanned<String>>,
    pub(crate) time_up: Option<toml::Spanned<String>>,
    pub(crate) time_limit_s: Option<toml::Spanned<f64>>,
    pub(crate) any_map: Option<toml::Spanned<bool>>,
    pub(crate) map: Option<toml::Spanned<Vec<toml::Spanned<RawMapEntry>>>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawGamemode {
    pub(crate) gamemode: Option<toml::Spanned<RawGamemodeTable>>,
}

const MAX_KEY_CHARS: usize = 64;

fn named(field: Option<String>, m: &str) -> String {
    match field {
        Some(f) => format!("{f}: {}", ssx_core::text::message(m)),
        None => ssx_core::text::message(m).into_string(),
    }
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
    let map = table.is_some_and(|t| t.trim() == "[[gamemode.map]]");
    Some(if map { format!("map.{key}") } else { key.to_string() })
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

fn limit_ms(s: f64, what: &str) -> Result<u32, String> {
    if !s.is_finite() || s <= 0.0 || s > MAX_TIME_LIMIT_S {
        return Err(format!("{what}: above 0, at most {MAX_TIME_LIMIT_S} (the event clock's ceiling)"));
    }
    Ok(((s * 1000.0).round() as u32).max(1))
}

fn target_value(m: Measure, v: f64, what: &str) -> Result<u32, String> {
    let (scale, unit) = match m {
        Measure::Time => (1000.0, "seconds"),
        Measure::Points => (1.0, "points"),
        Measure::Descent => (1000.0, "metres"),
    };
    let max = f64::from(m.max()) / scale;
    if !v.is_finite() || v <= 0.0 || v > max {
        return Err(format!("{what}: above 0, at most {max} {unit}"));
    }
    if m == Measure::Points && v.fract() != 0.0 {
        return Err(format!("{what}: whole points"));
    }
    let x = (v * scale).round();
    if x < 1.0 || x > f64::from(m.max()) {
        return Err(format!("{what}: above 0, at most {max} {unit}"));
    }
    Ok(x as u32)
}

fn map_id(s: &str, what: &str) -> Result<ContentId, String> {
    let id: ContentId = s.parse().map_err(|e| format!("{what}: {e}"))?;
    if id.kind() != Kind::Map {
        return Err(format!("{what}: a `map` id (`<namespace>:map/…`), not `<namespace>:{}/…`", id.kind()));
    }
    Ok(id)
}

pub fn parse_toml(bytes: &[u8]) -> Result<GamemodeToml, Vec<Problem>> {
    let text = std::str::from_utf8(bytes).map_err(|e| vec![(0, 0, format!("not UTF-8: {e}"))])?;
    crate::package::parse_record_at(bytes).map_err(|(line, column, m)| {
        let field = line_start(text, line).and_then(|at| field_at(text, at));
        vec![(line, column, named(field, &m))]
    })?;
    let raw: RawGamemode = ::toml::from_str(text).map_err(|e| {
        let (line, column) = e.span().map_or((0, 0), |s| line_col(text, s.start));
        vec![(line, column, named(e.span().and_then(|s| field_at(text, s.start)), e.message()))]
    })?;
    let mut problems = Vec::new();
    let mut at = |span: std::ops::Range<usize>, m: String| {
        let (line, column) = line_col(text, span.start);
        problems.push((line, column, m));
    };
    let Some(table) = raw.gamemode else {
        return Err(vec![(0, 0, "a gamemode.toml has a [gamemode] table".into())]);
    };
    let span = table.span();
    let g = table.into_inner();
    let title = match &g.title {
        None => {
            at(span.clone(), format!("no title (1 to {MAX_TITLE_CHARS} characters)"));
            None
        }
        Some(t) => {
            let s = t.get_ref();
            let n = s.chars().count();
            if n == 0 || n > MAX_TITLE_CHARS || s.chars().any(char::is_control) {
                at(t.span(), format!("title: 1 to {MAX_TITLE_CHARS} characters, no control characters"));
                None
            } else {
                Some(s.clone())
            }
        }
    };
    let names: Vec<&str> = EVENTS.iter().map(|e| e.0).collect();
    let event_type = match &g.event {
        None => {
            at(span.clone(), format!("no event (the event type: {})", names.join(", ")));
            None
        }
        Some(e) => {
            let found = EVENTS.iter().find(|x| x.0 == e.get_ref()).map(|x| x.1);
            if found.is_none() {
                at(e.span(), format!("event: one of {}", names.join(", ")));
            }
            found
        }
    };
    let time_up = match &g.time_up {
        None => None,
        Some(t) => match t.get_ref().as_str() {
            "fail" => Some(TimeUp::Fail),
            "result" => Some(TimeUp::Result),
            _ => {
                at(t.span(), "time_up: fail or result".into());
                None
            }
        },
    };
    if let (Some(TimeUp::Result), Some(t), Some(raw)) = (time_up, event_type, &g.time_up)
        && t.is_race()
    {
        at(raw.span(), "time_up: a race has no result at its time limit (its value is the finish clock): fail".into());
    }
    let time_limit_ms = match &g.time_limit_s {
        None => None,
        Some(s) => match limit_ms(*s.get_ref(), "time_limit_s") {
            Ok(ms) => Some(ms),
            Err(m) => {
                at(s.span(), m);
                None
            }
        },
    };
    let any_map = g.any_map.as_ref().is_some_and(|b| *b.get_ref());
    let mut maps: Vec<MapEntry> = Vec::new();
    match &g.map {
        Some(list) if list.get_ref().len() > MAX_MAPS => {
            at(list.span(), format!("map: {} entries, at most {MAX_MAPS}", list.get_ref().len()));
        }
        Some(list) => {
            for e in list.get_ref() {
                let espan = e.span();
                let e = e.get_ref();
                let id = match &e.id {
                    None => {
                        at(espan.clone(), "map: no id (the map taken, `<namespace>:map/…`)".into());
                        None
                    }
                    Some(s) => match map_id(s.get_ref(), "map.id") {
                        Ok(id) if maps.iter().any(|m| m.map == id) => {
                            at(s.span(), format!("map.id: {id} twice"));
                            None
                        }
                        Ok(id) => Some(id),
                        Err(m) => {
                            at(s.span(), m);
                            None
                        }
                    },
                };
                let mut targets = Vec::new();
                if let (Some(list), Some(t)) = (&e.targets, event_type) {
                    let m = Measure::of(t);
                    if list.get_ref().len() > MAX_TARGETS {
                        at(list.span(), format!("map.targets: none or {MAX_TARGETS} (gold, silver, bronze)"));
                    } else {
                        for (k, v) in list.get_ref().iter().enumerate() {
                            match target_value(m, *v.get_ref(), &format!("map.targets[{k}]")) {
                                Ok(x) => targets.push(x),
                                Err(msg) => at(v.span(), msg),
                            }
                        }
                        if targets.len() == list.get_ref().len()
                            && let Err(err) = check_targets(t, &targets)
                        {
                            at(list.span(), format!("map.targets: {err}"));
                        }
                    }
                }
                let time_limit_ms = match &e.time_limit_s {
                    None => None,
                    Some(s) => match limit_ms(*s.get_ref(), "map.time_limit_s") {
                        Ok(ms) => Some(ms),
                        Err(m) => {
                            at(s.span(), m);
                            None
                        }
                    },
                };
                if let Some(map) = id {
                    maps.push(MapEntry { map, targets, time_limit_ms });
                }
            }
        }
        None => {}
    }
    if g.map.as_ref().is_none_or(|l| l.get_ref().is_empty()) && !any_map {
        at(span, "no [[gamemode.map]] and no any_map = true: the gamemode takes no map".into());
    }
    if !problems.is_empty() {
        problems.sort();
        return Err(problems);
    }
    let (Some(title), Some(event_type)) = (title, event_type) else {
        return Err(vec![(0, 0, "an incomplete [gamemode] table".into())]);
    };
    Ok(GamemodeToml { title, event_type, time_up, time_limit_ms, any_map, maps })
}
