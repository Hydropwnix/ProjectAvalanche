use std::collections::BTreeMap;

use serde::Deserialize;

use super::music::{At, LoopRegion};

pub const MAX_SECTIONS: usize = 64;
pub const MAX_LAYERS: usize = 8;
pub const MAX_STINGERS: usize = 64;
pub const MAX_EVENTS: usize = 256;

fn name_ok(s: &str) -> bool {
    (1..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum When {
    Now,
    Bar,
    End,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub name: String,
    pub file: String,
    pub looping: Option<LoopRegion>,
    pub repeat: bool,
    pub layers: BTreeMap<String, String>,
    pub next: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stinger {
    pub name: String,
    pub file: String,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Action {
    pub section: Option<(String, When)>,
    pub hold: bool,
    pub stinger: Option<String>,
    pub layers: BTreeMap<String, f32>,
    pub fade: f32,
    pub volume: Option<f32>,
    pub pitch: Option<f32>,
    pub stop: Option<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Interactive {
    pub gain: f32,
    pub bpm: Option<f64>,
    pub beats_per_bar: u32,
    pub sections: Vec<Section>,
    pub stingers: Vec<Stinger>,
    pub layers: BTreeMap<String, f32>,
    pub events: BTreeMap<String, Action>,
    pub control: BTreeMap<u8, Action>,
    pub start: Option<String>,
}

pub const HELD: &str = "@held";

impl Interactive {
    pub fn files(&self) -> Vec<&str> {
        let mut v: Vec<&str> = Vec::new();
        for s in &self.sections {
            v.push(&s.file);
            v.extend(s.layers.values().map(String::as_str));
        }
        v.extend(self.stingers.iter().map(|s| s.file.as_str()));
        v
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawScore {
    #[serde(rename = "type")]
    _kind: String,
    gain: Option<toml::Spanned<f64>>,
    bpm: Option<toml::Spanned<f64>>,
    beats_per_bar: Option<toml::Spanned<i64>>,
    start: Option<toml::Spanned<String>>,
    #[serde(default)]
    layer: Vec<RawLayer>,
    section: toml::Spanned<Vec<RawSection>>,
    #[serde(default)]
    stinger: Vec<RawStinger>,
    #[serde(default)]
    events: BTreeMap<String, toml::Spanned<RawAction>>,
    #[serde(default)]
    control: BTreeMap<String, toml::Spanned<RawAction>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLayer {
    name: toml::Spanned<String>,
    level: Option<toml::Spanned<f64>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSection {
    name: toml::Spanned<String>,
    file: toml::Spanned<String>,
    #[serde(rename = "loop")]
    looping: Option<toml::Spanned<Vec<f64>>>,
    repeat: Option<bool>,
    next: Option<toml::Spanned<String>>,
    #[serde(default)]
    layers: BTreeMap<String, toml::Spanned<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStinger {
    name: toml::Spanned<String>,
    file: toml::Spanned<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawAction {
    section: Option<toml::Spanned<String>>,
    when: Option<toml::Spanned<String>>,
    hold: Option<bool>,
    stinger: Option<toml::Spanned<String>>,
    #[serde(default)]
    layers: BTreeMap<String, toml::Spanned<f64>>,
    fade: Option<toml::Spanned<f64>>,
    volume: Option<toml::Spanned<f64>>,
    pitch: Option<toml::Spanned<f64>>,
    stop: Option<toml::Spanned<f64>>,
}

fn event_key_ok(k: &str) -> bool {
    if let Some(h) = k.strip_prefix("0x") {
        return h.len() == 8 && h.bytes().all(|b| b.is_ascii_hexdigit());
    }
    (1..=64).contains(&k.len()) && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

pub(super) fn parse(at: &mut At<'_>, text: &str) -> Result<Interactive, toml::de::Error> {
    let raw: RawScore = toml::from_str(text)?;
    let gain = at.gain(&raw.gain);
    let bpm = raw.bpm.as_ref().and_then(|b| {
        let ok = b.get_ref().is_finite() && (20.0..=400.0).contains(b.get_ref());
        if !ok {
            at.at(b.span(), "bpm: from 20 to 400".into());
        }
        ok.then_some(*b.get_ref())
    });
    let beats_per_bar = match &raw.beats_per_bar {
        Some(b) if (1..=16).contains(b.get_ref()) => *b.get_ref() as u32,
        Some(b) => {
            at.at(b.span(), "beats_per_bar: from 1 to 16".into());
            4
        }
        None => 4,
    };
    let mut layers = BTreeMap::new();
    if raw.layer.len() > MAX_LAYERS {
        at.at(raw.section.span(), format!("at most {MAX_LAYERS} layers"));
    }
    for l in &raw.layer {
        let n = l.name.get_ref();
        if !name_ok(n) || layers.contains_key(n) {
            at.at(l.name.span(), format!("layer {}: a new name of [a-z0-9_]", ssx_core::text::quoted(n)));
        }
        let level = level(at, &l.level, 1.0, "level");
        layers.insert(n.clone(), level);
    }
    let list = raw.section.get_ref();
    if list.is_empty() || list.len() > MAX_SECTIONS {
        at.at(raw.section.span(), format!("section: 1 to {MAX_SECTIONS} sections"));
    }
    let mut sections: Vec<Section> = Vec::new();
    for s in list {
        let name = s.name.get_ref().clone();
        if !name_ok(&name) || sections.iter().any(|x| x.name == name) {
            at.at(s.name.span(), format!("section {}: a new name of [a-z0-9_]", ssx_core::text::quoted(&name)));
        }
        let file = at.audio_file(&s.file);
        let mut stems = BTreeMap::new();
        for (layer, f) in &s.layers {
            if !layers.contains_key(layer) {
                at.at(f.span(), format!("layer {} is not declared by a [[layer]]", ssx_core::text::quoted(layer)));
            }
            stems.insert(layer.clone(), at.audio_file(f));
        }
        let looping = s.looping.as_ref().and_then(|r| at.region(r));
        let repeat = s.repeat.unwrap_or(true);
        sections.push(Section {
            name,
            file,
            looping,
            repeat,
            layers: stems,
            next: s.next.as_ref().map(|n| n.get_ref().clone()),
        });
    }
    for (s, raw) in sections.iter().zip(list) {
        if let Some(n) = &raw.next
            && !sections.iter().any(|x| &x.name == n.get_ref())
        {
            at.at(n.span(), format!("next: no section {}", ssx_core::text::quoted(n.get_ref())));
        }
        if s.next.is_some() && s.repeat {
            at.at(
                raw.name.span(),
                format!("section {}: `next` is for a section with `repeat = false`", ssx_core::text::quoted(&s.name)),
            );
        }
    }
    let mut stingers: Vec<Stinger> = Vec::new();
    if raw.stinger.len() > MAX_STINGERS {
        at.at(raw.section.span(), format!("at most {MAX_STINGERS} stingers"));
    }
    for s in &raw.stinger {
        let name = s.name.get_ref().clone();
        if !name_ok(&name) || stingers.iter().any(|x| x.name == name) {
            at.at(s.name.span(), format!("stinger {}: a new name of [a-z0-9_]", ssx_core::text::quoted(&name)));
        }
        stingers.push(Stinger { name, file: at.audio_file(&s.file) });
    }
    let start = raw.start.as_ref().map(|s| {
        if !sections.iter().any(|x| &x.name == s.get_ref()) {
            at.at(s.span(), format!("start: no section {}", ssx_core::text::quoted(s.get_ref())));
        }
        s.get_ref().clone()
    });
    let score = Interactive {
        gain,
        bpm,
        beats_per_bar,
        sections,
        stingers,
        layers,
        events: BTreeMap::new(),
        control: BTreeMap::new(),
        start,
    };
    let mut events = BTreeMap::new();
    if raw.events.len() > MAX_EVENTS {
        at.at(raw.section.span(), format!("at most {MAX_EVENTS} events"));
    }
    for (k, a) in &raw.events {
        if !event_key_ok(k) {
            at.at(
                a.span(),
                format!("event {}: a name of the project's events or 0x + 8 hex digits", ssx_core::text::quoted(k)),
            );
        }
        events.insert(k.clone(), action(at, &score, a));
    }
    let mut control = BTreeMap::new();
    for (k, a) in &raw.control {
        match k.parse::<u8>() {
            Ok(v) if v <= 127 => {
                control.insert(v, action(at, &score, a));
            }
            _ => at.at(a.span(), format!("control {}: a control level from 0 to 127", ssx_core::text::quoted(k))),
        }
    }
    Ok(Interactive { events, control, ..score })
}

fn level(at: &mut At<'_>, v: &Option<toml::Spanned<f64>>, default: f32, what: &str) -> f32 {
    match v {
        Some(x) if x.get_ref().is_finite() && (0.0..=1.0).contains(x.get_ref()) => *x.get_ref() as f32,
        Some(x) => {
            at.at(x.span(), format!("{what}: from 0 to 1"));
            default
        }
        None => default,
    }
}

fn action(at: &mut At<'_>, score: &Interactive, a: &toml::Spanned<RawAction>) -> Action {
    let r = a.get_ref();
    let mut out = Action { hold: r.hold.unwrap_or(false), ..Default::default() };
    let when = match r.when.as_ref().map(|w| (w.get_ref().as_str(), w.span())) {
        None | Some(("now", _)) => When::Now,
        Some(("bar", s)) => {
            if score.bpm.is_none() {
                at.at(s, "when = \"bar\" needs the score's bpm".into());
            }
            When::Bar
        }
        Some(("end", _)) => When::End,
        Some((other, s)) => {
            at.at(s, format!("when {}: now, bar or end", ssx_core::text::quoted(other)));
            When::Now
        }
    };
    if let Some(s) = &r.section {
        let n = s.get_ref();
        if n != HELD && !score.sections.iter().any(|x| &x.name == n) {
            at.at(s.span(), format!("section: no section {} (or \"{HELD}\")", ssx_core::text::quoted(n)));
        }
        out.section = Some((n.clone(), when));
    }
    if let Some(s) = &r.stinger
        && !score.stingers.iter().any(|x| &x.name == s.get_ref())
    {
        at.at(s.span(), format!("stinger: no stinger {}", ssx_core::text::quoted(s.get_ref())));
    }
    out.stinger = r.stinger.as_ref().map(|s| s.get_ref().clone());
    for (k, v) in &r.layers {
        if !score.layers.contains_key(k) {
            at.at(v.span(), format!("layers: no layer {}", ssx_core::text::quoted(k)));
        }
        out.layers.insert(k.clone(), level(at, &Some(v.clone()), 0.0, "a layer's level"));
    }
    let seconds = |v: &toml::Spanned<f64>, what: &str, at: &mut At<'_>| {
        let ok = v.get_ref().is_finite() && (0.0..=60.0).contains(v.get_ref());
        if !ok {
            at.at(v.span(), format!("{what}: seconds from 0 to 60"));
        }
        if ok { *v.get_ref() as f32 } else { 0.0 }
    };
    out.fade = r.fade.as_ref().map_or(0.0, |f| seconds(f, "fade", at));
    out.stop = r.stop.as_ref().map(|f| seconds(f, "stop", at));
    out.volume = r.volume.as_ref().map(|v| level(at, &Some(v.clone()), 1.0, "volume"));
    out.pitch = r.pitch.as_ref().map(|p| {
        let ok = p.get_ref().is_finite() && (0.0..=4.0).contains(p.get_ref());
        if !ok {
            at.at(p.span(), "pitch: from 0 to 4".into());
        }
        if ok { *p.get_ref() as f32 } else { 1.0 }
    });
    out
}
