pub mod chat;
pub mod pause;
pub mod players;
pub mod results;

use std::collections::BTreeSet;

pub use ssx_net_proto::PlayerStatus;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionPlayer {
    pub rider: u16,
    pub name: String,
    pub key_id: [u8; 8],
    pub character: Option<u32>,
    pub status: Option<PlayerStatus>,
    pub ping_ms: Option<u16>,
    pub own: bool,
    pub place: Option<u16>,
    pub standing: Option<ssx_net_proto::StandingRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sender {
    Player { name: String, key_id: [u8; 8] },
    Server,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    Text(String),
    Quick(u8),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatIn {
    Line { from: Sender, body: Body },
    Flood { wait_ms: u16 },
    Disabled,
    ModsOff { names: Vec<String>, whole: bool },
    Muted { until: Option<String>, reason: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatOut {
    Text(String),
    Quick(u8),
}

#[derive(Default)]
pub struct Session {
    pub active: bool,
    pub server: String,
    pub max_players: Option<u16>,
    pub players: Vec<SessionPlayer>,
    pub muted: BTreeSet<[u8; 8]>,
    pub chat: chat::Chat,
    pub quick: chat::Quick,
    pub list: players::Players,
    pub rounds: Option<ssx_net_proto::Mode>,
    pub banner: String,
    pub banner_shown: String,
    pub results_exit: Option<(results::ResultsExit, f64)>,
    pub results_quit: Option<f64>,
}

impl Session {
    pub fn begin(&mut self, server: &str, max_players: Option<u16>) {
        *self = Session { active: true, server: server.to_string(), max_players, ..Session::default() };
    }

    pub fn end(&mut self) {
        *self = Session::default();
    }

    pub fn set_players(&mut self, mut players: Vec<SessionPlayer>) {
        players.sort_by(|a, b| {
            let place = |p: &SessionPlayer| p.place.map_or(u32::MAX, u32::from);
            place(a).cmp(&place(b)).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())).then(a.rider.cmp(&b.rider))
        });
        if players != self.players {
            self.players = players;
            self.list.dirty = true;
        }
    }

    pub fn muted(&self, key_id: &[u8; 8]) -> bool {
        self.muted.contains(key_id)
    }

    pub fn toggle_mute(&mut self, key_id: [u8; 8]) -> bool {
        if self.players.iter().any(|p| p.own && p.key_id == key_id) {
            return false;
        }
        if !self.muted.remove(&key_id) {
            self.muted.insert(key_id);
        }
        self.chat.dirty = true;
        self.list.dirty = true;
        true
    }
}
