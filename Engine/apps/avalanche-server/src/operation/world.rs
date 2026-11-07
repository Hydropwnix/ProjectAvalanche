use std::collections::HashMap;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use bevy::prelude::*;
use ssx_net::ServerStats;
use ssx_net::lightyear::prelude::server::ClientOf;
use ssx_net::lightyear::prelude::{Link, PingManager};
use ssx_net::server::{Instances, Kicked, LinkTraffic, PlayerSession};
use ssx_net_proto::Check;
use ssx_net_proto::dynamic::{DYNAMIC_VERSION, Dynamic, DynamicInstance};
use ssx_net_proto::instance::Level;

use super::commands::{Caller, OPERATION_LOG, OperationRes};
use super::http::{OperationHttp, RconQueue};
use super::metrics::{JITTER_BOUNDS, RATE_BOUNDS, RTT_BOUNDS};
use super::store::unix_now;
use crate::systems::ServerFacts;

#[derive(Resource, Clone)]
pub struct OperationHttpRes(pub Arc<OperationHttp>);

#[derive(Resource)]
pub struct RconQueueRes(pub std::sync::Mutex<RconQueue>);

const COMMANDS_PER_TICK: usize = 16;
const _: () = assert!(COMMANDS_PER_TICK == ssx_script::host::server::rights::MAX_COMMANDS_PER_FRAME);

pub fn run_rcon(world: &mut World) {
    let tasks: Vec<_> = {
        let Some(queue) = world.get_resource::<RconQueueRes>() else { return };
        let mut queue = queue.0.lock().unwrap_or_else(PoisonError::into_inner);
        std::iter::from_fn(|| queue.0.try_recv().ok()).take(COMMANDS_PER_TICK).collect()
    };
    for task in tasks {
        let id = task.operator.id().short();
        let task = match (task.command, world.get_resource::<crate::scripts::ServerScripts>()) {
            (crate::console::Command::Script(line), Some(s)) => {
                let shown = ssx_core::text::quoted_max(&task.line, 200).to_string();
                match crate::scripts::console(s, &line, Some(task.reply)) {
                    Ok(()) => info!(target: OPERATION_LOG, "rcon {id}: {shown}: handed to the scripts"),
                    Err(e) => warn!(target: OPERATION_LOG, "rcon {id}: {shown}: not run: {e}"),
                }
                continue;
            }
            (command, _) => super::rcon::RconTask { command, ..task },
        };
        let caller = Caller::Operator(task.operator);
        let (ok, lines) = crate::systems::run_command_for(world, task.command, &caller);
        info!(
            target: OPERATION_LOG,
            "rcon {id}: {}: {} ({} lines)",
            ssx_core::text::quoted_max(&task.line, 200),
            if ok { "done" } else { "not done" },
            lines.len()
        );
        let _ = task.reply.send((ok, lines));
    }
}

#[derive(Default)]
pub struct LastPublish {
    at: Option<Duration>,
    traffic: HashMap<Entity, (u64, u64)>,
}

type Measured<'a> = (Entity, &'a Link, &'a PingManager, &'a LinkTraffic);
type Riders = (With<ClientOf>, With<PlayerSession>, Without<Kicked>);
type Sources<'w> = (Option<Res<'w, Instances>>, Res<'w, ServerStats>, Res<'w, ServerFacts>, Res<'w, Time<Real>>);

pub fn publish(
    ops: Option<Res<OperationHttpRes>>,
    (instances, stats, facts, time): Sources,
    links: Query<Measured, Riders>,
    mut last: Local<LastPublish>,
) {
    let Some(ops) = ops else { return };
    let now = time.elapsed();
    let period = match last.at {
        Some(at) if now.saturating_sub(at) < Duration::from_secs(1) => return,
        Some(at) => now.saturating_sub(at).as_secs_f64(),
        None => 0.0,
    };
    last.at = Some(now);
    let list = instances.as_ref().map(|i| i.list.as_slice()).unwrap_or_default();
    let rows: Vec<DynamicInstance> = list
        .iter()
        .map(|s| {
            let i = &s.instance;
            DynamicInstance {
                id: i.id(),
                mode: i.spec().mode.name().into(),
                drop: i.spec().drop().unwrap_or_default().to_string(),
                map: match &i.spec().level {
                    Level::Map { id, .. } => Some(id.clone()),
                    Level::Drop(_) => None,
                },
                gamemode: match &i.spec().level {
                    Level::Map { gamemode, .. } => gamemode.clone(),
                    Level::Drop(_) => None,
                },
                state: i.state().name().into(),
                round: i.round(),
                players: u32::try_from(i.len()).unwrap_or(u32::MAX),
            }
        })
        .collect();
    let players = rows.iter().map(|r| r.players).fold(0u32, u32::saturating_add);
    let dynamic = Dynamic {
        dynamic_version: DYNAMIC_VERSION,
        players,
        max_players: u32::from(facts.max_players),
        instances: rows,
    };
    *ops.0.dynamic.lock().unwrap_or_else(PoisonError::into_inner) = dynamic.validate().is_ok().then_some(dynamic);
    let mut snap = ops.0.snapshot.lock().unwrap_or_else(PoisonError::into_inner);
    snap.instances = list
        .iter()
        .map(|s| {
            let i = &s.instance;
            (i.id(), i.spec().mode.name(), u32::try_from(i.len()).unwrap_or(u32::MAX), i.round())
        })
        .collect();
    let mut seen = HashMap::new();
    for (entity, link, ping, traffic) in &links {
        let t = traffic.0;
        if let Some((sent, received)) = last.traffic.get(&entity)
            && period > 0.0
        {
            snap.sent_rate.observe(&RATE_BOUNDS, t.sent_bytes.saturating_sub(*sent) as f64 / period);
            snap.received_rate.observe(&RATE_BOUNDS, t.recv_bytes.saturating_sub(*received) as f64 / period);
        }
        if ping.pongs_recv > 0 {
            snap.rtt.observe(&RTT_BOUNDS, link.stats.rtt.as_secs_f64());
            snap.jitter.observe(&JITTER_BOUNDS, link.stats.jitter.as_secs_f64());
        }
        seen.insert(entity, (t.sent_bytes, t.recv_bytes));
    }
    last.traffic = seen;
    snap.validation = Check::ALL
        .iter()
        .map(|c| {
            let o = &stats.validation[c.index()];
            (c.name(), [o.made, o.found, o.warned, o.invalidated, o.kicked])
        })
        .collect();
    (snap.chat_received, snap.chat_sent, snap.chat_dropped) =
        (stats.chat_received, stats.chat_sent, stats.chat_dropped);
    (snap.chat_floods, snap.chat_muted) = (stats.chat_floods, stats.chat_muted);
    (snap.sessions, snap.kicked) = (stats.sessions, stats.kicked);
    (snap.sent_bytes, snap.received_bytes) = (stats.traffic.sent_bytes, stats.traffic.recv_bytes);
}

const SWEEP: Duration = Duration::from_secs(60);

pub fn sweep(ops: Option<Res<OperationRes>>, time: Res<Time<Real>>, mut last: Local<Option<Duration>>) {
    let Some(ops) = ops else { return };
    let now = time.elapsed();
    if last.is_some_and(|at| now.saturating_sub(at) < SWEEP) {
        return;
    }
    *last = Some(now);
    match ops.0.expire(unix_now()) {
        Ok((bans, mutes)) => {
            for b in bans {
                info!(target: OPERATION_LOG, "the ban of {} ended ({})", b.target, b.until.describe());
            }
            for m in mutes {
                let who = ssx_net_ops::Target::Player(m.player);
                info!(target: OPERATION_LOG, "the mute of {who} ended ({})", m.until.describe());
            }
        }
        Err(e) => warn!(target: OPERATION_LOG, "the passed bans were not removed: {}", ssx_core::text::message(&e)),
    }
}
