use std::collections::BTreeMap;

use ssx_formats::t3db::format::{Placeholder, placeholders};

use crate::id::{ContentId, Kind};
use crate::limits::MAX_RECORD_BYTES;
use crate::material::Problem;

pub const DIR: &str = "strings";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Lang {
    En,
    Fr,
    De,
    Es,
    It,
    Ja,
}

impl Lang {
    pub const ALL: [Lang; 6] = [Lang::En, Lang::Fr, Lang::De, Lang::Es, Lang::It, Lang::Ja];

    pub fn code(self) -> &'static str {
        ["en", "fr", "de", "es", "it", "ja"][self as usize]
    }

    pub fn game_file(self) -> &'static str {
        ["eng_us", "fre_fr", "ger_de", "spa_es", "ita_it", "jpn_jp"][self as usize]
    }

    pub fn from_code(code: &str) -> Option<Lang> {
        Lang::ALL.into_iter().find(|l| l.code() == code)
    }

    pub fn from_game_file(file: &str) -> Option<Lang> {
        Lang::ALL.into_iter().find(|l| l.game_file() == file)
    }
}

pub mod caps {
    pub const ENTRIES: usize = 16_384;
    pub const KEY_BYTES: usize = 128;
    pub const TEXT_BYTES: usize = 4096;
    pub const LINES: usize = 64;
    pub const TAGS: usize = 64;
    pub const DEPTH: usize = 16;
    pub const TAG_BYTES: usize = 256;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    Base(ContentId),
    Added(ContentId),
}

impl Key {
    pub fn id(&self) -> &ContentId {
        match self {
            Key::Base(id) | Key::Added(id) => id,
        }
    }
}

pub fn key(namespace: &str, key: &str) -> Result<Key, String> {
    if key.len() > caps::KEY_BYTES {
        return Err(format!("a key of {} bytes (at most {})", key.len(), caps::KEY_BYTES));
    }
    if key.get(..4).is_some_and(|p| p.eq_ignore_ascii_case("txt_")) {
        let lower = key.to_ascii_lowercase();
        let ns = if lower.starts_with("txt_ava_") { crate::PORT_NAMESPACE } else { crate::BASE_NAMESPACE };
        return ContentId::new(ns, Kind::String, &lower)
            .map(Key::Base)
            .map_err(|e| format!("{} is not a string key: {e}", ssx_core::text::quoted(key)));
    }
    ContentId::new(namespace, Kind::String, key).map(Key::Added).map_err(|e| {
        format!(
            "{}: a key the package adds is an id's path (lower case a-z 0-9 _ . -, `/` between segments): {e}",
            ssx_core::text::quoted(key)
        )
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub key: String,
    pub text: String,
    pub at: (usize, usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    pub lang: Lang,
    pub file: String,
    pub entries: BTreeMap<ContentId, Entry>,
}

pub fn table_lang(path: &str) -> Option<Result<Lang, String>> {
    let name = path.strip_prefix(DIR)?.strip_prefix('/')?;
    if name.contains('/') {
        return Some(Err(format!("{path}: string tables are `{DIR}/<language>.toml` (no folders below `{DIR}/`)")));
    }
    let stem = name.strip_suffix(".toml");
    Some(stem.and_then(Lang::from_code).ok_or_else(|| {
        let codes: Vec<&str> = Lang::ALL.iter().map(|l| l.code()).collect();
        format!("{path}: not a string table (`{DIR}/<language>.toml`, a language of {})", codes.join(", "))
    }))
}

pub fn parse_table(bytes: &[u8], file: &str, lang: Lang, namespace: &str) -> Result<Table, Vec<Problem>> {
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err(vec![(0, 0, format!("{} bytes (a table is at most {MAX_RECORD_BYTES})", bytes.len()))]);
    }
    crate::package::parse_record_at(bytes).map_err(|p| vec![p])?;
    let text = std::str::from_utf8(bytes).map_err(|e| vec![(0, 0, format!("not UTF-8: {e}"))])?;
    let raw: BTreeMap<toml::Spanned<String>, toml::Spanned<toml::Value>> = toml::from_str(text).map_err(|e| {
        let (l, c) = e.span().map_or((0, 0), |s| crate::manifest::line_col(text, s.start));
        vec![(l, c, ssx_core::text::message(e.message()).into_string())]
    })?;
    if raw.len() > caps::ENTRIES {
        return Err(vec![(0, 0, format!("{} strings (a table holds at most {})", raw.len(), caps::ENTRIES))]);
    }
    let mut problems = Vec::new();
    let mut entries = BTreeMap::new();
    let mut keys: Vec<(&toml::Spanned<String>, &toml::Spanned<toml::Value>)> = raw.iter().collect();
    keys.sort_by_key(|(k, _)| k.span().start);
    for (k, v) in keys {
        let at = crate::manifest::line_col(text, k.span().start);
        let name = k.get_ref().as_str();
        let id = match key(namespace, name) {
            Ok(k) => k.id().clone(),
            Err(e) => {
                problems.push((at.0, at.1, e));
                continue;
            }
        };
        let toml::Value::String(s) = v.get_ref() else {
            problems.push((at.0, at.1, format!("{}: a string's text is a TOML string", ssx_core::text::quoted(name))));
            continue;
        };
        if let Err(e) = check_text(s) {
            problems.push((at.0, at.1, format!("{}: {e}", ssx_core::text::quoted(name))));
            continue;
        }
        if let Some(first) = entries.get(&id).map(|e: &Entry| e.key.clone()) {
            problems.push((
                at.0,
                at.1,
                format!(
                    "{}: the key {} again (keys are case-insensitive)",
                    ssx_core::text::quoted(name),
                    ssx_core::text::quoted(&first)
                ),
            ));
            continue;
        }
        entries.insert(id, Entry { key: name.to_string(), text: s.to_string(), at });
    }
    if problems.is_empty() { Ok(Table { lang, file: file.to_string(), entries }) } else { Err(problems) }
}

pub fn check_text(text: &str) -> Result<(), String> {
    if text.len() > caps::TEXT_BYTES {
        return Err(format!("{} bytes of text (at most {})", text.len(), caps::TEXT_BYTES));
    }
    let lines = text.split(['\r', '\n']).count();
    if lines > caps::LINES {
        return Err(format!("{lines} lines (at most {})", caps::LINES));
    }
    if let Some(c) = text.chars().find(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')) {
        return Err(format!("the control character U+{:04X}", u32::from(c)));
    }
    check_markup(text)
}

const TAGS: &[(&str, &[&str])] = &[
    ("p", &["align"]),
    ("br", &[]),
    ("b", &[]),
    ("i", &[]),
    ("u", &[]),
    ("font", &["face", "size", "color", "alpha", "letterspacing", "kerning"]),
    ("textformat", &["leading", "leftmargin", "rightmargin", "indent"]),
];

pub fn check_markup(text: &str) -> Result<(), String> {
    let mut open: Vec<&str> = Vec::new();
    let (mut tags, mut rest) = (0usize, text);
    while let Some(i) = rest.find('<') {
        let after = &rest[i + 1..];
        if !after.starts_with(|c: char| c.is_ascii_alphabetic() || c == '/') {
            rest = after;
            continue;
        }
        let end = after.find('>').ok_or("a tag without `>`")?;
        let tag = &after[..end];
        rest = &after[end + 1..];
        if tag.len() > caps::TAG_BYTES {
            return Err(format!("a tag of {} bytes (at most {})", tag.len(), caps::TAG_BYTES));
        }
        tags += 1;
        if tags > caps::TAGS {
            return Err(format!("more than {} tags", caps::TAGS));
        }
        if let Some(name) = tag.strip_prefix('/') {
            let name = name.trim().to_ascii_lowercase();
            match open.pop() {
                Some(o) if o == name => {}
                Some(o) => return Err(format!("</{}> closes <{o}>", ssx_core::text::shown(&name))),
                None => return Err(format!("</{}> closes nothing", ssx_core::text::shown(&name))),
            }
            continue;
        }
        let self_closing = tag.ends_with('/');
        let tag = tag.trim_end_matches('/');
        let mut parts = tag.splitn(2, |c: char| c.is_ascii_whitespace());
        let name = parts.next().unwrap_or("").to_ascii_lowercase();
        let Some(&(known, attrs)) = TAGS.iter().find(|(n, _)| *n == name) else {
            return Err(format!(
                "<{}> is not a tag the player reads (p br b i u font textformat)",
                ssx_core::text::shown(&name)
            ));
        };
        check_attributes(known, attrs, parts.next().unwrap_or(""))?;
        if known != "br" && !self_closing {
            open.push(known);
            if open.len() > caps::DEPTH {
                return Err(format!("tags nested deeper than {}", caps::DEPTH));
            }
        }
    }
    match open.last() {
        Some(o) => Err(format!("<{o}> is not closed")),
        None => Ok(()),
    }
}

fn check_attributes(tag: &str, known: &[&str], mut s: &str) -> Result<(), String> {
    loop {
        s = s.trim_start();
        if s.is_empty() {
            return Ok(());
        }
        let eq = s.find('=').ok_or_else(|| format!("<{tag}>: an attribute without a value"))?;
        let name = s[..eq].trim().to_ascii_lowercase();
        let shown_name = ssx_core::text::shown(&name);
        let v = s[eq + 1..].trim_start();
        let q = v
            .chars()
            .next()
            .filter(|&c| c == '"' || c == '\'')
            .ok_or_else(|| format!("<{tag} {shown_name}>: quote the value"))?;
        let close = v[1..].find(q).ok_or_else(|| format!("<{tag} {shown_name}>: an unclosed value"))?;
        let value = &v[1..1 + close];
        let shown_value = ssx_core::text::shown(value);
        s = &v[close + 2..];
        if !known.contains(&name.as_str()) {
            return Err(format!("<{tag}> has no attribute {} the player reads", ssx_core::text::quoted(&name)));
        }
        let number = |lo: f64, hi: f64| -> Result<(), String> {
            match value.trim().parse::<f64>() {
                Ok(n) if (lo..=hi).contains(&n) => Ok(()),
                _ => Err(format!("<{tag} {shown_name}=\"{shown_value}\">: a number from {lo} to {hi}")),
            }
        };
        let hex = |digits: usize| -> Result<(), String> {
            match value.strip_prefix('#') {
                Some(h) if h.len() == digits && h.chars().all(|c| c.is_ascii_hexdigit()) => Ok(()),
                _ => Err(format!("<{tag} {shown_name}=\"{shown_value}\">: `#` and {digits} hex digits")),
            }
        };
        match name.as_str() {
            "align" if ["left", "right", "center", "justify"].contains(&value.to_ascii_lowercase().as_str()) => {}
            "align" => return Err(format!("<{tag} align=\"{shown_value}\">: left, right, center or justify")),
            "face" if !value.is_empty() && value.len() <= 64 && !value.chars().any(char::is_control) => {}
            "face" => return Err(format!("<{tag} face>: a font name of 1 to 64 bytes")),
            "size" => number(1.0, 255.0)?,
            "color" => hex(6)?,
            "alpha" => hex(2)?,
            "letterspacing" => number(-64.0, 64.0)?,
            "kerning" if value == "0" || value == "1" => {}
            "kerning" => return Err(format!("<{tag} kerning=\"{shown_value}\">: 0 or 1")),
            _ => number(-255.0, 1024.0)?,
        }
    }
}

pub fn arguments(text: &str) -> BTreeMap<u8, (char, Option<String>)> {
    let mut out = BTreeMap::new();
    for p in placeholders(text) {
        if let Placeholder::Arg { number, spec, conversion, .. } = p {
            out.entry(number).or_insert((conversion, spec));
        }
    }
    out
}

pub fn tokens(text: &str) -> Vec<String> {
    let parts: Vec<&str> = text.split('^').collect();
    let mut out: Vec<String> = parts
        .iter()
        .enumerate()
        .filter(|(i, p)| i % 2 == 1 && *i + 1 < parts.len() && !p.is_empty())
        .map(|(_, p)| p.to_string())
        .collect();
    out.sort();
    out
}

pub fn check_against(text: &str, base: Option<&str>) -> Result<(), String> {
    let mine = arguments(text);
    let Some(base) = base else {
        return match mine.keys().next() {
            Some(n) => Err(format!(
                "a placeholder (%{n}…) in a string the package adds (only the game's strings are formatted)"
            )),
            None => Ok(()),
        };
    };
    let theirs = arguments(base);
    if mine != theirs {
        let show = |m: &BTreeMap<u8, (char, Option<String>)>| {
            let v: Vec<String> = m
                .iter()
                .map(|(n, (c, s))| match s {
                    Some(s) => format!("%{n}[{}]{c}", ssx_core::text::shown_max(s, ssx_core::text::VALUE_CHARS)),
                    None => format!("%{n}{c}"),
                })
                .collect();
            if v.is_empty() { "none".to_string() } else { v.join(" ") }
        };
        return Err(format!("its placeholders {} are not the base string's {}", show(&mine), show(&theirs)));
    }
    let (a, b) = (tokens(text), tokens(base));
    if a != b {
        return Err(format!(
            "its button tokens {} are not the base string's {}",
            ssx_core::text::quoted_list(&a),
            ssx_core::text::quoted_list(&b)
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
