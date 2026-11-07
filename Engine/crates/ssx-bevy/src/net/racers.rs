use std::collections::BTreeMap;
use std::time::Instant;

use bevy::prelude::*;
use ssx_assets::ghosts::GhostPose;
use ssx_net::client::RemotePoseReceived;
use ssx_net::lightyear::prelude::LocalTimeline;
use ssx_net::{ClientRoster, ServerClock, ViewClock, ViewMap};
use ssx_net_proto::PoseFlag;
use ssx_physics::session::bots::live::{SampleFlags, Stamp};

use super::session::NetSession;
use crate::ride::Ride;
use crate::ride::bots::Bots;
use crate::ride::bots::live::LiveLook;

fn now(view: &ViewClock, clock: &ServerClock, timeline: &LocalTimeline, fixed: &Time<Fixed>) -> Option<f64> {
    view.of(clock.server_now(timeline, fixed)?)
}

#[derive(Default)]
pub struct Churn {
    joined: u32,
    left: u32,
    since: Option<Instant>,
    frames: u32,
    frame_sum_ms: f64,
    frame_max_ms: f64,
    traffic: ssx_net::traffic::Traffic,
}

#[derive(Resource, Default)]
pub struct ViewMaps(BTreeMap<u16, (Entity, ViewMap)>);

pub fn sync(
    session: Res<NetSession>,
    roster: Res<ClientRoster>,
    (ride, bots, mut maps): (Option<ResMut<Ride>>, Option<ResMut<Bots>>, ResMut<ViewMaps>),
    (real, traffic): (Res<Time<Real>>, Res<ssx_net::traffic::ClientTraffic>),
    mut churn: Local<Churn>,
) {
    let (Some(mut ride), Some(mut bots)) = (ride, bots) else { return };
    let frame_ms = real.delta_secs_f64() * 1000.0;
    churn.frames += 1;
    churn.frame_sum_ms += frame_ms;
    churn.frame_max_ms = churn.frame_max_ms.max(frame_ms);
    let Some(race) = ride.session.bots.as_mut() else { return };
    let wanted: BTreeMap<u16, LiveLook> = if session.is_ended() {
        BTreeMap::new()
    } else {
        roster
            .others()
            .filter(|(_, p)| p.status != Some(ssx_net_proto::PlayerStatus::Spectating))
            .filter_map(|(id, p)| {
                let a = p.appearance?;
                let look =
                    LiveLook { name: p.name.clone(), character: a.character, gear: a.gear, hologram: session.hologram };
                Some((id.0, look))
            })
            .collect()
    };
    let gone: Vec<u16> = race.live.iter().map(|r| r.id).filter(|id| !wanted.contains_key(id)).collect();
    maps.0.retain(|id, _| wanted.contains_key(id));
    for id in gone {
        race.remove_live(id);
        bots.live.forget(id);
        churn.left += 1;
    }
    for (id, look) in wanted {
        if !race.live.iter().any(|r| r.id == id) {
            race.add_live(id);
            churn.joined += 1;
        }
        bots.live.want(id, look);
    }
    let since = *churn.since.get_or_insert_with(Instant::now);
    let elapsed = since.elapsed().as_secs_f32();
    let changed = churn.joined > 0 || churn.left > 0;
    if (elapsed >= 1.0 && changed) || elapsed >= 10.0 {
        let (t, before, secs) = (traffic.0, churn.traffic, f64::from(elapsed));
        info!(
            "network: {} riders joined, {} left; {} live racers ({} drawn); frames over {elapsed:.1} s: mean {:.2} \
             ms, max {:.2} ms; UDP payload down {:.0} B/s ({:.1} datagrams/s), up {:.0} B/s ({:.1} datagrams/s)",
            churn.joined,
            churn.left,
            race.live.len(),
            bots.live.drawn(),
            churn.frame_sum_ms / f64::from(churn.frames.max(1)),
            churn.frame_max_ms,
            (t.recv_bytes - before.recv_bytes) as f64 / secs,
            (t.recv_datagrams - before.recv_datagrams) as f64 / secs,
            (t.sent_bytes - before.sent_bytes) as f64 / secs,
            (t.sent_datagrams - before.sent_datagrams) as f64 / secs,
        );
        *churn = Churn { since: Some(Instant::now()), traffic: t, ..Churn::default() };
    }
}

pub fn samples(
    mut received: MessageReader<RemotePoseReceived>,
    (ride, roster, mut maps): (Option<ResMut<Ride>>, Res<ClientRoster>, ResMut<ViewMaps>),
    (clock, view, timeline, fixed): (Res<ServerClock>, Res<ViewClock>, Res<LocalTimeline>, Res<Time<Fixed>>),
) {
    let (Some(mut ride), Some(t)) = (ride, now(&view, &clock, &timeline, &fixed)) else {
        received.clear();
        return;
    };
    let Some(race) = ride.session.bots.as_mut() else {
        received.clear();
        return;
    };
    for r in received.read() {
        let Some(racer) = race.live_mut(r.rider.0) else { continue };
        let f = r.pose.flags();
        let flags = SampleFlags {
            teleport: f.contains(PoseFlag::Teleport),
            respawn: f.contains(PoseFlag::Respawn),
            rewind: f.contains(PoseFlag::Rewind),
            epoch: f.warp_epoch(),
        };
        let Some(player) = roster.players.get(&r.rider) else { continue };
        let map = maps.0.entry(r.rider.0).or_insert_with(|| (player.entity, ViewMap::default()));
        if map.0 != player.entity {
            *map = (player.entity, ViewMap::default());
        }
        let Some(tick) = map.1.place(&view, r.pose.view_time(), r.pose.view_lead(), t) else { continue };
        let at = Stamp { tick, execution: r.pose.execution() };
        racer.pose.push(at, flags, GhostPose::of(r.pose.block()), t);
    }
}

pub fn step_live(
    ride: Option<ResMut<Ride>>,
    (clock, view): (Res<ServerClock>, Res<ViewClock>),
    timeline: Res<LocalTimeline>,
) {
    let Some(t) = clock.server_time(f64::from(timeline.tick().0)).and_then(|t| view.of(t)) else { return };
    let Some(mut ride) = ride else { return };
    let player = ride.state.traj;
    ride.session.update_live_racers(t, player);
}

pub fn clock(
    bots: Option<ResMut<Bots>>,
    (clock, view): (Res<ServerClock>, Res<ViewClock>),
    timeline: Res<LocalTimeline>,
    fixed: Res<Time<Fixed>>,
) {
    if let (Some(mut bots), Some(t)) = (bots, now(&view, &clock, &timeline, &fixed)) {
        bots.live.now = t;
    }
}
