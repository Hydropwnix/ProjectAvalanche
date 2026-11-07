use core::time::Duration;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};

use bevy_ecs::prelude::*;
pub use lightyear::netcode::ServerStateSizes as NetcodeSizes;
use lightyear::netcode::{ConnectToken, NetcodeServer};
use lightyear::prelude::{
    Authentication, ChannelRegistry, LinkOf, LocalTimeline, MessageSender, PeerAddr, RoomAllocator, Rooms,
};
use lightyear::transport::channel::ChannelKind;
use ssx_net_proto::{NetPose, PROTOCOL_MAGIC, PROTOCOL_VERSION, PoseFlags, RelayedPose, netcode_protocol_id};

use super::craft::{Noise, Proxy};
use super::{Injector, Io, Peers, ServerKind};
use crate::client::{PoseOutbox, RemotePoses};
use crate::protocol::{PoseChannel, Wire};
use crate::server::{KickReason, ServerStats};

pub(super) const FRAME: Duration = Duration::from_micros(16_667);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Case {
    GarbageDatagrams,
    TruncatedPackets,
    OversizedDatagrams,
    WrongProtocolVersion,
    InvalidToken,
    ExpiredToken,
    WrongServerAddress,
    TokenReplay,
    MalformedPoses,
    UnknownMessageId,
    UnknownChannel,
    GarbagePayload,
    WrongDirection,
    ReplicationFromClient,
    FragmentBomb,
    FragmentFlood,
    PoseFlood,
    PoseTickOutOfWindow,
    PoseViewOutOfWindow,
    ProtocolMismatch,
    ReplicationFromServer,
    PongOverflow,
    MetadataFromClient,
    MetadataOverflowFromServer,
    ReplicationUnknownFnsFromServer,
    ReplicationOversizeFromServer,
}

impl Case {
    pub const ALL: [Case; 26] = [
        Case::GarbageDatagrams,
        Case::TruncatedPackets,
        Case::OversizedDatagrams,
        Case::WrongProtocolVersion,
        Case::InvalidToken,
        Case::ExpiredToken,
        Case::WrongServerAddress,
        Case::TokenReplay,
        Case::MalformedPoses,
        Case::UnknownMessageId,
        Case::UnknownChannel,
        Case::GarbagePayload,
        Case::WrongDirection,
        Case::ReplicationFromClient,
        Case::FragmentBomb,
        Case::FragmentFlood,
        Case::PoseFlood,
        Case::PoseTickOutOfWindow,
        Case::PoseViewOutOfWindow,
        Case::ProtocolMismatch,
        Case::ReplicationFromServer,
        Case::PongOverflow,
        Case::MetadataFromClient,
        Case::MetadataOverflowFromServer,
        Case::ReplicationUnknownFnsFromServer,
        Case::ReplicationOversizeFromServer,
    ];

    pub fn from_server(self) -> bool {
        matches!(
            self,
            Case::ReplicationFromServer
                | Case::MetadataOverflowFromServer
                | Case::ReplicationUnknownFnsFromServer
                | Case::ReplicationOversizeFromServer
        )
    }

    pub fn name(self) -> String {
        format!("{self:?}")
    }

    pub fn from_name(name: &str) -> Option<Case> {
        Case::ALL.into_iter().find(|c| c.name() == name)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Outcome {
    pub attacker_connected: bool,
    pub kicks: Vec<KickReason>,
    pub riders_connected: bool,
    pub newcomer_connected: bool,
    pub relay_after: Option<bool>,
    pub relay: Option<RelayCheck>,
    pub rider_numbers: [Option<u16>; 2],
    pub links_after_attack: usize,
    pub links_after_timeout: usize,
    pub addresses_after_timeout: Option<usize>,
    pub netcode_after_attack: Option<NetcodeSizes>,
    pub netcode_after_timeout: Option<NetcodeSizes>,
    pub attacker_notified: Option<bool>,
    pub max_fragment_bytes_in_flight: usize,
    pub rss_growth_kib: Option<i64>,
    pub client_refusals: Vec<String>,
    pub stats: ServerStats,
    pub notes: Vec<String>,
}

const RELAY_STEPS: u32 = 10;

const RELAY_ATTEMPTS: u32 = 5;

#[derive(Clone, Debug, Default)]
pub struct RelayCheck {
    pub arrived: bool,
    pub attempts: u32,
    pub steps: u32,
    pub wall: Duration,
    pub traces: Vec<String>,
}

fn relay_check(peers: &mut Peers) -> RelayCheck {
    let started = std::time::Instant::now();
    let mut check = RelayCheck::default();
    let within = |started: std::time::Instant| started.elapsed() < super::HANG;
    while check.attempts < RELAY_ATTEMPTS && within(started) {
        check.attempts += 1;
        let sent_before = peers.clients[0].world().resource::<PoseOutbox>().sent;
        let mut offered = sample(peers, 99 + u64::from(check.attempts));
        let mut waited = 0u32;
        while peers.clients[0].world().resource::<PoseOutbox>().sent == sent_before
            && waited < RELAY_STEPS
            && within(started)
        {
            offered = sample(peers, 99 + u64::from(check.attempts));
            peers.clients[0].world_mut().resource_mut::<PoseOutbox>().next = Some(offered.clone());
            peers.step();
            (waited, check.steps) = (waited + 1, check.steps + 1);
        }
        let sent = peers.clients[0].world().resource::<PoseOutbox>().sent > sent_before;
        let crossed = |peers: &Peers| {
            peers.rider(0).is_some_and(|sender| {
                peers.clients[1]
                    .world()
                    .resource::<RemotePoses>()
                    .riders
                    .get(&sender)
                    .is_some_and(|r| r.pose == offered)
            })
        };
        let mut steps = 0u32;
        while sent && !crossed(peers) && steps < RELAY_STEPS && within(started) {
            peers.step();
            (steps, check.steps) = (steps + 1, check.steps + 1);
        }
        if sent && crossed(peers) {
            check.arrived = true;
            break;
        }
        check.traces.push(relay_trace(peers, &offered, sent, steps, started.elapsed()));
    }
    check.wall = started.elapsed();
    check
}

fn relay_trace(peers: &mut Peers, p: &NetPose, sent: bool, steps: u32, waited: Duration) -> String {
    let links: Vec<String> = {
        let world = peers.server.world_mut();
        let mut q = world.query_filtered::<(
            &crate::server::RiderLink,
            Option<&Rooms>,
            Option<&crate::server::PlacedIn>,
            Has<lightyear::prelude::Connected>,
            Has<crate::server::Kicked>,
        ), With<lightyear::prelude::server::ClientOf>>();
        q.iter(world)
            .map(|(link, rooms, placed, connected, kicked)| {
                let rooms: Vec<String> = rooms.map_or_else(Vec::new, |r| r.rooms().map(|r| format!("{r:?}")).collect());
                format!(
                    "rider {} rooms [{}] placed {:?} connected {connected} kicked {kicked}",
                    link.rider().0,
                    rooms.join(" "),
                    placed.map(|p| p.0)
                )
            })
            .collect()
    };
    let receiver_connected = peers.connected(1);
    let outbox = peers.clients[0].world().resource::<PoseOutbox>();
    let receiver = peers.clients[1].world();
    let roster = receiver.resource::<crate::client::ClientRoster>();
    let refused = receiver.resource::<RemotePoses>().refused.clone();
    let sender = peers.rider(0);
    let newest = sender
        .and_then(|s| receiver.resource::<RemotePoses>().riders.get(&s))
        .map(|r| (r.pose.tick(), r.samples, r.stale));
    let server_tick = peers.server.world().resource::<LocalTimeline>().tick().0;
    let stats = peers.server.world().get_resource::<ServerStats>();
    format!(
        "relay trace: the sample's tick {} (the server's now {server_tick}), sent {sent}; the sender's outbox pending \
         {}, sent {}, off tick {}, implausible {}; the server received {}, relayed {}; the receiver knows the sender {}, \
         unknown samples {}, its newest of the sender {sender:?} (tick, samples, stale) {newest:?}, connected \
         {receiver_connected}, refused {refused:?}, own {:?}; the server's links: {}; {steps} steps after the send, \
         {:.2} s since the check began",
        p.tick(),
        outbox.next.is_some(),
        outbox.sent,
        outbox.off_tick,
        outbox.implausible,
        stats.map_or(0, |s| s.poses_received),
        stats.map_or(0, |s| s.poses_relayed),
        sender.is_some_and(|s| roster.players.contains_key(&s)),
        roster.unknown_samples,
        roster.own,
        links.join("; "),
        waited.as_secs_f64()
    )
}

pub(super) fn rss_kib() -> Option<i64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("VmRSS:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

pub(super) fn address_count(peers: &Peers) -> Option<usize> {
    peers
        .server
        .world()
        .get::<lightyear::prelude::endpoint::UdpEndpoint>(peers.server_entity)
        .map(|e| e.address_count())
}

pub(super) fn netcode_sizes(peers: &Peers) -> Option<NetcodeSizes> {
    peers.server.world().get::<NetcodeServer>(peers.server_entity).map(NetcodeServer::state_sizes)
}

pub(super) fn fragment_bytes_in_flight(peers: &mut Peers) -> usize {
    let world = peers.server.world_mut();
    let mut transports = world.query_filtered::<&mut lightyear::prelude::Transport, With<LinkOf>>();
    transports
        .iter_mut(world)
        .map(|mut t| t.channel_receives_mut().map(|c| c.fragment_bytes_in_flight()).sum::<usize>())
        .max()
        .unwrap_or(0)
}

pub(super) fn link_count(peers: &mut Peers) -> usize {
    let world = peers.server.world_mut();
    world.query_filtered::<Entity, With<LinkOf>>().iter(world).count()
}

pub(super) fn net_id<C: Send + Sync + 'static>(peers: &Peers) -> Result<u16, String> {
    peers
        .server
        .world()
        .resource::<ChannelRegistry>()
        .get_net_from_kind(&ChannelKind::of::<C>())
        .copied()
        .ok_or_else(|| format!("{} is not registered", core::any::type_name::<C>()))
}

pub(super) fn sample(peers: &Peers, seed: u64) -> NetPose {
    let tick = peers.server.world().resource::<LocalTimeline>().tick().0;
    NetPose::new(tick, PoseFlags::default(), super::sample_block(seed)).unwrap_or_else(|e| panic!("sample: {e}"))
}

pub(super) const TEST_CHARACTER: u32 = ssx_formats::attrib::fnv1(b"elise");

pub fn run(case: Case, kind: ServerKind) -> Result<Outcome, String> {
    let kind = if case == Case::ProtocolMismatch { ServerKind::Mismatched } else { kind };
    let mut peers = Peers::with_server(kind, Io::Udp, None, FRAME)?;
    let mut outcome = Outcome::default();
    for id in [1, 2] {
        peers.add_client(id)?;
    }
    if case == Case::ProtocolMismatch {
        return mismatch(&mut peers, outcome);
    }
    if !peers.steps_for(Duration::from_secs(10), |p| p.connected(0) && p.connected(1) && p.synced(0) && p.synced(1)) {
        return Err("the two riders did not connect".into());
    }
    if case.from_server() {
        return hostile_server(case, &mut peers, outcome);
    }
    if kind != ServerKind::Bare {
        if !peers.steps_for(Duration::from_secs(10), |p| p.knows(0, 2) && p.knows(1, 2)) {
            return Err("the two riders' rosters did not fill".into());
        }
        for c in [0, 1] {
            peers.dress(c, TEST_CHARACTER)?;
        }
        if !peers.steps_for(Duration::from_secs(10), |p| p.riding(0) && p.riding(1)) {
            return Err("the two riders did not get ready".into());
        }
    }
    let room = peers.server.world_mut().resource_mut::<RoomAllocator>().allocate();
    let lobby = peers.server.world().get_resource::<crate::server::Lobby>().map(|l| l.0);
    for c in [0, 1] {
        let link = peers.server_link(c).ok_or("no server link")?;
        peers.server.world_mut().entity_mut(link).insert(Rooms::from(lobby.into_iter().chain([room])));
    }
    let rss_before = rss_kib();
    super::cases::attack(case, &mut peers, &mut outcome)?;
    outcome.rss_growth_kib = rss_before.zip(rss_kib()).map(|(before, after)| after - before);
    for _ in 0..30 {
        peers.step();
    }
    outcome.links_after_attack = link_count(&mut peers);
    outcome.netcode_after_attack = netcode_sizes(&peers);
    outcome.riders_connected = peers.connected(0) && peers.connected(1);
    let newcomer = peers.add_client(1_000)?;
    outcome.newcomer_connected = peers.steps_for(Duration::from_secs(5), |p| p.connected(newcomer));
    if kind != ServerKind::Bare && outcome.riders_connected {
        outcome.rider_numbers = [peers.rider(0).map(|r| r.0), peers.rider(1).map(|r| r.0)];
        let relay = relay_check(&mut peers);
        outcome.relay_after = Some(relay.arrived);
        outcome.relay = Some(relay);
    }
    if matches!(case, Case::GarbageDatagrams | Case::TokenReplay) {
        let token_timeout = Duration::from_secs(crate::limits::CONNECTION_TIMEOUT_SECS.unsigned_abs().into());
        peers.run_until(crate::limits::PENDING_LINK_TIMEOUT.max(token_timeout) + Duration::from_secs(1), |_| false);
        outcome.links_after_timeout = link_count(&mut peers);
        outcome.addresses_after_timeout = address_count(&peers);
        outcome.netcode_after_timeout = netcode_sizes(&peers);
    } else {
        outcome.links_after_timeout = outcome.links_after_attack;
    }
    if let Some(stats) = peers.server.world().get_resource::<ServerStats>() {
        outcome.stats = stats.clone();
        outcome.kicks = stats.kick_reasons.iter().copied().collect();
    }
    Ok(outcome)
}

fn mismatch(peers: &mut Peers, mut outcome: Outcome) -> Result<Outcome, String> {
    let checked = peers.steps_for(Duration::from_secs(10), |p| {
        (0..2).all(|c| p.clients[c].world().resource::<RemotePoses>().refused.is_some())
    });
    peers.run_until(Duration::from_secs(1), |_| false);
    outcome.client_refusals =
        (0..2).filter_map(|c| peers.clients[c].world().resource::<RemotePoses>().refused.clone()).collect();
    outcome.riders_connected = peers.connected(0) || peers.connected(1);
    outcome.notes.push(format!("both riders refused the server within 10 s of steps: {checked}"));
    outcome.links_after_attack = link_count(peers);
    outcome.links_after_timeout = outcome.links_after_attack;
    Ok(outcome)
}

fn hostile_server(case: Case, peers: &mut Peers, mut outcome: Outcome) -> Result<Outcome, String> {
    use lightyear_replication::channels::{RepliconMutationAcksChannel, RepliconUpdatesChannel};
    let link = peers.server_link(0).ok_or("no link of rider 0")?;
    let (kind, bytes) = match case {
        Case::ReplicationFromServer => (ChannelKind::of::<RepliconMutationAcksChannel>(), vec![0u8; 8]),
        Case::ReplicationUnknownFnsFromServer => {
            (ChannelKind::of::<RepliconUpdatesChannel>(), vec![0x10, 1, 0, 1, 0x7F])
        }
        Case::ReplicationOversizeFromServer => (ChannelKind::of::<RepliconUpdatesChannel>(), vec![0x10, 1, 0, 100, 0]),
        Case::MetadataOverflowFromServer => {
            use lightyear::core::time::PositiveTickDelta;
            use lightyear_replication::metadata::{MetadataChannel, SenderMetadata};
            let metadata = SenderMetadata { send_interval: PositiveTickDelta::lit("2147483648"), sender_entity: link };
            let world = peers.server.world_mut();
            let mut sender = world
                .get_mut::<lightyear::prelude::EventSender<SenderMetadata>>(link)
                .ok_or("no metadata sender on the server link")?;
            sender.trigger::<MetadataChannel>(metadata);
            return server_attack_outcome(peers, outcome);
        }
        _ => return Err(format!("{case:?} is not a hostile-server case")),
    };
    peers
        .server
        .world()
        .get::<lightyear::prelude::Transport>(link)
        .ok_or("no transport")?
        .send_erased(kind, bytes.into(), 1.0)
        .map_err(|e| e.to_string())?;
    outcome = server_attack_outcome(peers, outcome)?;
    Ok(outcome)
}

fn server_attack_outcome(peers: &mut Peers, mut outcome: Outcome) -> Result<Outcome, String> {
    let refused = peers.steps_for(Duration::from_secs(5), |p| {
        p.clients[0].world().resource::<RemotePoses>().refused.is_some() && !p.connected(0)
    });
    outcome.client_refusals =
        (0..2).filter_map(|c| peers.clients[c].world().resource::<RemotePoses>().refused.clone()).collect();
    outcome.riders_connected = peers.connected(1);
    outcome.notes.push(format!("rider 0 refused the server and disconnected within 5 s of steps: {refused}"));
    let newcomer = peers.add_client(1_000)?;
    outcome.newcomer_connected = peers.steps_for(Duration::from_secs(5), |p| p.connected(newcomer));
    outcome.links_after_attack = link_count(peers);
    outcome.links_after_timeout = outcome.links_after_attack;
    Ok(outcome)
}

pub(super) fn sockets(n: usize) -> Result<Vec<UdpSocket>, String> {
    (0..n).map(|_| UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|e| e.to_string())).collect()
}

pub(super) fn proxied_client(peers: &mut Peers, id: u64) -> Result<(usize, Proxy), String> {
    let proxy = Proxy::new(peers.server_addr).map_err(|e| e.to_string())?;
    let front = proxy.addr().map_err(|e| e.to_string())?;
    let c = peers.add_client(id)?;
    let entity = peers.client_entities[c];
    peers.clients[c].world_mut().entity_mut(entity).insert(PeerAddr(front));
    Ok((c, proxy))
}

pub(super) fn step_proxied(peers: &mut Peers, proxy: &mut Proxy, frames: usize) -> Result<(), String> {
    for _ in 0..frames {
        peers.step();
        proxy.pump().map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub(super) fn attacker(peers: &mut Peers, id: u64) -> Result<usize, String> {
    let c = peers.add_client(id)?;
    peers.clients[c].world_mut().insert_resource(lightyear::prelude::ReliableLimits::default());
    if !peers.steps_for(Duration::from_secs(5), |p| p.connected(c)) {
        return Err(format!("attacker {id} did not connect"));
    }
    Ok(c)
}

pub(super) fn inject_channel(peers: &mut Peers, c: usize, channel: u16, bytes: Vec<u8>) {
    peers.clients[c].world_mut().resource_mut::<Injector>().on_channel.push((channel, bytes));
}

pub(super) fn inject_raw(peers: &mut Peers, c: usize, bytes: Vec<u8>) {
    peers.clients[c].world_mut().resource_mut::<Injector>().raw.push(bytes);
}

pub(super) fn send_wire(peers: &mut Peers, c: usize, wire: Wire<NetPose>) {
    let entity = peers.client_entities[c];
    if let Some(mut sender) = peers.clients[c].world_mut().get_mut::<MessageSender<Wire<NetPose>>>(entity) {
        sender.send::<PoseChannel>(wire);
    }
}

pub(super) fn send_relayed(peers: &mut Peers, c: usize, wire: Wire<RelayedPose>) {
    let entity = peers.client_entities[c];
    let world = peers.clients[c].world_mut();
    if world.get::<MessageSender<Wire<RelayedPose>>>(entity).is_none() {
        world.entity_mut(entity).insert(MessageSender::<Wire<RelayedPose>>::default());
    }
    if let Some(mut sender) = world.get_mut::<MessageSender<Wire<RelayedPose>>>(entity) {
        sender.send::<PoseChannel>(wire);
    }
}

pub(super) fn next_version_protocol_id() -> u64 {
    debug_assert_ne!(netcode_protocol_id(), ((PROTOCOL_MAGIC as u64) << 32) | u64::from(PROTOCOL_VERSION + 1));
    ((PROTOCOL_MAGIC as u64) << 32) | u64::from(PROTOCOL_VERSION + 1)
}

pub(super) fn token(server: SocketAddr, id: u64, key: [u8; 32], expire_secs: i32) -> Result<Authentication, String> {
    ConnectToken::build(server, netcode_protocol_id(), id, key)
        .expire_seconds(expire_secs)
        .generate()
        .map(Authentication::Token)
        .map_err(|e| e.to_string())
}

pub(super) fn noise(seed: u64) -> Noise {
    Noise(seed | 1)
}
