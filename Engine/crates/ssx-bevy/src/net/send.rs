use bevy::prelude::*;
use ssx_net::lightyear::prelude::LocalTimeline;
use ssx_net::{ControlOutbox, PoseOutbox, ServerClock, ViewClock};
use ssx_net_proto::{Appearance, ClientControl, NetPose, PoseFlag, PoseFlags};

use super::session::NetSession;
use crate::ride::Ride;
use crate::shell::live::{LiveLevel, LiveSession};

const SAMPLE_TICKS: u32 = 2;

const SLOT_HEAD_LAMP: usize = 6;
const SLOT_ICE_AXE: usize = 2;

#[derive(Resource, Default)]
pub struct Sent {
    appearance: bool,
    loaded: Option<u16>,
    ready: Option<u16>,
    acked: Option<u16>,
    left: bool,
    sample_tick: Option<u32>,
    refused: bool,
    implausible_said: bool,
}

fn appearance(ride: &Ride) -> Option<Appearance> {
    let s = &ride.world().survival;
    let mut gear = 0;
    if s.item_enabled(SLOT_HEAD_LAMP) {
        gear |= Appearance::HEAD_LAMP;
    }
    if s.item_enabled(SLOT_ICE_AXE) {
        gear |= Appearance::ICE_AXE;
    }
    Appearance::new(ride.wire_key(), gear).ok()
}

pub fn control(
    session: Res<NetSession>,
    live: Res<LiveSession>,
    ride: Option<Res<Ride>>,
    (mut sent, mut rounds): (ResMut<Sent>, ResMut<super::round::Rounds>),
    mut outbox: ResMut<ControlOutbox>,
) {
    if !session.connected() || live.ending() {
        return;
    }
    if matches!(live.level, LiveLevel::Away | LiveLevel::Next) || live.relaunch.is_some() {
        if sent.appearance && !sent.left {
            info!("network: the level is left: level left");
            outbox.0.push(ClientControl::LevelLeft);
            *sent =
                Sent { left: true, refused: sent.refused, implausible_said: sent.implausible_said, ..Sent::default() };
        }
        return;
    }
    let Some(ride) = ride else { return };
    if !sent.appearance {
        let Some(a) = appearance(&ride) else { return };
        info!("network: the look sent: {} ({:#010x}), gear {:#x}", ride.character(), a.character, a.gear);
        outbox.0.push(ClientControl::Appearance(a));
        sent.appearance = true;
        sent.left = false;
    }
    let rounds_mode = live.rules.is_some_and(|r| r.rounds);
    let round = if rounds_mode { rounds.round } else { Some(0) };
    let Some(round) = round else { return };
    let at_start = !rounds_mode || super::round::waiting(&ride);
    if sent.loaded != Some(round) && live.level == LiveLevel::Up && at_start {
        info!("network: the level is up: loaded (round {round})");
        outbox.0.push(ClientControl::Loaded { round });
        sent.loaded = Some(round);
    }
    if rounds_mode
        && sent.loaded == Some(round)
        && sent.acked != Some(round)
        && rounds.start.is_some()
        && !rounds.spectator
    {
        info!("network: the round {round}'s start acknowledged");
        outbox.0.push(ClientControl::StartAck { round });
        sent.acked = Some(round);
    }
    let started = !rounds_mode || (ride.go_step.is_some() && rounds.go_given.is_some());
    if sent.loaded == Some(round) && sent.ready != Some(round) && ride.state.start.is_none() && started {
        let (Some(sample), Some(second)) = (rounds.handover.clone(), rounds.second.clone()) else { return };
        info!(
            "network: the intro is done: ready (round {round}, executions {} and {})",
            sample.execution(),
            second.execution()
        );
        outbox.0.push(ClientControl::Ready { round, sample: Box::new(sample), second: Box::new(second) });
        sent.ready = Some(round);
    }
    if sent.ready == Some(round) && !rounds.pending.is_empty() {
        outbox.0.extend(rounds.pending.drain(..).map(|r| ClientControl::Report(Box::new(r))));
    }
}

pub fn sample(
    session: Res<NetSession>,
    ride: Option<ResMut<Ride>>,
    (timeline, clock, view): (Res<LocalTimeline>, Res<ServerClock>, Res<ViewClock>),
    rounds: Res<super::round::Rounds>,
    mut sent: ResMut<Sent>,
    mut outbox: ResMut<PoseOutbox>,
) {
    if !session.connected() || !sent.appearance {
        return;
    }
    let Some(mut ride) = ride else { return };
    let tick = timeline.tick().0;
    if sent.sample_tick.is_some_and(|t| tick.wrapping_sub(t) < SAMPLE_TICKS) {
        return;
    }
    let Some(time) = clock.server_time(f64::from(tick)).filter(|t| *t >= 0.0) else { return };
    let Some(view_time) = view.of(time) else { return };
    let Some(blocks) = ride.pose_blocks.as_mut() else { return };
    let Some(b) = blocks.take() else { return };
    let mut flags = PoseFlags::default().with_warp_epoch(b.epoch);
    for (on, flag) in [
        (b.teleport, PoseFlag::Teleport),
        (b.rewinding, PoseFlag::Rewind),
        (b.respawn, PoseFlag::Respawn),
        (b.impact, PoseFlag::ImpactLatched),
    ] {
        if on {
            flags = flags.with(flag);
        }
    }
    if outbox.implausible > 0 && !std::mem::replace(&mut sent.implausible_said, true) {
        info!(
            "network: samples the server's rules refuse are not sent (the other players do not see this rider until \
             it is within them): {}",
            outbox.implausible_why.as_deref().unwrap_or("?")
        );
    }
    let execution = rounds.executions.index(b.step, rounds.mode.is_some());
    match NetPose::at(time, flags, b.block).and_then(|p| p.with_execution(execution)?.with_view_time(view_time)) {
        Ok(pose) => {
            outbox.next = Some(pose);
            sent.sample_tick = Some(tick);
        }
        Err(e) => {
            if !std::mem::replace(&mut sent.refused, true) {
                error!("network: the pose block does not make a sample: {e}");
            }
        }
    }
}
