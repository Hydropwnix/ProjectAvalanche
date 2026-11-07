use serde::Deserialize;

use crate::id::{BASE_NAMESPACE, ContentId, Kind};
use crate::manifest::line_col;
use crate::material::Problem;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Mode {
    Race,
    Trick,
    Freeride,
    Survive,
    Death,
}

impl Mode {
    pub const ALL: [Mode; 5] = [Mode::Race, Mode::Trick, Mode::Freeride, Mode::Survive, Mode::Death];

    pub fn name(self) -> &'static str {
        match self {
            Mode::Race => "race",
            Mode::Trick => "trick",
            Mode::Freeride => "freeride",
            Mode::Survive => "survive",
            Mode::Death => "death",
        }
    }

    pub fn event_type(self) -> ssx_rules::events::EventType {
        use ssx_rules::events::EventType;
        match self {
            Mode::Race => EventType::RaceClock,
            Mode::Trick => EventType::TrickScore,
            Mode::Freeride => EventType::FreeRide,
            Mode::Survive => EventType::SurviveToBottom,
            Mode::Death => EventType::SurviveToDeath,
        }
    }

    pub fn needs_finish(self) -> bool {
        self != Mode::Freeride
    }

    pub fn parse(s: &str) -> Option<Mode> {
        Mode::ALL.into_iter().find(|m| m.name() == s)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Intro {
    #[default]
    Spawn,
    Heli,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MapToml {
    pub title: String,
    pub scene: String,
    pub base: Option<ContentId>,
    pub modes: Vec<Mode>,
    pub lighting: Option<ContentId>,
    pub sky: Option<ContentId>,
    pub postfx: Option<ContentId>,
    pub music: Option<ContentId>,
    pub time_limit_s: Option<f32>,
    pub intro: Intro,
    pub midground: Option<ContentId>,
}

impl MapToml {
    pub fn ids(&self) -> Vec<ContentId> {
        let ids = [&self.base, &self.lighting, &self.sky, &self.postfx, &self.music, &self.midground];
        ids.into_iter().flatten().cloned().collect()
    }
}

pub const MAX_TIME_LIMIT_S: f64 = 3_599.999;
pub const MAX_TITLE_CHARS: usize = 64;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawMapTable {
    pub(crate) title: Option<toml::Spanned<String>>,
    pub(crate) scene: Option<toml::Spanned<String>>,
    pub(crate) base: Option<toml::Spanned<String>>,
    pub(crate) modes: Option<toml::Spanned<Vec<toml::Spanned<String>>>>,
    pub(crate) lighting: Option<toml::Spanned<String>>,
    pub(crate) sky: Option<toml::Spanned<String>>,
    pub(crate) postfx: Option<toml::Spanned<String>>,
    pub(crate) music: Option<toml::Spanned<String>>,
    pub(crate) time_limit_s: Option<toml::Spanned<f64>>,
    pub(crate) intro: Option<toml::Spanned<String>>,
    pub(crate) midground: Option<toml::Spanned<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawMap {
    pub(crate) map: Option<toml::Spanned<RawMapTable>>,
}

fn id_of(
    s: &toml::Spanned<String>,
    kind: Kind,
    what: &str,
    at: &mut dyn FnMut(std::ops::Range<usize>, String),
) -> Option<ContentId> {
    match s.get_ref().parse::<ContentId>() {
        Ok(id) if id.kind() == kind => Some(id),
        Ok(id) => {
            at(s.span(), format!("{what}: {id} is not a {kind} id"));
            None
        }
        Err(e) => {
            at(s.span(), format!("{what}: {e}"));
            None
        }
    }
}

pub fn parse_toml(bytes: &[u8]) -> Result<MapToml, Vec<Problem>> {
    crate::package::parse_record_at(bytes).map_err(|p| vec![p])?;
    let text = std::str::from_utf8(bytes).map_err(|e| vec![(0, 0, format!("not UTF-8: {e}"))])?;
    let raw: RawMap = ::toml::from_str(text).map_err(|e| {
        let (line, column) = e.span().map_or((0, 0), |s| line_col(text, s.start));
        vec![(line, column, ssx_core::text::message(e.message()).into_string())]
    })?;
    let mut problems = Vec::new();
    let mut at = |span: std::ops::Range<usize>, m: String| {
        let (line, column) = line_col(text, span.start);
        problems.push((line, column, m));
    };
    let Some(table) = raw.map else {
        return Err(vec![(0, 0, "a map.toml has a [map] table".into())]);
    };
    let span = table.span();
    let m = table.into_inner();
    let title = match &m.title {
        Some(t)
            if (1..=MAX_TITLE_CHARS).contains(&t.get_ref().chars().count())
                && !t.get_ref().chars().any(char::is_control) =>
        {
            t.get_ref().clone()
        }
        Some(t) => {
            at(t.span(), format!("title: 1 to {MAX_TITLE_CHARS} characters, no control characters"));
            String::new()
        }
        None => {
            at(span.clone(), "the map has no title".into());
            String::new()
        }
    };
    let scene = match &m.scene {
        Some(s) => {
            let lower = s.get_ref().to_ascii_lowercase();
            if !(lower.ends_with(".glb") || lower.ends_with(".gltf")) {
                at(s.span(), "scene: a package path to a .glb or .gltf file".into());
            } else if let Err(e) = crate::path::check_package_path(s.get_ref(), false) {
                at(s.span(), format!("scene: {e}"));
            }
            s.get_ref().clone()
        }
        None => {
            at(span.clone(), "the map has no scene (a new track's world, or a layout's additions)".into());
            String::new()
        }
    };
    let base = m.base.as_ref().and_then(|b| {
        let id = id_of(b, Kind::Drop, "base", &mut at)?;
        if id.namespace() != BASE_NAMESPACE {
            at(b.span(), "base: a layout sits on a drop of the disc (ssx:drop/<ui_drop name>)".into());
            return None;
        }
        Some(id)
    });
    let mut modes = Vec::new();
    match &m.modes {
        Some(list) if !list.get_ref().is_empty() && list.get_ref().len() <= Mode::ALL.len() => {
            for s in list.get_ref() {
                match Mode::parse(s.get_ref()) {
                    Some(x) if !modes.contains(&x) => modes.push(x),
                    Some(_) => at(s.span(), format!("modes: {} twice", s.get_ref())),
                    None => at(s.span(), "modes: race, trick, survive, death or freeride".into()),
                }
            }
        }
        Some(list) => at(list.span(), "modes: one to five of race, trick, survive, death, freeride".into()),
        None => at(span.clone(), "the map names no modes (race, trick, survive, death, freeride)".into()),
    }
    let lighting = m.lighting.as_ref().and_then(|s| {
        let id = id_of(s, Kind::Lighting, "lighting", &mut at)?;
        if id.namespace() != BASE_NAMESPACE {
            at(s.span(), "lighting: a preset of the disc (ssx:lighting/<name>)".into());
            return None;
        }
        Some(id)
    });
    let sky = m.sky.as_ref().and_then(|s| id_of(s, Kind::Sky, "sky", &mut at));
    let postfx = m.postfx.as_ref().and_then(|s| {
        let id = id_of(s, Kind::Postfx, "postfx", &mut at)?;
        if id.namespace() != BASE_NAMESPACE {
            at(s.span(), "postfx: a preset of the disc (ssx:postfx/<name>)".into());
            return None;
        }
        Some(id)
    });
    let music = m.music.as_ref().and_then(|s| {
        let id = id_of(s, Kind::Music, "music", &mut at)?;
        if id.namespace() == BASE_NAMESPACE && !id.path().starts_with("licensed/") {
            at(s.span(), "music: a song of the disc (ssx:music/licensed/<id>) or a package's song or playlist".into());
            return None;
        }
        Some(id)
    });
    let time_limit_s = match &m.time_limit_s {
        Some(t) if t.get_ref().is_finite() && *t.get_ref() > 0.0 && *t.get_ref() <= MAX_TIME_LIMIT_S => {
            Some(*t.get_ref() as f32)
        }
        Some(t) => {
            at(t.span(), format!("time_limit_s: above 0, at most {MAX_TIME_LIMIT_S} (the event clock's ceiling)"));
            None
        }
        None => None,
    };
    let intro = match m.intro.as_ref().map(|s| (s.get_ref().as_str(), s.span())) {
        None | Some(("spawn", _)) => Intro::Spawn,
        Some(("heli", _)) => Intro::Heli,
        Some((_, span)) => {
            at(span, "intro: \"spawn\" or \"heli\"".into());
            Intro::Spawn
        }
    };
    let midground = m.midground.as_ref().and_then(|s| {
        let id = id_of(s, Kind::Track, "midground", &mut at)?;
        if id.namespace() != BASE_NAMESPACE {
            at(s.span(), "midground: a track of the disc (ssx:track/<name>), whose mid-ground the map shows".into());
            return None;
        }
        if base.is_some() {
            at(s.span(), "midground: a layout shows its drop's mid-ground; only a new track names one".into());
            return None;
        }
        Some(id)
    });
    if problems.is_empty() {
        Ok(MapToml { title, scene, base, modes, lighting, sky, postfx, music, time_limit_s, intro, midground })
    } else {
        problems.sort();
        Err(problems)
    }
}

pub fn from_table(table: &::toml::Table) -> Result<MapToml, Vec<Problem>> {
    let text = ::toml::to_string(table).map_err(|e| vec![(0, 0, format!("the record cannot be written: {e}"))])?;
    parse_toml(text.as_bytes())
}
