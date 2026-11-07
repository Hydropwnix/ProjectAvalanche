use core::time::Duration;

use bevy_time::{Fixed, Time};
use lightyear::prelude::LocalTimeline;
use ssx_gameplay::round::CycleRules;
use ssx_net_proto::{
    Appearance, ClientControl, GhostStyle, InstanceSpec, InstanceState, Mode, NetPose, PoseFlags, ProgressReport,
    ReportKind, Rewinds, StandingRow,
};

use super::{Conditions, Io, Peers, ServerKind};
use crate::client::{ClientRoster, ControlOutbox, ServerClock, ServerSaid, local_time};
use crate::server::{InstanceConfig, InstanceSetup};

pub const ROTATION: [&str; 2] = ["exp_alps_vorassay_drop_3", "exp_rockies_peak_drop_1"];

pub fn round_instance(mode: Mode, rules: CycleRules) -> Result<InstanceConfig, String> {
    let spec =
        InstanceSpec::new(ROTATION[0], mode, Rewinds::On, false, GhostStyle::Character).map_err(|e| e.to_string())?;
    let rotation = ROTATION
        .iter()
        .map(|d| ssx_net_proto::instance::Level::drop(d).map_err(|e| e.to_string()))
        .collect::<Result<_, _>>()?;
    let setup = InstanceSetup { spec, capacity: crate::limits::MAX_CLIENTS, rounds: Some((rules, rotation)) };
    Ok(InstanceConfig { instances: vec![setup] })
}

pub fn riders(n: usize, mode: Mode, rules: CycleRules, conditions: Option<Conditions>) -> Result<Peers, String> {
    riders_with(n, mode, rules, conditions, |_| {})
}

pub fn riders_with(
    n: usize,
    mode: Mode,
    rules: CycleRules,
    conditions: Option<Conditions>,
    setup: impl FnOnce(&mut bevy_app::App),
) -> Result<Peers, String> {
    let config = round_instance(mode, rules)?;
    let frame = super::hostile::FRAME;
    let mut peers = Peers::with_server_setup(ServerKind::Guarded, Io::Udp, conditions, frame, config, setup)?;
    for k in 0..n {
        peers.add_client(k as u64)?;
        let batch = k + 1;
        if (batch.is_multiple_of(crate::limits::MAX_TICKETS_PER_ADDRESS) || batch == n)
            && !peers.steps_for(Duration::from_secs(20), |p| (0..batch).all(|c| p.connected(c)))
        {
            return Err(format!("the first {batch} riders did not connect"));
        }
    }
    let all = |p: &mut Peers| (0..n).all(|c| p.connected(c) && p.synced(c) && p.knows(c, n));
    if !peers.steps_for(Duration::from_secs(20), all) {
        return Err(format!("the {n} riders did not connect"));
    }
    Ok(peers)
}

pub fn look() -> Appearance {
    Appearance { character: super::hostile::TEST_CHARACTER, gear: 0 }
}

pub fn send(peers: &mut Peers, c: usize, messages: impl IntoIterator<Item = ClientControl>) {
    peers.clients[c].world_mut().resource_mut::<ControlOutbox>().0.extend(messages);
}

pub fn server_time(peers: &Peers, c: usize) -> Option<f64> {
    let world = peers.clients[c].world();
    let now = local_time(world.resource::<LocalTimeline>(), world.resource::<Time<Fixed>>());
    world.resource::<ServerClock>().server_time(now)
}

pub fn start(peers: &Peers, c: usize) -> Option<(u16, u32)> {
    peers.clients[c].world().resource::<ServerSaid>().start
}

pub fn instance(peers: &Peers, c: usize) -> Option<(InstanceState, u16, String)> {
    let roster = peers.clients[c].world().resource::<ClientRoster>();
    roster.instance.as_ref().map(|i| (i.state, i.round, i.spec.level.name().to_string()))
}

pub fn standings(peers: &Peers, c: usize) -> Option<(u16, Vec<StandingRow>)> {
    peers.clients[c].world().resource::<ClientRoster>().standings.clone()
}

pub fn rider(peers: &Peers, c: usize) -> Option<u16> {
    peers.clients[c].world().resource::<ClientRoster>().own.map(|(r, _)| r.0)
}

pub fn execution(peers: &Peers, c: usize) -> u32 {
    match (server_time(peers, c), start(peers, c)) {
        (Some(now), Some((_, t))) => (now - f64::from(t)).max(0.0).floor() as u32,
        _ => 0,
    }
}

pub fn report(
    peers: &Peers,
    c: usize,
    round: u16,
    kind: ReportKind,
    split: Option<u8>,
    clock_ms: u32,
) -> ProgressReport {
    let time = server_time(peers, c).unwrap_or(0.0).max(0.0);
    let sample = NetPose::at(time, PoseFlags::default(), super::sample_block(c as u64))
        .and_then(|p| p.with_execution(execution(peers, c).min(ssx_net_proto::MAX_EXECUTION)))
        .unwrap_or_else(|e| unreachable!("a test block at a server time: {e}"));
    ProgressReport {
        instance: crate::server::INSTANCE_ID,
        round,
        kind,
        split,
        clock_ms,
        points: 0,
        descent_mm: 0,
        rewinds: 0,
        loops: 0,
        sample,
    }
}

pub fn clock(peers: &Peers, c: usize, start: u32, behind: u32) -> u32 {
    let since = (server_time(peers, c).unwrap_or(0.0) - f64::from(start)).max(0.0).floor();
    ((since * 1000.0 / 30.0) as u32).saturating_sub(behind)
}

pub fn send_raw(peers: &mut Peers, c: usize, bytes: Vec<u8>) -> Result<(), String> {
    let entity = peers.client_entities[c];
    let world = peers.clients[c].world_mut();
    let mut sender = world
        .get_mut::<lightyear::prelude::MessageSender<crate::protocol::Wire<ssx_net_proto::ClientControl>>>(entity)
        .ok_or("no control sender")?;
    sender.send::<crate::protocol::ControlChannel>(crate::protocol::Wire::from_raw(bytes));
    Ok(())
}
