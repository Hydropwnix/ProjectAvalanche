use serde::{Deserialize, Serialize};

use crate::info::{INFO_VERSION, MAX_ANNOUNCED_PLAYERS};
use crate::name::player_name;

pub const MAX_PLAYERS_BYTES: usize = 8192;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerList {
    pub info_version: u32,
    pub players: Vec<String>,
}

impl PlayerList {
    pub fn new(players: Vec<String>) -> Self {
        Self { info_version: INFO_VERSION, players }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub fn from_json(body: &[u8]) -> Result<Self, String> {
        if body.len() > MAX_PLAYERS_BYTES {
            return Err(format!("{} bytes of players, at most {MAX_PLAYERS_BYTES}", body.len()));
        }
        let list: Self = serde_json::from_slice(body).map_err(|e| format!("the player list does not read: {e}"))?;
        if list.info_version != INFO_VERSION {
            return Err(format!("the player list is version {}, this client reads {INFO_VERSION}", list.info_version));
        }
        if list.players.len() > MAX_ANNOUNCED_PLAYERS as usize {
            return Err(format!("{} players listed, at most {MAX_ANNOUNCED_PLAYERS}", list.players.len()));
        }
        for name in &list.players {
            match player_name(name) {
                Ok(checked) if checked == *name => {}
                Ok(_) => return Err("a player name has spaces the name rule removes".into()),
                Err(e) => return Err(format!("a player name: {e}")),
            }
        }
        Ok(list)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_reading_and_the_largest_list() {
        let good = PlayerList::new(vec!["Elise".into(), "Zoë Payne".into()]);
        assert_eq!(PlayerList::from_json(good.to_json().as_bytes()), Ok(good.clone()));
        let full = PlayerList::new((0..64).map(|_| "Œ".repeat(20)).collect());
        let escaped = full.to_json().replace('Œ', "\\u0152");
        assert!(escaped.len() <= MAX_PLAYERS_BYTES, "{}", escaped.len());
        assert_eq!(PlayerList::from_json(escaped.as_bytes()), Ok(full));
        let json = good.to_json();
        for bad in [
            json.replacen(
                &format!("\"info_version\":{INFO_VERSION}"),
                &format!("\"info_version\":{}", INFO_VERSION - 1),
                1,
            ),
            json.replacen('}', ",\"keys\":[]}", 1),
            json.replacen("Elise", "Eli\\u0007se", 1),
            json.replacen("Elise", " Elise", 1),
            json.replacen("Elise", &"x".repeat(21), 1),
            json.replacen("Elise", "日本", 1),
            json.replacen("\"Elise\"", "1", 1),
            PlayerList::new((0..65).map(|i| format!("p{i}")).collect()).to_json(),
            format!("{{\"info_version\":{INFO_VERSION},\"players\":[\"{}\"]}}", "x".repeat(10_000)),
            format!("{{\"info_version\":{INFO_VERSION}}}"),
        ] {
            assert!(PlayerList::from_json(bad.as_bytes()).is_err(), "{}", &bad[..bad.len().min(120)]);
        }
    }
}
