use ssx_content::gamemode::{GamemodeToml, PairError, Rules};
use ssx_content::map::{Map, MapToml, Mode};
use ssx_content::{ContentId, Kind, Layer, LoadError};

use super::GameContent;
use super::resolve::Found;

pub const MAX_LISTED_GAMEMODES: usize = 256;
pub const MAX_MAP_MODES: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ModeChoice {
    BuiltIn(Mode),
    Gamemode(ContentId),
}

impl ModeChoice {
    pub fn parse(s: &str) -> Result<Self, String> {
        if let Some(m) = Mode::parse(s) {
            return Ok(ModeChoice::BuiltIn(m));
        }
        let id: ContentId =
            s.parse().map_err(|e| format!("not a mode (race, trick, survive, death, freeride) nor an id: {e}"))?;
        if id.kind() != Kind::Gamemode {
            return Err(format!("{id}: not a gamemode id (`<namespace>:gamemode/…`)"));
        }
        Ok(ModeChoice::Gamemode(id))
    }

    pub fn name(&self) -> String {
        match self {
            ModeChoice::BuiltIn(m) => m.name().to_string(),
            ModeChoice::Gamemode(id) => id.to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MapMode {
    pub choice: ModeChoice,
    pub title: Option<String>,
    pub event_type: u32,
    pub rules: Result<Rules, String>,
}

#[derive(Clone, Debug)]
pub struct ResolvedMode {
    pub choice: ModeChoice,
    pub rules: Rules,
    pub gamemode: Option<Found<GamemodeToml>>,
}

pub fn map_limit_ms(t: &MapToml) -> Option<u32> {
    t.time_limit_s.map(|s| (f64::from(s) * 1000.0).round() as u32)
}

pub fn finish_from_record(t: &MapToml) -> Option<bool> {
    (t.base.is_none() && t.modes.iter().any(|m| m.needs_finish())).then_some(true)
}

impl MapMode {
    pub fn medal_targets(&self) -> Vec<u32> {
        match (&self.choice, &self.rules) {
            (ModeChoice::Gamemode(_), Ok(r)) if r.rivals() != ssx_content::gamemode::Rivals::None => r.board_values(),
            _ => Vec::new(),
        }
    }
}

impl GameContent {
    pub fn gamemode(&self, id: &ContentId) -> Result<Found<GamemodeToml>, LoadError> {
        if id.kind() != Kind::Gamemode {
            return Err(LoadError::Failed { id: id.clone(), message: "not a gamemode id".into() });
        }
        self.strongest_in(id, self.session_skip(), |_: &GamemodeToml| None)
    }

    pub fn listed_gamemodes(&self) -> (Vec<ContentId>, usize) {
        self.with_registry(|r| {
            let ids = r.list_above(Layer::Base, Kind::Gamemode);
            let more = ids.len().saturating_sub(MAX_LISTED_GAMEMODES);
            (ids.into_iter().take(MAX_LISTED_GAMEMODES).collect(), more)
        })
    }

    pub fn map_modes(&self, id: &ContentId, t: &MapToml) -> Vec<MapMode> {
        let has_finish = finish_from_record(t).unwrap_or(true);
        let limit = map_limit_ms(t);
        let mut out: Vec<MapMode> = t
            .modes
            .iter()
            .map(|&m| MapMode {
                choice: ModeChoice::BuiltIn(m),
                title: None,
                event_type: m.event_type() as u32,
                rules: Rules::plain(m.event_type(), limit).map_err(|e| e.to_string()),
            })
            .collect();
        let (ids, _) = self.listed_gamemodes();
        for g in ids {
            if out.len() >= MAX_MAP_MODES {
                break;
            }
            let Ok(found) = self.gamemode(&g) else { continue };
            let toml = &found.value;
            if toml.entry(id).is_none() && !toml.any_map {
                continue;
            }
            out.push(MapMode {
                choice: ModeChoice::Gamemode(g),
                title: Some(toml.title.clone()),
                event_type: toml.event_type as u32,
                rules: toml.rules(id, limit, has_finish).map_err(|e| e.to_string()),
            });
        }
        out
    }

    pub fn resolve_mode(
        &self,
        map: &Found<Map>,
        choice: &ModeChoice,
        has_finish: bool,
    ) -> Result<ResolvedMode, String> {
        let limit = map_limit_ms(&map.value.toml);
        match choice {
            ModeChoice::BuiltIn(m) => {
                if !map.value.toml.modes.contains(m) {
                    return Err(format!("{}: the map does not offer the mode {}", map.id, m.name()));
                }
                if m.needs_finish() && !has_finish {
                    return Err(format!("{}: the mode {} ends at a finish, and the map has none", map.id, m.name()));
                }
                let rules = Rules::plain(m.event_type(), limit).map_err(|e| format!("{}: {e}", map.id))?;
                Ok(ResolvedMode { choice: choice.clone(), rules, gamemode: None })
            }
            ModeChoice::Gamemode(g) => {
                let found = self.gamemode(g).map_err(|e| e.to_string())?;
                let rules = found
                    .value
                    .rules(&map.id, limit, has_finish)
                    .map_err(|e: PairError| format!("{g} on {}: {e}", map.id))?;
                Ok(ResolvedMode { choice: choice.clone(), rules, gamemode: Some(found) })
            }
        }
    }
}

#[cfg(test)]
mod tests;
