pub mod record;
pub mod template;
mod toml;

#[cfg(test)]
mod tests;

pub use ssx_rules::events::EventType;
pub use ssx_rules::gamemode::{Measure, Record, Rivals, Rules, RulesError, RunOutcome, TimeUp};
pub use toml::{RawGamemode, RawGamemodeTable, RawMapEntry, parse_toml};

use crate::hash::ContentHash;
use crate::id::ContentId;

pub const GAMEMODE_FILE: &str = "gamemode.toml";

pub const EVENTS: [(&str, EventType); 9] = [
    ("trick_score", EventType::TrickScore),
    ("trick_bots", EventType::TrickBots),
    ("trick_ghosts", EventType::TrickGhosts),
    ("survive_to_bottom", EventType::SurviveToBottom),
    ("survive_to_death", EventType::SurviveToDeath),
    ("race_clock", EventType::RaceClock),
    ("race_ghosts", EventType::RaceGhosts),
    ("race_bots", EventType::RaceBots),
    ("freeride", EventType::FreeRide),
];

pub fn event_name(t: EventType) -> &'static str {
    EVENTS[t as usize].0
}

pub const MAX_MAPS: usize = 256;
pub const MAX_TITLE_CHARS: usize = crate::map::MAX_TITLE_CHARS;
pub const MAX_TIME_LIMIT_S: f64 = 3_599.999;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapEntry {
    pub map: ContentId,
    pub targets: Vec<u32>,
    pub time_limit_ms: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GamemodeToml {
    pub title: String,
    pub event_type: EventType,
    pub time_up: Option<TimeUp>,
    pub time_limit_ms: Option<u32>,
    pub any_map: bool,
    pub maps: Vec<MapEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PairError {
    NotTaken,
    NoFinish,
    Rules(RulesError),
    NoHelicopter,
}

pub fn event_takes_start(t: EventType, intro: crate::map::Intro) -> Result<(), PairError> {
    match (t, intro) {
        (EventType::SurviveToDeath, crate::map::Intro::Spawn) => Err(PairError::NoHelicopter),
        _ => Ok(()),
    }
}

impl std::fmt::Display for PairError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PairError::NotTaken => write!(f, "the gamemode does not name this map ([[gamemode.map]]) nor take any_map"),
            PairError::NoFinish => write!(f, "the map has no finish, and the gamemode's event ends at one"),
            PairError::Rules(e) => write!(f, "{e}"),
            PairError::NoHelicopter => write!(
                f,
                "the gamemode's survive to death loops through the helicopter's pickup, and the map starts at its spawn \
                 (intro = \"heli\" starts it in the helicopter)"
            ),
        }
    }
}

impl std::error::Error for PairError {}

impl GamemodeToml {
    pub fn entry(&self, map: &ContentId) -> Option<&MapEntry> {
        self.maps.iter().find(|e| &e.map == map)
    }

    pub fn needs_finish(&self) -> bool {
        self.event_type != EventType::FreeRide
    }

    pub fn takes_start(&self, intro: crate::map::Intro) -> Result<(), PairError> {
        event_takes_start(self.event_type, intro)
    }

    pub fn rules(&self, map: &ContentId, map_limit_ms: Option<u32>, has_finish: bool) -> Result<Rules, PairError> {
        let entry = self.entry(map);
        if entry.is_none() && !self.any_map {
            return Err(PairError::NotTaken);
        }
        if self.needs_finish() && !has_finish {
            return Err(PairError::NoFinish);
        }
        let targets = entry.map_or(&[][..], |e| e.targets.as_slice());
        let limit = entry.and_then(|e| e.time_limit_ms).or(self.time_limit_ms).or(map_limit_ms);
        Rules::new(self.event_type, targets, limit, self.time_up).map_err(PairError::Rules)
    }
}

pub const DOMAIN: &[u8] = b"project-avalanche/gamemode-sim/1\n";

pub fn sim_hash(r: &Rules) -> ContentHash {
    let mut h = blake3::Hasher::new();
    h.update(DOMAIN);
    h.update(&(r.event_type() as u64).to_le_bytes());
    h.update(&u64::from(r.time_limit_ms().unwrap_or(u32::MAX)).to_le_bytes());
    h.update(&[r.time_up() as u8]);
    h.update(&(r.targets().len() as u64).to_le_bytes());
    for t in r.targets() {
        h.update(&u64::from(*t).to_le_bytes());
    }
    ContentHash(*h.finalize().as_bytes())
}
