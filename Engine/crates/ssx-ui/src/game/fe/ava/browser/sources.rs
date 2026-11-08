use std::collections::HashMap;
use std::time::{Duration, Instant};

use ssx_net_join::browse::lan::{LanSearch, Targets, test_targets};
use ssx_net_join::browse::masters::MasterList;
use ssx_net_join::browse::ping::Pinger;
use ssx_net_join::browse::{Entry, Host, ServerAddress, State};
use ssx_net_proto::query::info_hash;

use super::{Browser, LIST_INTERVAL, Tab};
use crate::game::fe::{FeEnv, FrontEnd};

pub const LAN_TARGETS_ENV: &str = "SSX_LAN_TARGETS";

pub const DETAIL_INTERVAL: Duration = Duration::from_millis(2500);

pub fn internet_master() -> Option<ssx_net_proto::master::MasterUrl> {
    ssx_net_proto::master::the_master()
}

fn note_old_masters(fe: &mut FrontEnd) {
    if std::mem::replace(&mut fe.ava.browser.said_old_masters, true) {
        return;
    }
    let read = fe.ava.options.file.as_ref().and_then(|p| super::super::port_settings::read_file(p).ok());
    if let Some(line) = read.as_ref().and_then(super::super::port_settings::old_masters_line) {
        fe.note(format!("browser: {line}"));
    }
}

pub fn open_tab(fe: &mut FrontEnd, tab: Tab) {
    let b = &mut fe.ava.browser;
    b.master = None;
    b.lan = None;
    b.source_problem = None;
    b.left_out = 0;
    if b.fake.is_some() {
        return;
    }
    match tab {
        Tab::Internet => {
            note_old_masters(fe);
            match internet_master() {
                Some(url) => {
                    fe.note(format!("browser: asking the master {url}"));
                    fe.ava.browser.master = Some(MasterList::new(url, None));
                    super::blocklists::start(fe);
                }
                None => fe.note(format!("browser: no master: {}", ssx_core::master::master().describe())),
            }
        }
        Tab::Lan => {
            let targets = match std::env::var(LAN_TARGETS_ENV) {
                Ok(text) => match test_targets(&text) {
                    Ok(list) => Targets::Given(list),
                    Err(why) => {
                        fe.note(format!("browser: {LAN_TARGETS_ENV}: {why}; no LAN search"));
                        fe.ava.browser.source_problem = Some(format!("{LAN_TARGETS_ENV}: {why}"));
                        return;
                    }
                },
                Err(_) => Targets::Broadcast,
            };
            fe.note(format!("browser: LAN search to {targets:?}"));
            match LanSearch::start(targets) {
                Ok(s) => fe.ava.browser.lan = Some(s),
                Err(e) => fe.ava.browser.source_problem = Some(format!("the LAN search: {e}")),
            }
        }
        Tab::Favourites | Tab::Recent => {}
    }
}

fn take_lists(b: &mut Browser, now: Instant) -> bool {
    let Some(m) = b.master.as_mut() else { return false };
    if !m.poll(now) {
        return false;
    }
    let (listed, left_out) = m.listed();
    b.left_out = left_out;
    let mut old: HashMap<ServerAddress, Entry> = b.entries.drain(..).map(|e| (e.address.clone(), e)).collect();
    b.entries = listed
        .into_iter()
        .map(|l| {
            let mut e = old.remove(&l.address).unwrap_or_else(|| Entry::new(l.address.clone(), None));
            if e.listed.as_ref().is_some_and(|x| x.key != l.key) {
                e = Entry::new(l.address.clone(), None);
            }
            e.listed = Some(Box::new(l));
            e
        })
        .collect();
    true
}

fn take_lan(b: &mut Browser) -> bool {
    let Some(s) = b.lan.as_ref() else { return false };
    let mut changed = false;
    for f in s.take() {
        let address = ServerAddress { host: Host::Ip(f.game.ip()), port: f.game.port() };
        if !b.entries.iter().any(|e| e.address == address) {
            b.entries.push(Entry::new(address, None));
            changed = true;
        }
    }
    changed
}

fn pings(b: &mut Browser, selected: Option<&ServerAddress>, now: Instant) -> bool {
    let Some(p) = b.pinger.as_mut() else { return false };
    for e in b.entries.iter().filter(|e| !e.fake) {
        let Some(socket) = e.socket else { continue };
        let interval = if selected == Some(&e.address) { DETAIL_INTERVAL } else { LIST_INTERVAL };
        p.ask(socket, interval, now);
    }
    let mut changed = false;
    for r in p.take() {
        for e in b.entries.iter_mut().filter(|e| e.socket == Some(r.game)) {
            match &r.result {
                Ok((answer, rtt)) => {
                    e.query = Some(*answer);
                    e.ping = Some(*rtt);
                    e.query_why = None;
                }
                Err(why) => {
                    e.query = None;
                    e.ping = None;
                    e.query_why = Some(why.clone());
                }
            }
            changed = true;
        }
    }
    changed
}

pub fn info_changed(e: &Entry) -> bool {
    match &e.state {
        State::Info(i) => e.query.is_some_and(|q| q.info_hash != info_hash(i)),
        _ => false,
    }
}

pub fn update(fe: &mut FrontEnd, _env: &mut FeEnv, selected: Option<ServerAddress>, now: Instant) -> bool {
    super::blocklists::poll(fe);
    let b = &mut fe.ava.browser;
    let lists = take_lists(b, now);
    let lan = take_lan(b);
    let pinged = pings(b, selected.as_ref(), now);
    lists || lan || pinged
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MasterState {
    Waiting,
    Answered,
    Failed(String),
}

pub fn master_state(b: &Browser) -> Option<MasterState> {
    let m = b.master.as_ref()?;
    Some(match &m.status {
        Some(Ok(_)) => MasterState::Answered,
        Some(Err(e)) => MasterState::Failed(e.chars().take(120).collect()),
        None => MasterState::Waiting,
    })
}

pub fn refresh(fe: &mut FrontEnd, now: Instant) {
    if let Some(m) = fe.ava.browser.master.as_mut()
        && m.refresh(now)
    {
        fe.note("browser: the master asked again");
        super::blocklists::start(fe);
    }
}

pub fn start_pings(fe: &mut FrontEnd) {
    if fe.ava.browser.fake.is_some() || fe.ava.browser.pinger.is_some() {
        return;
    }
    match Pinger::start() {
        Ok(p) => fe.ava.browser.pinger = Some(p),
        Err(e) => fe.note(format!("browser: ERROR the ping's socket: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_internet_tab_asks_the_builds_master_whatever_the_settings_say() {
        let m = internet_master();
        assert_eq!(m.as_ref().map(ToString::to_string).as_deref(), ssx_core::master::master().url());
        assert_eq!(m, ssx_net_proto::master::the_master());
        let file: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(r#"{"masters": ["https://m.example.org"], "vsync": false}"#).unwrap();
        let line = crate::game::fe::ava::port_settings::old_masters_line(&file).expect("noted");
        assert!(line.contains("`masters` is not read") && line.contains(ssx_core::OFFICIAL_MASTER), "{line}");
        assert!(!line.contains("m.example.org"), "the old value is never shown or used");
        assert_eq!(crate::game::fe::ava::port_settings::old_masters_line(&serde_json::Map::new()), None);
        let (s, problems) =
            crate::game::fe::ava::port_settings::PortSettings::from_json(&serde_json::Value::Object(file));
        assert!(!s.vsync && problems.is_empty(), "{problems:?}");
    }
}
