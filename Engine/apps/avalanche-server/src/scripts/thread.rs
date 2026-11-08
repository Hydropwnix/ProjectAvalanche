use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender};
use std::time::{Duration, Instant};

use ssx_core::text::{MESSAGE_CHARS, shown, shown_max};
use ssx_net::server::Shared;
use ssx_net_proto::scripts::ScriptItem;
use ssx_script::host::game::Catalog;
use ssx_script::host::monitor::Monitor;
use ssx_script::host::outbox::{Body, Destination, Message, Peer};
use ssx_script::host::server::joins::JoinVerdict;
use ssx_script::host::server::rights::Rights;
use ssx_script::host::server::{ServerFrame, ServerOrders};
use ssx_script::host::wire::{from_item, to_item};
use ssx_script::host::{ClientId, CommandOrigin, HostConfig, ScriptHost, Side};

use super::joins::DEFER_BOUND;
use super::lifecycle::Transaction;

pub(super) enum Job {
    Tick(Box<TickJob>),
    Transact(Transaction, SyncSender<Result<Vec<String>, String>>),
    Catalog(Arc<Catalog>),
    Command(String, Option<CommandReply>),
    Rights(Arc<dyn Rights>),
    Stop,
}

pub(crate) type CommandReply = tokio::sync::oneshot::Sender<(bool, Vec<String>)>;

#[derive(Default)]
pub(super) struct TickJob {
    pub left: Vec<(u32, String)>,
    pub joined: Vec<(u32, String)>,
    pub inbound: Vec<(u32, ScriptItem)>,
    pub resync: Vec<u32>,
    pub connecting: Vec<(u64, String, String)>,
    pub forgotten: Vec<u64>,
    pub flooded: Vec<(u32, u64)>,
    pub frame: ServerFrame,
    pub monitor: bool,
}

pub(super) enum Verdict {
    Accept,
    Refuse(String),
}

#[derive(Default)]
pub(super) struct Done {
    pub firsts: Vec<(u32, Vec<Shared>)>,
    pub out: Vec<(Vec<u32>, Shared, usize)>,
    pub orders: ServerOrders,
    pub verdicts: Vec<(u64, Verdict)>,
    pub holding: Vec<(u64, String)>,
    pub monitor: Option<(Monitor, usize)>,
    pub said: Vec<(bool, String)>,
    pub took: Duration,
    pub refused_inbound: u64,
    pub unencodable: u64,
    pub ticked: bool,
    pub packages: usize,
    pub spent: Vec<(String, Duration)>,
}

const MAX_SAID: usize = 64;

pub(super) const PRINT_CHARS: usize = ssx_script::MAX_OUTPUT_LINE + 128;

const MAX_LATENT: usize = 4096;

struct State {
    host: ScriptHost,
    held: BTreeMap<u64, (u64, Instant, Option<String>)>,
    latent: BTreeMap<u64, (Vec<u32>, u64, u64)>,
}

pub(super) fn run(
    storage: PathBuf,
    jobs: Receiver<Job>,
    done: SyncSender<Done>,
    ready: SyncSender<Result<(), String>>,
) {
    let mut config = HostConfig::new(Side::Server);
    config.storage_root = Some(storage);
    let host = match ScriptHost::new(config) {
        Ok(h) => {
            let _ = ready.send(Ok(()));
            h
        }
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    let mut s = State { host, held: BTreeMap::new(), latent: BTreeMap::new() };
    while let Ok(job) = jobs.recv() {
        match job {
            Job::Tick(t) => {
                if done.send(s.tick(*t)).is_err() {
                    return;
                }
            }
            Job::Transact(t, reply) => {
                let (answer, undo) = match t.run(&s.host) {
                    Ok((said, undo)) => (Ok(said), Some(undo)),
                    Err(e) => (Err(e), None),
                };
                if reply.send(answer).is_err()
                    && let Some(undo) = undo
                {
                    let mut d = Done::default();
                    d.said.push((true, "scripts: a resource change was given up by the server: undone".into()));
                    d.said.extend(
                        undo.run(&s.host)
                            .into_iter()
                            .map(|l| (true, format!("scripts: {}", shown_max(&l, MESSAGE_CHARS)))),
                    );
                    if done.send(d).is_err() {
                        return;
                    }
                }
            }
            Job::Catalog(c) => s.host.set_catalog(c),
            Job::Rights(r) => s.host.set_rights(r),
            Job::Command(line, reply) => {
                let name = line.split_ascii_whitespace().next().unwrap_or_default();
                let said = if !s.host.has_command(name) {
                    let help = crate::console::HELP;
                    (
                        true,
                        format!(
                            "unknown command {}: no server script registered it; {help}",
                            shown_max(&line, MESSAGE_CHARS)
                        ),
                    )
                } else {
                    match s.host.execute_command(CommandOrigin::Console, &line) {
                        Ok(0) => (
                            true,
                            format!(
                                "scripts: {}: every handler failed (the lines above say why)",
                                shown_max(&line, MESSAGE_CHARS)
                            ),
                        ),
                        Ok(_) => (false, format!("scripts: ran {}", shown_max(&line, MESSAGE_CHARS))),
                        Err(e) => (
                            true,
                            format!("scripts: {}: {}", shown_max(&line, MESSAGE_CHARS), shown_max(&e, MESSAGE_CHARS)),
                        ),
                    }
                };
                if let Some(reply) = reply {
                    let _ = reply.send((!said.0, vec![said.1.clone()]));
                }
                let mut d = Done::default();
                d.said.push(said);
                s.take_output(&mut d);
                if done.send(d).is_err() {
                    return;
                }
            }
            Job::Stop => return,
        }
    }
}

impl State {
    fn tick(&mut self, t: TickJob) -> Done {
        let started = Instant::now();
        let mut d = Done::default();
        for (c, why) in &t.left {
            self.host.player_left(ClientId(*c), why);
        }
        let gone: Vec<u32> = t.left.iter().map(|(c, _)| *c).collect();
        let before = self.host.monitor().refused_inbound;
        for (c, item) in t.inbound {
            if gone.contains(&c) {
                continue;
            }
            let (package, body) = from_item(item);
            if let Err(e) = self.host.deliver(Peer::Client(ClientId(c)), package.as_deref(), body)
                && d.said.len() < MAX_SAID
            {
                d.said.push((true, format!("scripts: client {c}: refused: {}", shown_max(&e, MESSAGE_CHARS))));
            }
        }
        for (c, n) in &t.flooded {
            if !gone.contains(c) {
                self.host.player_flooded(ClientId(*c), *n);
            }
        }
        self.drain(&mut d, &t.resync);
        for (c, name) in &t.joined {
            match self.host.join_session(ClientId(*c), name) {
                Ok((_, first)) => {
                    let first = first.iter().filter_map(|m| encode(m, &mut d)).collect();
                    d.firsts.push((*c, first));
                }
                Err(e) => d.said.push((
                    true,
                    format!("scripts: {} does not join the scripts: {}", shown(name), shown_max(&e, MESSAGE_CHARS)),
                )),
            }
        }
        for c in &t.resync {
            let snapshot = self.host.snapshot(ClientId(*c)).iter().filter_map(|m| encode(m, &mut d)).collect();
            d.firsts.push((*c, snapshot));
        }
        for (id, name, key) in &t.connecting {
            let (host_id, v) = self.host.player_connecting(name, key, t.frame.now);
            match v {
                JoinVerdict::Accept => d.verdicts.push((*id, Verdict::Accept)),
                JoinVerdict::Refuse(why) => d.verdicts.push((*id, Verdict::Refuse(why))),
                JoinVerdict::Held => {
                    self.held.insert(host_id, (*id, Instant::now(), None));
                }
            }
        }
        for id in &t.forgotten {
            let Some(host_id) = self.held.iter().find(|(_, (w, ..))| w == id).map(|(h, _)| *h) else { continue };
            self.held.remove(&host_id);
            if let Some(waiting) = self.host.join_expired(host_id) {
                let what = if waiting.is_empty() {
                    String::new()
                } else {
                    format!(" (it waited for {})", shown_max(&waiting, MESSAGE_CHARS))
                };
                d.said.push((false, format!("scripts: join {id} was given up before the scripts decided{what}")));
            }
        }
        match self.host.server_frame(t.frame) {
            Ok(orders) => d.orders = orders,
            Err(e) => d.said.push((true, format!("scripts: the frame: {}", shown_max(&e, MESSAGE_CHARS)))),
        }
        self.decide_held(&mut d);
        self.drain(&mut d, &[]);
        self.take_output(&mut d);
        let m = self.host.monitor();
        d.refused_inbound = m.refused_inbound.saturating_sub(before);
        if t.monitor {
            d.monitor = Some((m, self.host.element_count()));
        }
        d.took = started.elapsed();
        d.ticked = true;
        d.packages = self.host.running().len();
        d.spent = self.host.take_spent();
        d
    }

    fn decide_held(&mut self, d: &mut Done) {
        let ids: Vec<u64> = self.held.keys().copied().collect();
        for host_id in ids {
            let Some((id, since, _)) = self.held.get(&host_id).cloned() else { continue };
            let verdict = match self.host.join_verdict(host_id) {
                Some(JoinVerdict::Accept) => Verdict::Accept,
                Some(JoinVerdict::Refuse(why)) => Verdict::Refuse(why),
                Some(JoinVerdict::Held) | None if since.elapsed() < DEFER_BOUND => {
                    let text = self.host.join_text(host_id).unwrap_or_default();
                    if let Some((_, _, last)) = self.held.get_mut(&host_id)
                        && last.as_ref() != Some(&text)
                    {
                        *last = Some(text.clone());
                        d.holding.push((id, text));
                    }
                    continue;
                }
                Some(JoinVerdict::Held) | None => {
                    let last = self.host.join_expired(host_id).unwrap_or_default();
                    let why = format!("the server's scripts did not decide within {} s", DEFER_BOUND.as_secs());
                    Verdict::Refuse(if last.is_empty() { why } else { format!("{why} ({last})") })
                }
            };
            self.held.remove(&host_id);
            d.verdicts.push((id, verdict));
        }
    }

    fn drain(&mut self, d: &mut Done, skip: &[u32]) {
        let drained = self.host.drain(usize::MAX);
        if drained.resync {
            for c in self.host.clients() {
                let snapshot = self.host.snapshot(c).iter().filter_map(|m| encode(m, d)).collect();
                d.firsts.push((c.0, snapshot));
            }
        }
        for m in drained.messages {
            let Destination::Clients(_) = &m.to else {
                d.said.push((true, "scripts: a message addressed to the server on the server".into()));
                continue;
            };
            let mut to: Vec<u32> = match &m.body {
                Body::LatentChunk { transfer, bytes, .. } => match self.latent.get_mut(transfer) {
                    Some((to, total, sent)) => {
                        *sent += bytes.len() as u64;
                        let list = to.clone();
                        if *sent >= *total {
                            self.latent.remove(transfer);
                        }
                        list
                    }
                    None => Vec::new(),
                },
                Body::LatentCancel { transfer } => self.latent.remove(transfer).map(|(to, ..)| to).unwrap_or_default(),
                _ => self.host.recipients(&m).iter().map(|c| c.0).collect(),
            };
            if let Body::LatentStart { transfer, total, .. } = &m.body {
                if self.latent.len() >= MAX_LATENT {
                    self.latent.pop_first();
                }
                self.latent.insert(*transfer, (to.clone(), u64::from(*total), 0));
            }
            if m.body.is_replication() {
                to.retain(|c| !skip.contains(c));
            }
            if to.is_empty() {
                continue;
            }
            let paced = match &m.body {
                Body::LatentChunk { bytes, .. } => bytes.len(),
                _ => 0,
            };
            if let Some(bytes) = encode(&m, d) {
                d.out.push((to, bytes, paced));
            }
        }
    }

    fn take_output(&mut self, d: &mut Done) {
        let mut more = 0usize;
        let names = self.host.running();
        let last = self.host.take_last_output();
        let outputs = names.into_iter().filter_map(|n| self.host.take_output(&n).map(|o| (n, o))).chain(last);
        for (name, o) in outputs {
            for line in o.lines {
                if d.said.len() < MAX_SAID {
                    d.said.push((false, format!("scripts: {}: {}", shown(&name), shown_max(&line, PRINT_CHARS))));
                } else {
                    more += 1;
                }
            }
        }
        if more > 0 {
            d.said.push((true, format!("scripts: {more} more lines of the scripts not logged")));
        }
    }
}

fn encode(m: &Message, d: &mut Done) -> Option<Shared> {
    match to_item(m.package.as_deref(), &m.body).encode() {
        Ok(b) => Some(b.into()),
        Err(e) => {
            d.unencodable += 1;
            if d.said.len() < MAX_SAID {
                d.said.push((
                    true,
                    format!("scripts: a message that does not encode: {}", shown_max(&e.to_string(), MESSAGE_CHARS)),
                ));
            }
            None
        }
    }
}
