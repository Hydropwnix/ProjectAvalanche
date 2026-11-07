use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use bevy::prelude::*;
use ssx_net::lightyear::prelude::LocalTimeline;
use ssx_net::lightyear::prelude::server::ClientOf;
use ssx_net::roster::PlayerPing;
use ssx_net::server::{
    Instances, KickReason, Kicked, PlacedIn, PlayerEntity, PlayerSession, RiderLink, RoundLog, ScriptInbox, ScriptLink,
    ScriptReports, ScriptSessions, Shed, TOTAL_QUEUE_BYTES, leave, shed,
};
use ssx_net::tick_duration;
use ssx_script::host::server::{ServerEvent, ServerFrame};

use super::ServerScripts;
use super::frame::{LinkView, instances, players, report_event, round_events};
use super::joins::HeldJoins;
use super::rights::{Identities, OperationOrders};
use super::thread::{Done, Job, TickJob, Verdict};
use crate::operation::world::OperationHttpRes;

#[derive(Resource, Default, Debug)]
pub struct ScriptStats {
    pub jobs: u64,
    pub busy_ticks: u64,
    pub took_last: Duration,
    pub took_max: Duration,
    pub refused_inbound: u64,
    pub unencodable: u64,
    pub resyncs: u64,
    pub backlog_ends: u64,
    pub rounds_ended: u64,
    pub budget_resyncs: u64,
    pub budget_ends: u64,
    pub queues_peak: usize,
}

impl ScriptStats {
    pub fn line(&self) -> String {
        format!(
            "scripts: {} jobs ({} ticks the thread was busy), a job's time {:.2} ms (longest {:.2} ms); {} client \
             messages refused, {} not encodable; {} resyncs, {} players sent away for their backlog; {} rounds ended; \
             the clients' queues held at most {} bytes together (bound {}): {} resynced and {} sent away by the bound",
            self.jobs,
            self.busy_ticks,
            self.took_last.as_secs_f64() * 1e3,
            self.took_max.as_secs_f64() * 1e3,
            self.refused_inbound,
            self.unencodable,
            self.resyncs,
            self.backlog_ends,
            self.rounds_ended,
            self.queues_peak,
            ssx_net::server::TOTAL_QUEUE_BYTES,
            self.budget_resyncs,
            self.budget_ends,
        )
    }
}

#[derive(Resource, Default)]
pub struct Pending {
    job: TickJob,
    links: BTreeMap<u32, Entity>,
    resync: BTreeSet<u32>,
    last_monitor: Option<Duration>,
    drops: BTreeMap<u32, u64>,
    left_unsent: Vec<u32>,
    left_in_flight: Vec<u32>,
    shutdown_in_flight: bool,
}

impl Pending {
    pub fn link(&self, client: u32) -> Option<Entity> {
        self.links.get(&client).copied()
    }
}

#[derive(Resource, Default, Debug)]
pub struct ShutdownHeard {
    pub answered: bool,
}

pub fn shutdown_begun(begun: On<crate::operation::stop::ShutdownBegun>, mut pending: ResMut<Pending>) {
    let round_end_secs = begun.event().round_end_secs;
    pending.job.frame.events.push(ServerEvent::Shutdown { round_end_secs });
}

type Live<'a> =
    (Entity, &'a ScriptLink, &'a PlayerSession, Option<&'a RiderLink>, Option<&'a PlacedIn>, Option<&'a PlayerEntity>);

type Net<'a> = (ResMut<'a, ScriptInbox>, ResMut<'a, ScriptSessions>, ResMut<'a, ScriptReports>);
type Clocks<'a> = (Res<'a, LocalTimeline>, Res<'a, Time<Real>>);

pub fn step(
    (scripts, mut pending, mut stats): (Option<ResMut<ServerScripts>>, ResMut<Pending>, ResMut<ScriptStats>),
    (mut inbox, mut sessions, mut reports): Net,
    (log, list): (Res<RoundLog>, Option<Res<Instances>>),
    (timeline, time): Clocks,
    (mut held, identities): (ResMut<HeldJoins>, Res<Identities>),
    live: Query<Live, (With<ClientOf>, Without<Kicked>)>,
    pings: Query<&PlayerPing>,
) {
    let p = &mut *pending;
    for (link, client, name) in sessions.joined.drain(..) {
        p.links.insert(client, link);
        p.job.joined.push((client, name));
        if let Ok((_, _, session, ..)) = live.get(link) {
            identities.insert(client, session.id.0, session.address);
        }
    }
    for (client, why) in sessions.left.drain(..) {
        p.links.remove(&client);
        p.resync.remove(&client);
        p.drops.remove(&client);
        if let Some(k) = p.job.joined.iter().position(|(c, _)| *c == client) {
            p.job.joined.remove(k);
            p.job.inbound.retain(|(c, _)| *c != client);
            identities.forget(client);
        } else {
            p.job.left.push((client, why));
            p.left_unsent.push(client);
        }
    }
    p.job.inbound.append(&mut inbox.0);
    let by_link: BTreeMap<Entity, u32> = p.links.iter().map(|(c, e)| (*e, *c)).collect();
    for r in reports.events.drain(..) {
        if let Some(c) = by_link.get(&r.link) {
            report_event(&r, *c, &mut p.job.frame.events);
        }
    }
    round_events(log.events.iter(), &mut p.job.frame.events);
    let Some(mut scripts) = scripts else {
        p.job = TickJob::default();
        return;
    };
    if scripts.in_flight {
        stats.busy_ticks += 1;
        return;
    }
    let now = time.elapsed();
    let tick = timeline.tick().0;
    let tick_ms = tick_duration().as_secs_f64() * 1e3;
    let empty = Instances::default();
    let list = list.as_deref().unwrap_or(&empty);
    let mut names = BTreeMap::new();
    let mut views = Vec::new();
    for (_, link, session, rider_link, placed, player) in &live {
        let rider = rider_link.map(|r| r.rider().0);
        if let (Some(i), Some(r)) = (placed, rider) {
            names.insert((i.0, r), session.name.clone());
        }
        let seen = p.drops.entry(link.client).or_default();
        if link.dropped > *seen {
            p.job.flooded.push((link.client, link.dropped - *seen));
            *seen = link.dropped;
        }
        views.push(LinkView {
            client: link.client,
            name: session.name.clone(),
            instance: placed.map(|i| i.0),
            rider,
            position: rider_link.and_then(RiderLink::position),
            ping_ms: player.and_then(|e| pings.get(e.0).ok()).map(|p| p.millis()),
        });
    }
    p.drops.retain(|c, _| views.iter().any(|v| v.client == *c));
    let events = std::mem::take(&mut p.job.frame.events);
    p.job.frame =
        ServerFrame { now, instances: instances(list, &names, tick, tick_ms), players: players(list, views), events };
    p.job.resync = std::mem::take(&mut p.resync).into_iter().collect();
    p.job.connecting = std::mem::take(&mut held.to_ask);
    p.job.forgotten = std::mem::take(&mut held.forgotten);
    p.job.monitor = p.last_monitor.is_none_or(|t| now.saturating_sub(t) >= Duration::from_secs(1));
    if p.job.monitor {
        p.last_monitor = Some(now);
    }
    let job = std::mem::take(&mut p.job);
    let shutdown = job.frame.events.iter().any(|e| matches!(e, ServerEvent::Shutdown { .. }));
    let left = std::mem::take(&mut p.left_unsent);
    match scripts.handle.send(Job::Tick(Box::new(job))) {
        Ok(()) => {
            scripts.in_flight = true;
            stats.jobs += 1;
            p.left_in_flight = left;
            p.shutdown_in_flight = shutdown;
        }
        Err(e) => {
            left.into_iter().for_each(|c| identities.forget(c));
            warn!("scripts: {e}");
        }
    }
}

pub fn collect(
    (scripts, mut pending, mut stats): (Option<ResMut<ServerScripts>>, ResMut<Pending>, ResMut<ScriptStats>),
    mut links: Query<&mut ScriptLink, (With<ClientOf>, Without<Kicked>)>,
    mut list: Option<ResMut<Instances>>,
    (timeline, time): Clocks,
    mut held: ResMut<HeldJoins>,
    (mut orders, mut heard, ops): (ResMut<OperationOrders>, ResMut<ShutdownHeard>, Option<Res<OperationHttpRes>>),
    mut commands: Commands,
) {
    let now = time.elapsed();
    held.expire(now);
    let Some(mut scripts) = scripts else { return };
    let answers: Vec<Done> = match scripts.done.lock() {
        Ok(rx) => rx.try_iter().collect(),
        Err(_) => return,
    };
    for mut d in answers {
        if d.ticked {
            scripts.in_flight = false;
            held.asking = d.packages > 0;
            orders.forget.append(&mut pending.left_in_flight);
            heard.answered |= std::mem::take(&mut pending.shutdown_in_flight);
        }
        orders.orders.append(&mut d.orders.operation);
        for (resource, spent) in std::mem::take(&mut d.spent) {
            if let Some(ops) = &ops
                && let Err(e) = ops.0.script_time(&resource, spent)
            {
                warn!("scripts: /metrics: {}", ssx_core::text::shown(&e));
            }
        }
        apply(&mut pending, &mut links, &mut stats, &mut commands, now, &d);
        for o in &d.orders.end_round {
            let tick = timeline.tick().0;
            let instance = list.as_deref_mut().and_then(|l| l.by_id_mut(o.instance));
            let ended = instance.is_some_and(|s| s.instance.end_round(o.round, tick));
            if ended {
                stats.rounds_ended += 1;
                info!(
                    "instance {} round {}: ended by the scripts: {}",
                    o.instance,
                    o.round,
                    ssx_core::text::shown(&o.reason)
                );
            }
        }
        for (id, text) in &d.holding {
            held.hold(*id, text, now);
        }
        for (id, v) in d.verdicts {
            held.decide(
                id,
                match v {
                    Verdict::Accept => Ok(()),
                    Verdict::Refuse(why) => Err(why),
                },
            );
        }
        for (problem, line) in d.said {
            if problem { warn!("{line}") } else { info!("{line}") }
        }
        if let Some(m) = d.monitor {
            scripts.monitor = Some(m);
        }
    }
}

fn apply(
    pending: &mut Pending,
    links: &mut Query<&mut ScriptLink, (With<ClientOf>, Without<Kicked>)>,
    stats: &mut ScriptStats,
    commands: &mut Commands,
    now: Duration,
    d: &Done,
) {
    if d.ticked {
        stats.took_last = d.took;
        stats.took_max = stats.took_max.max(d.took);
        stats.refused_inbound += d.refused_inbound;
        stats.unencodable += d.unencodable;
    }
    let mut all: Vec<Mut<ScriptLink>> = links.iter_mut().collect();
    let mut refs: Vec<(u32, &mut ScriptLink)> = all.iter_mut().map(|l| (l.client, &mut **l)).collect();
    refs.sort_by_key(|(c, _)| *c);
    for (client, why) in queue_answer(d, &mut refs, &mut pending.resync, stats, now) {
        if let Some(&entity) = pending.links.get(&client) {
            leave(commands, entity, KickReason::ScriptBacklog, why, now);
        }
    }
}

pub(super) fn queue_answer(
    d: &Done,
    links: &mut [(u32, &mut ScriptLink)],
    resync: &mut BTreeSet<u32>,
    stats: &mut ScriptStats,
    now: Duration,
) -> Vec<(u32, &'static str)> {
    let mut ends: Vec<(u32, &'static str)> = Vec::new();
    let mut gone: BTreeSet<u32> = BTreeSet::new();
    let find = |links: &[(u32, &mut ScriptLink)], client: u32| links.binary_search_by_key(&client, |(c, _)| *c).ok();
    for (client, items) in &d.firsts {
        let Some(i) = find(links, *client) else { continue };
        resync.remove(client);
        let link = &mut links[i].1;
        if link.queue_snapshot(items, now).is_err() {
            link.clear();
            stats.backlog_ends += 1;
            ends.push((*client, "the server's scripts' state does not fit this connection"));
            gone.insert(*client);
        }
    }
    let mut total: usize = links.iter().map(|(_, l)| l.held()).sum();
    if total > TOTAL_QUEUE_BYTES {
        total = keep_the_bound(links, resync, stats, &mut gone, &mut ends);
    }
    for (to, bytes, paced) in &d.out {
        for client in to {
            if gone.contains(client) || resync.contains(client) {
                continue;
            }
            let Some(i) = find(links, *client) else { continue };
            let link = &mut links[i].1;
            if link.queue(Arc::clone(bytes), *paced, now).is_ok() {
                total += bytes.len();
            } else {
                total -= link.held();
                link.clear();
                total += link.held();
                stats.resyncs += 1;
                resync.insert(*client);
            }
        }
        if total > TOTAL_QUEUE_BYTES {
            total = keep_the_bound(links, resync, stats, &mut gone, &mut ends);
        }
    }
    stats.queues_peak = stats.queues_peak.max(total);
    ends
}

fn keep_the_bound(
    links: &mut [(u32, &mut ScriptLink)],
    resync: &mut BTreeSet<u32>,
    stats: &mut ScriptStats,
    gone: &mut BTreeSet<u32>,
    ends: &mut Vec<(u32, &'static str)>,
) -> usize {
    for (client, what) in shed(links, TOTAL_QUEUE_BYTES) {
        match what {
            Shed::Resync => {
                stats.budget_resyncs += 1;
                resync.insert(client);
            }
            Shed::SendAway => {
                stats.budget_ends += 1;
                gone.insert(client);
                ends.push((client, "the server's scripts' queues are full and this connection waited longest"));
            }
        }
    }
    links.iter().map(|(_, l)| l.held()).sum()
}
