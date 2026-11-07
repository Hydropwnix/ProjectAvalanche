use serde::{Deserialize, Serialize};

use crate::info::MAX_ANNOUNCED_PLAYERS;
use crate::info::present;
use crate::instance::{InstanceState, Mode, drop_name, gamemode_id, map_id};

pub const DYNAMIC_VERSION: u32 = 2;
pub const MAX_DYNAMIC_INSTANCES: usize = 8;
pub const MAX_DYNAMIC_BYTES: usize = 6144;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DynamicInstance {
    pub id: u16,
    pub mode: String,
    pub drop: String,
    #[serde(deserialize_with = "present")]
    pub map: Option<String>,
    #[serde(deserialize_with = "present")]
    pub gamemode: Option<String>,
    pub state: String,
    pub round: u16,
    pub players: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dynamic {
    pub dynamic_version: u32,
    pub players: u32,
    pub max_players: u32,
    pub instances: Vec<DynamicInstance>,
}

impl Dynamic {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub fn from_json(body: &[u8]) -> Result<Self, String> {
        if body.len() > MAX_DYNAMIC_BYTES {
            return Err(format!("{} bytes of dynamic state, at most {MAX_DYNAMIC_BYTES}", body.len()));
        }
        let d: Self = serde_json::from_slice(body).map_err(|e| format!("the dynamic state does not read: {e}"))?;
        d.validate()?;
        Ok(d)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.dynamic_version != DYNAMIC_VERSION {
            return Err(format!("dynamic state version {}, this reader reads {DYNAMIC_VERSION}", self.dynamic_version));
        }
        if self.max_players == 0 || self.max_players > MAX_ANNOUNCED_PLAYERS || self.players > self.max_players {
            return Err(format!("{} of {} players", self.players, self.max_players));
        }
        if self.instances.is_empty() || self.instances.len() > MAX_DYNAMIC_INSTANCES {
            return Err(format!("{} instances, 1 to {MAX_DYNAMIC_INSTANCES}", self.instances.len()));
        }
        let mut sum = 0u32;
        for (k, i) in self.instances.iter().enumerate() {
            if usize::from(i.id) != k + 1 {
                return Err(format!("instance {} where {} belongs", i.id, k + 1));
            }
            if Mode::parse(&i.mode).is_none() {
                return Err(format!("instance {}: the mode is not one this reader knows", i.id));
            }
            match &i.map {
                None => {
                    drop_name(&i.drop).map_err(|e| format!("instance {}: {e}", i.id))?;
                    if i.gamemode.is_some() {
                        return Err(format!("instance {}: a gamemode without a map", i.id));
                    }
                }
                Some(map) => {
                    if !i.drop.is_empty() {
                        return Err(format!("instance {}: a drop beside a map", i.id));
                    }
                    map_id(map).map_err(|e| format!("instance {}: {e}", i.id))?;
                    if let Some(g) = &i.gamemode {
                        gamemode_id(g).map_err(|e| format!("instance {}: {e}", i.id))?;
                    }
                }
            }
            if !InstanceState::ALL.iter().any(|s| s.name() == i.state) {
                return Err(format!("instance {}: the state is not one this reader knows", i.id));
            }
            sum = sum.saturating_add(i.players);
        }
        if sum != self.players {
            return Err(format!("the instances' players ({sum}) are not the server's ({})", self.players));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instance::MAX_MAP_ID;

    fn good() -> Dynamic {
        Dynamic {
            dynamic_version: DYNAMIC_VERSION,
            players: 5,
            max_players: 16,
            instances: vec![
                DynamicInstance {
                    id: 1,
                    mode: "race".into(),
                    drop: "alps_dd_race_1".into(),
                    map: None,
                    gamemode: None,
                    state: "running".into(),
                    round: 4,
                    players: 3,
                },
                DynamicInstance {
                    id: 2,
                    mode: "freeride".into(),
                    drop: "exp_alps_vorassay_drop_3".into(),
                    map: None,
                    gamemode: None,
                    state: "running".into(),
                    round: 0,
                    players: 2,
                },
            ],
        }
    }

    #[test]
    fn strict_reading() {
        let d = good();
        assert_eq!(Dynamic::from_json(d.to_json().as_bytes()), Ok(d.clone()));
        let json = d.to_json();
        for bad in [
            json.replacen(
                &format!("\"dynamic_version\":{DYNAMIC_VERSION}"),
                &format!("\"dynamic_version\":{}", DYNAMIC_VERSION - 1),
                1,
            ),
            json.replacen("\"players\":5", "\"players\":6", 1),
            json.replacen("\"players\":3", "\"players\":4", 1),
            json.replacen("\"max_players\":16", "\"max_players\":65", 1),
            json.replacen("\"id\":2", "\"id\":3", 1),
            json.replacen("\"race\"", "\"tag\"", 1),
            json.replacen("alps_dd_race_1", "../x", 1),
            json.replacen("\"state\":\"running\"", "\"state\":\"paused\"", 1),
            json.replacen("\"round\":4", "\"round\":-1", 1),
            json.replacen('}', ",\"key\":\"00\"}", 1),
            json.replacen("\"instances\":[", "\"instances\":[],\"x\":[", 1),
            format!("{json}{}", " ".repeat(MAX_DYNAMIC_BYTES)),
        ] {
            assert!(Dynamic::from_json(bad.as_bytes()).is_err(), "{bad}");
        }
        let none = Dynamic { instances: Vec::new(), players: 0, ..good() };
        assert!(none.validate().is_err());
    }

    #[test]
    fn a_map_reads_with_an_empty_drop_and_its_gamemode() {
        let mut d = good();
        let on_map = &mut d.instances[0];
        (on_map.drop, on_map.map, on_map.gamemode) =
            (String::new(), Some("slope:map/course".into()), Some("gm9:gamemode/race_clock".into()));
        assert_eq!(Dynamic::from_json(d.to_json().as_bytes()), Ok(d.clone()));
        let json = d.to_json();
        for bad in [
            json.replacen("\"drop\":\"\"", "\"drop\":\"alps_dd_race_1\"", 1),
            json.replacen("\"gamemode\":null", "\"gamemode\":\"gm9:gamemode/race_clock\"", 1),
            json.replacen(",\"map\":null", "", 1),
            json.replacen(",\"gamemode\":null", "", 1),
            json.replacen("slope:map/course", "slope:object/course", 1),
            json.replacen("slope:map/course", "Slope:map/course", 1),
            json.replacen("gm9:gamemode/race_clock", "gm9:map/race_clock", 1),
            json.replacen("gm9:gamemode/race_clock", "gm9:gamemode/../x", 1),
        ] {
            assert!(Dynamic::from_json(bad.as_bytes()).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_widest_body_fits_its_cap() {
        let mode = Mode::ALL.iter().map(|m| m.name()).max_by_key(|m| m.len()).expect("modes");
        let state = InstanceState::ALL.iter().map(|s| s.name()).max_by_key(|s| s.len()).expect("states");
        let instances: Vec<DynamicInstance> = (1..=MAX_DYNAMIC_INSTANCES as u16)
            .map(|id| DynamicInstance {
                id,
                mode: mode.into(),
                drop: String::new(),
                map: Some(format!("a:map/{}", "c".repeat(MAX_MAP_ID - 6))),
                gamemode: Some(format!("a:gamemode/{}", "c".repeat(MAX_MAP_ID - 11))),
                state: state.into(),
                round: u16::MAX,
                players: MAX_ANNOUNCED_PLAYERS / MAX_DYNAMIC_INSTANCES as u32,
            })
            .collect();
        let d = Dynamic {
            dynamic_version: DYNAMIC_VERSION,
            players: MAX_ANNOUNCED_PLAYERS,
            max_players: MAX_ANNOUNCED_PLAYERS,
            instances,
        };
        let json = d.to_json();
        assert!(json.len() <= MAX_DYNAMIC_BYTES, "{} > {MAX_DYNAMIC_BYTES}", json.len());
        assert_eq!(json.len(), 5_009, "the widest body (measured; stated at MAX_DYNAMIC_BYTES)");
        assert_eq!(Dynamic::from_json(json.as_bytes()), Ok(d));
    }
}
