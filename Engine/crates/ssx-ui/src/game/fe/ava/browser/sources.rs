use std::collections::HashMap;
use std::time::{Duration, Instant};

use ssx_net_join::browse::lan::{LanSearch, Targets, test_targets};
use ssx_net_join::browse::masters::{Masters, from_settings};
use ssx_net_join::browse::ping::Pinger;
use ssx_net_join::browse::{Entry, Host, ServerAddress, State};
use ssx_net_proto::query::info_hash;

use super::{Browser, LIST_INTERVAL, Tab};
use crate::game::fe::{FeEnv, FrontEnd};

pub const LAN_TARGETS_ENV: &str = "SSX_LAN_TARGETS";

pub const DETAIL_INTERVAL: Duration = Duration::from_millis(2500);

fn read_masters(fe: &FrontEnd) -> Result<Vec<ssx_net_proto::master::MasterUrl>, String> {
    let Some(path) = fe.ava.options.file.as_ref() else { return Ok(Vec::new()) };
    let map = crate::game::fe::ava::port_settings::read_file(path).map_err(|e| format!("{e:#}"))?;
    from_settings(map.get("masters")).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn open_tab(fe: &mut FrontEnd, tab: Tab) {
    let b = &mut fe.ava.browser;
    b.masters = None;
    b.lan = None;
    b.masters_problem = None;
    b.left_out = 0;
    if b.fake.is_some() {
        return;
    }
    match tab {
        Tab::Internet => match read_masters(fe) {
            Ok(list) if list.is_empty() => fe.note("browser: no master servers are set (settings.json `masters`)"),
            Ok(list) => {
                fe.note(format!(
                    "browser: asking the masters {}",
                    list.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
                ));
                fe.ava.browser.masters = Some(Masters::new(list, None));
                super::blocklists::start(fe);
            }
            Err(why) => {
                fe.note(format!("browser: the master list: {why}"));
                fe.ava.browser.masters_problem = Some(why);
            }
        },
        Tab::Lan => {
            let targets = match std::env::var(LAN_TARGETS_ENV) {
                Ok(text) => match test_targets(&text) {
                    Ok(list) => Targets::Given(list),
                    Err(why) => {
                        fe.note(format!("browser: {LAN_TARGETS_ENV}: {why}; no LAN search"));
                        fe.ava.browser.masters_problem = Some(format!("{LAN_TARGETS_ENV}: {why}"));
                        return;
                    }
                },
                Err(_) => Targets::Broadcast,
            };
            fe.note(format!("browser: LAN search to {targets:?}"));
            match LanSearch::start(targets) {
                Ok(s) => fe.ava.browser.lan = Some(s),
                Err(e) => fe.ava.browser.masters_problem = Some(format!("the LAN search: {e}")),
            }
        }
        Tab::Favourites | Tab::Recent => {}
    }
}

fn take_lists(b: &mut Browser, now: Instant) -> bool {
    let Some(m) = b.masters.as_mut() else { return false };
    if !m.poll(now) {
        return false;
    }
    let (listed, left_out) = m.merged();
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

pub fn masters_status(b: &Browser) -> Option<(usize, usize, Option<String>)> {
    let m = b.masters.as_ref()?;
    let answered = m.status.iter().filter(|(_, r)| r.is_ok()).count();
    let first_failure =
        m.status.iter().find_map(|(u, r)| r.as_ref().err().map(|e| format!("{u}: {e}").chars().take(120).collect()));
    Some((answered, m.masters().len(), first_failure))
}

pub fn refresh(fe: &mut FrontEnd, now: Instant) {
    if let Some(m) = fe.ava.browser.masters.as_mut()
        && m.refresh(now)
    {
        fe.note("browser: the masters asked again");
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
