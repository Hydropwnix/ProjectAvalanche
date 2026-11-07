use core::time::Duration;

use ssx_net_proto::info::{InfoHead, ServerInfo};
use ssx_net_proto::query::{QueryAnswer, mode_name};

use super::address::ServerAddress;
use super::masters::Listed;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Pending,
    Info(Box<ServerInfo>),
    Incompatible { head: InfoHead, why: String },
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub address: ServerAddress,
    pub saved_name: Option<String>,
    pub state: State,
    pub ping: Option<Duration>,
    pub players: Option<Vec<String>>,
    pub players_why: Option<String>,
    pub fake: bool,
    pub listed: Option<Box<Listed>>,
    pub query: Option<QueryAnswer>,
    pub query_why: Option<String>,
    pub socket: Option<std::net::SocketAddr>,
    pub need: Option<crate::resources::Need>,
    pub blocked: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    pub name: String,
    pub players: u32,
    pub max_players: u32,
    pub password: bool,
    pub mode: String,
    pub drop: String,
    pub map: Option<ssx_net_proto::instance::MapSummary>,
    pub tags: Vec<String>,
    pub region: String,
    pub resources: ssx_net_proto::resources::ResourceSummary,
}

impl Summary {
    pub fn level_title(&self, drop_title: &dyn Fn(&str) -> String) -> String {
        match &self.map {
            Some(m) => m.title.clone(),
            None => drop_title(&self.drop),
        }
    }
}

impl Entry {
    pub fn new(address: ServerAddress, saved_name: Option<String>) -> Self {
        let socket = match address.host {
            super::address::Host::Ip(ip) => Some(std::net::SocketAddr::new(ip, address.port)),
            super::address::Host::Name(_) => None,
        };
        Self {
            address,
            saved_name,
            state: State::Pending,
            ping: None,
            players: None,
            players_why: None,
            fake: false,
            listed: None,
            query: None,
            query_why: None,
            socket,
            need: None,
            blocked: None,
        }
    }

    pub fn info(&self) -> Option<&ServerInfo> {
        match &self.state {
            State::Info(i) => Some(i.as_ref()),
            _ => None,
        }
    }

    pub fn name(&self) -> String {
        match &self.state {
            State::Info(i) => i.name.clone(),
            State::Incompatible { head: InfoHead { name: Some(n), .. }, .. } => n.clone(),
            _ => match &self.listed {
                Some(l) => l.server.name.clone(),
                None => self.saved_name.clone().unwrap_or_else(|| self.address.to_string()),
            },
        }
    }

    pub fn summary(&self) -> Option<Summary> {
        let listed = self.listed.as_deref().map(|l| &l.server);
        let mut s = match (self.info(), listed) {
            (Some(i), _) => Summary {
                name: i.name.clone(),
                players: i.players,
                max_players: i.max_players,
                password: i.password,
                mode: i.mode.clone(),
                drop: i.drop.clone(),
                map: i.map.clone(),
                tags: listed.map(|l| l.tags.clone()).unwrap_or_default(),
                region: listed.map(|l| l.region.clone()).unwrap_or_default(),
                resources: i.resources.clone(),
            },
            (None, Some(l)) => Summary {
                name: l.name.clone(),
                players: l.players,
                max_players: l.max_players,
                password: l.password,
                mode: l.mode.clone(),
                drop: l.drop.clone(),
                map: l.map.clone(),
                tags: l.tags.clone(),
                region: l.region.clone(),
                resources: l.resources.clone(),
            },
            (None, None) => return None,
        };
        if let Some(q) = &self.query
            && u32::from(q.max_players) == s.max_players
        {
            s.players = u32::from(q.players);
            s.password = q.password;
            if let Some(m) = mode_name(q.mode) {
                s.mode = m.to_string();
            }
        }
        Some(s)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortKey {
    #[default]
    Name,
    Players,
    Ping,
    Mode,
    Drop,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filters {
    pub not_full: bool,
    pub not_empty: bool,
    pub no_password: bool,
    pub mode: Option<String>,
    pub compatible: bool,
    pub sort: SortKey,
    pub descending: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Term {
    pub negated: bool,
    pub category: Option<Category>,
    pub value: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Name,
    Player,
    Mode,
    Drop,
    Address,
    Tag,
    Region,
}

impl Category {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "name" => Self::Name,
            "player" => Self::Player,
            "mode" => Self::Mode,
            "drop" => Self::Drop,
            "address" => Self::Address,
            "tag" => Self::Tag,
            "region" => Self::Region,
            _ => return None,
        })
    }
}

pub const MAX_TERMS: usize = 16;

pub fn parse_search(text: &str) -> Vec<Term> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in text.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    words.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    words
        .into_iter()
        .take(MAX_TERMS)
        .filter_map(|w| {
            let (negated, w) = match w.strip_prefix('~') {
                Some(rest) => (true, rest.to_string()),
                None => (false, w),
            };
            let (category, value) = match w.split_once(':') {
                Some((c, v)) => match Category::parse(&c.to_lowercase()) {
                    Some(cat) => (Some(cat), v.to_string()),
                    None => (None, w.clone()),
                },
                None => (None, w),
            };
            (!value.is_empty()).then(|| Term { negated, category, value: value.to_lowercase() })
        })
        .collect()
}

pub struct ViewParams<'a> {
    pub filters: &'a Filters,
    pub search: &'a [Term],
    pub drop_title: &'a dyn Fn(&str) -> String,
}

fn has(hay: &str, needle: &str) -> bool {
    hay.to_lowercase().contains(needle)
}

fn term_matches(e: &Entry, t: &Term, p: &ViewParams) -> bool {
    let s = e.summary();
    let players = || e.players.iter().flatten();
    let found = match t.category {
        None => has(&e.name(), &t.value) || players().any(|n| has(n, &t.value)),
        Some(Category::Name) => has(&e.name(), &t.value),
        Some(Category::Player) => players().any(|n| has(n, &t.value)),
        Some(Category::Mode) => s.as_ref().is_some_and(|s| has(&s.mode, &t.value)),
        Some(Category::Drop) => s.as_ref().is_some_and(|s| {
            has(&s.drop, &t.value)
                || has(&s.level_title(p.drop_title), &t.value)
                || s.map.as_ref().is_some_and(|m| has(&m.id, &t.value))
        }),
        Some(Category::Address) => has(&e.address.to_string(), &t.value),
        Some(Category::Tag) => s.as_ref().is_some_and(|s| s.tags.iter().any(|g| has(g, &t.value))),
        Some(Category::Region) => s.as_ref().is_some_and(|s| has(&s.region, &t.value)),
    };
    found != t.negated
}

fn passes(e: &Entry, p: &ViewParams) -> bool {
    let f = p.filters;
    if f.compatible && matches!(e.state, State::Incompatible { .. }) {
        return false;
    }
    if let Some(s) = e.summary() {
        if (f.not_full && s.players >= s.max_players)
            || (f.not_empty && s.players == 0)
            || (f.no_password && s.password)
            || f.mode.as_ref().is_some_and(|m| *m != s.mode)
        {
            return false;
        }
    } else if f.mode.is_some() {
        return false;
    }
    p.search.iter().all(|t| term_matches(e, t, p))
}

pub fn view(entries: &[Entry], p: &ViewParams) -> Vec<usize> {
    let mut rows: Vec<usize> = (0..entries.len()).filter(|&i| passes(&entries[i], p)).collect();
    let names: Vec<String> = entries.iter().map(|e| e.name().to_lowercase()).collect();
    let key = |i: usize| -> (bool, i64, String) {
        let e = &entries[i];
        let s = e.summary();
        match p.filters.sort {
            SortKey::Name => (false, 0, String::new()),
            SortKey::Players => (s.is_none(), s.as_ref().map_or(0, |s| i64::from(s.players)), String::new()),
            SortKey::Ping => (e.ping.is_none(), e.ping.map_or(0, |d| d.as_micros() as i64), String::new()),
            SortKey::Mode => (s.is_none(), 0, s.map_or_else(String::new, |s| s.mode)),
            SortKey::Drop => {
                (s.is_none(), 0, s.map_or_else(String::new, |s| s.level_title(p.drop_title).to_lowercase()))
            }
        }
    };
    let keys: Vec<Option<(bool, i64, String)>> =
        (0..entries.len()).map(|i| rows.binary_search(&i).is_ok().then(|| key(i))).collect();
    let empty = (false, 0, String::new());
    rows.sort_by(|&a, &b| {
        let (ka, kb) = (keys[a].as_ref().unwrap_or(&empty), keys[b].as_ref().unwrap_or(&empty));
        let mut by = ka.1.cmp(&kb.1).then_with(|| ka.2.cmp(&kb.2));
        if p.filters.descending {
            by = by.reverse();
        }
        let mut by_name = names[a].cmp(&names[b]);
        if p.filters.descending && p.filters.sort == SortKey::Name {
            by_name = by_name.reverse();
        }
        ka.0.cmp(&kb.0).then(by).then(by_name).then_with(|| entries[a].address.cmp(&entries[b].address))
    });
    rows
}

#[cfg(test)]
mod tests;
