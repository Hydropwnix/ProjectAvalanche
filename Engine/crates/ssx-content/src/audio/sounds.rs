use serde::Deserialize;

use crate::id::{BASE_NAMESPACE, ContentId};
use crate::manifest::line_col;
use crate::material::Problem;

pub const MAX_FILES: usize = 256;
pub const MAX_PLAYERS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoundClass {
    Effect,
    Speech,
    Trigger,
    MusicEvent,
    Added,
}

impl SoundClass {
    pub fn of(id: &ContentId) -> Self {
        if id.namespace() != BASE_NAMESPACE {
            return SoundClass::Added;
        }
        let path = id.path();
        if path.starts_with("speech/") {
            if path.contains("_contexteventdef_") { SoundClass::Trigger } else { SoundClass::Speech }
        } else if path.starts_with("music/") {
            SoundClass::MusicEvent
        } else {
            SoundClass::Effect
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variation {
    Random,
    Sequential,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Looping {
    Disc,
    On,
    Off,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    Game,
    Bus(String),
    Emitter(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerRef {
    pub bank: String,
    pub module: Option<u32>,
    pub player: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Others {
    Keep,
    Mute,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SoundDef {
    pub files: Vec<String>,
    pub variation: Variation,
    pub volume: f32,
    pub pitch: (f32, f32),
    pub looping: Looping,
    pub route: Route,
    pub players: Option<Vec<PlayerRef>>,
    pub voices: Option<Vec<String>>,
    pub others: Others,
}

impl SoundDef {
    pub fn file(rel: &str) -> Self {
        Self {
            files: vec![rel.to_string()],
            variation: Variation::Sequential,
            volume: 1.0,
            pitch: (1.0, 1.0),
            looping: Looping::Disc,
            route: Route::Game,
            players: None,
            voices: None,
            others: Others::Keep,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSound {
    files: toml::Spanned<Vec<toml::Spanned<String>>>,
    variation: Option<toml::Spanned<String>>,
    volume: Option<toml::Spanned<f64>>,
    pitch: Option<toml::Spanned<Vec<f64>>>,
    #[serde(rename = "loop")]
    looping: Option<toml::Spanned<toml::Value>>,
    bus: Option<toml::Spanned<String>>,
    attenuation: Option<toml::Spanned<String>>,
    players: Option<toml::Spanned<Vec<RawPlayer>>>,
    voices: Option<toml::Spanned<Vec<toml::Spanned<String>>>>,
    others: Option<toml::Spanned<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPlayer {
    bank: toml::Spanned<String>,
    module: Option<toml::Spanned<i64>>,
    player: toml::Spanned<i64>,
}

fn game_name(s: &str) -> bool {
    (1..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

pub fn parse_toml(bytes: &[u8], class: SoundClass) -> Result<SoundDef, Vec<Problem>> {
    crate::package::parse_record_at(bytes).map_err(|p| vec![p])?;
    let text = std::str::from_utf8(bytes).map_err(|e| vec![(0, 0, format!("not UTF-8: {e}"))])?;
    let raw: RawSound = toml::from_str(text).map_err(|e| {
        let (line, column) = e.span().map_or((0, 0), |s| line_col(text, s.start));
        vec![(line, column, ssx_core::text::message(e.message()).into_string())]
    })?;
    let mut problems = Vec::new();
    let mut at = |span: std::ops::Range<usize>, m: String| {
        let (line, column) = line_col(text, span.start);
        problems.push((line, column, m));
    };
    let mut def = SoundDef::file("");
    def.files.clear();
    let list = raw.files.get_ref();
    if list.is_empty() || list.len() > MAX_FILES {
        at(raw.files.span(), format!("files: 1 to {MAX_FILES} audio files"));
    }
    for f in list {
        let p = f.get_ref();
        if let Err(e) = crate::path::check_package_path(p, false) {
            at(f.span(), e.to_string());
        } else if super::Format::from_name(p).is_err() {
            at(f.span(), format!("{}: not .ogg, .flac or .wav", ssx_core::text::quoted(p)));
        } else {
            def.files.push(p.clone());
        }
    }
    if let Some(v) = &raw.variation {
        match v.get_ref().as_str() {
            "random" => def.variation = Variation::Random,
            "sequential" => def.variation = Variation::Sequential,
            other => at(v.span(), format!("variation {}: random or sequential", ssx_core::text::quoted(other))),
        }
    }
    if let Some(v) = &raw.volume {
        let x = *v.get_ref();
        if x.is_finite() && (0.0..=4.0).contains(&x) {
            def.volume = x as f32;
        } else {
            at(v.span(), "volume: from 0 to 4 (a factor on the game's volume)".into());
        }
    }
    if let Some(p) = &raw.pitch {
        match p.get_ref().as_slice() {
            [lo, hi] if lo.is_finite() && hi.is_finite() && 0.25 <= *lo && lo <= hi && *hi <= 4.0 => {
                def.pitch = (*lo as f32, *hi as f32);
            }
            _ => at(p.span(), "pitch: [low, high] factors from 0.25 to 4, low not above high".into()),
        }
    }
    let effect_only = |what: &str, span: std::ops::Range<usize>, at: &mut dyn FnMut(std::ops::Range<usize>, String)| {
        if class != SoundClass::Effect {
            let why = match class {
                SoundClass::Speech => "a speech line plays on its event's voice (its bus and 3-D path are the voice's)",
                SoundClass::Added => {
                    "a package's own sound has no game voices (scripts, markers and voice lines play it)"
                }
                _ => "not a sound of the game's effects",
            };
            at(span, format!("{what}: only for the game's effects; {why}"));
        }
    };
    if let Some(l) = &raw.looping {
        effect_only("loop", l.span(), &mut at);
        match l.get_ref() {
            toml::Value::Boolean(true) => def.looping = Looping::On,
            toml::Value::Boolean(false) => def.looping = Looping::Off,
            toml::Value::String(s) if s == "disc" => def.looping = Looping::Disc,
            _ => at(l.span(), "loop: true, false or \"disc\" (as the game's sample it replaces)".into()),
        }
    }
    match (&raw.bus, &raw.attenuation) {
        (Some(b), Some(a)) => {
            at(a.span(), "attenuation sends to its emitter's 3-D bus: give a bus or an attenuation, not both".into());
            let _ = b;
        }
        (Some(b), None) => {
            effect_only("bus", b.span(), &mut at);
            if game_name(b.get_ref()) {
                def.route = Route::Bus(b.get_ref().clone());
            } else {
                at(b.span(), format!("bus {}: a bus name of the game's mixer", ssx_core::text::quoted(b.get_ref())));
            }
        }
        (None, Some(a)) => {
            effect_only("attenuation", a.span(), &mut at);
            if game_name(a.get_ref()) {
                def.route = Route::Emitter(a.get_ref().clone());
            } else {
                at(
                    a.span(),
                    format!(
                        "attenuation {}: an emitter of the game's (`audio_3d_settings`)",
                        ssx_core::text::quoted(a.get_ref())
                    ),
                );
            }
        }
        (None, None) => {}
    }
    if let Some(ps) = &raw.players {
        effect_only("players", ps.span(), &mut at);
        if ps.get_ref().is_empty() || ps.get_ref().len() > MAX_PLAYERS {
            at(ps.span(), format!("players: 1 to {MAX_PLAYERS} players"));
        }
        let mut list = Vec::new();
        for p in ps.get_ref() {
            let bank = p.bank.get_ref();
            if !game_name(bank) {
                at(p.bank.span(), format!("bank {}: an .abk bank's name", ssx_core::text::quoted(bank)));
            }
            let index = |v: &toml::Spanned<i64>, what: &str, at: &mut dyn FnMut(std::ops::Range<usize>, String)| {
                u32::try_from(*v.get_ref()).ok().filter(|&x| x < 256).or_else(|| {
                    at(v.span(), format!("{what}: an index from 0 to 255"));
                    None
                })
            };
            let module = p.module.as_ref().map(|m| index(m, "module", &mut at));
            let player = index(&p.player, "player", &mut at);
            if let (Some(player), true) = (player, module.is_none_or(|m| m.is_some())) {
                list.push(PlayerRef { bank: bank.clone(), module: module.flatten(), player });
            }
        }
        def.players = Some(list);
    }
    if let Some(vs) = &raw.voices {
        effect_only("voices", vs.span(), &mut at);
        if raw.players.is_some() {
            at(vs.span(), "voices: give the voices by their players or by their mixer voices, not both".into());
        }
        if vs.get_ref().is_empty() || vs.get_ref().len() > MAX_PLAYERS {
            at(vs.span(), format!("voices: 1 to {MAX_PLAYERS} mixer voice names"));
        }
        let mut names = Vec::new();
        for v in vs.get_ref() {
            if game_name(v.get_ref()) {
                names.push(v.get_ref().clone());
            } else {
                at(
                    v.span(),
                    format!(
                        "voice {}: a mixer voice's name (`audio_mixer_voices`)",
                        ssx_core::text::quoted(v.get_ref())
                    ),
                );
            }
        }
        def.voices = Some(names);
    }
    if let Some(o) = &raw.others {
        effect_only("others", o.span(), &mut at);
        match o.get_ref().as_str() {
            "keep" => def.others = Others::Keep,
            "mute" => def.others = Others::Mute,
            other => at(o.span(), format!("others {}: keep or mute", ssx_core::text::quoted(other))),
        }
        if raw.players.is_none() && raw.voices.is_none() {
            at(o.span(), "others: names what the voices outside `players` or `voices` do; there is neither".into());
        }
    }
    if problems.is_empty() { Ok(def) } else { Err(problems) }
}
