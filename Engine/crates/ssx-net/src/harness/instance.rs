use core::time::Duration;

use bevy_ecs::prelude::*;
use lightyear::prelude::{Disconnected, MessageSender, NetworkTarget, Replicate, Rooms};
use ssx_net_proto::{Appearance, ClientControl, NetPose, PoseFlags, RelayedPose, RiderId, ServerChat};

use super::craft::Proxy;
use super::hostile::{proxied_client, sample, send_wire};
use super::{Io, Peers};
use crate::client::{ClientRoster, ControlOutbox, RemotePoses};
use crate::protocol::{ChatChannel, ControlChannel, PoseChannel, Wire};
use crate::roster::{InInstance, PlayerKeyId, PlayerName, RiderNumber};
use crate::server::{KickReason, Lobby, ServerStats};

pub(super) const FRAME: Duration = Duration::from_micros(16_667);

pub(super) const CHARACTER: u32 = ssx_formats::attrib::fnv1(b"elise");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstanceCase {
    ControlOutOfOrder,
    MalformedControl,
    AbsurdCoordinate,
    AbsurdRotation,
    AbsurdSpeed,
    AbsurdStep,
    NeverAcknowledges,
    MalformedRelayed,
    OversizedRelayed,
    UnknownRacer,
    NanRelayed,
    AbsurdRelayed,
    NameWithControls,
    LongName,
    PlayerFloodOneFrame,
    PlayerFlood,
    PlayerChurn,
    SilentServer,
}

#[derive(Clone, Debug, Default)]
pub struct InstanceOutcome {
    pub kicks: Vec<KickReason>,
    pub refusal: Option<String>,
    pub rider0_connected: bool,
    pub rider1_connected: bool,
    pub unknown_samples: u64,
    pub roster: usize,
    pub notes: Vec<String>,
}

pub(super) fn run(
    peers: &mut Peers,
    proxy: &mut Option<Proxy>,
    downstream: bool,
    timeout: Duration,
    mut done: impl FnMut(&mut Peers) -> bool,
) -> Result<bool, String> {
    let end = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < end {
        peers.step();
        if let Some(p) = proxy.as_mut() {
            if downstream { p.pump() } else { p.pump_upstream() }.map_err(|e| e.to_string())?;
        }
        if done(peers) {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn riders(proxied: bool) -> Result<(Peers, Option<Proxy>), String> {
    let mut peers = Peers::new(Io::Udp, None, FRAME)?;
    let mut proxy = if proxied { Some(proxied_client(&mut peers, 1)?.1) } else { None };
    if !proxied {
        peers.add_client(1)?;
    }
    peers.add_client(2)?;
    let up = |p: &mut Peers| (0..2).all(|c| p.connected(c) && p.synced(c) && p.knows(c, 2));
    if !run(&mut peers, &mut proxy, true, Duration::from_secs(10), up)? {
        return Err("the two riders did not connect".into());
    }
    for c in 0..2 {
        peers.dress(c, CHARACTER)?;
    }
    if !run(&mut peers, &mut proxy, true, Duration::from_secs(10), |p| p.riding(0) && p.riding(1))? {
        return Err("the two riders did not get ready".into());
    }
    Ok((peers, proxy))
}

fn edited(peers: &Peers, seed: u64, edit: impl FnOnce(&mut ssx_formats::ghost::PoseBlock)) -> Result<NetPose, String> {
    let base = sample(peers, seed);
    let mut block = base.block().clone();
    edit(&mut block);
    NetPose::new(base.tick(), PoseFlags::default(), block).map_err(|e| e.to_string())
}

fn relay_raw(peers: &mut Peers, bytes: Vec<u8>) -> Result<(), String> {
    let link = peers.server_link(0).ok_or("no link of rider 0")?;
    let mut sender =
        peers.server.world_mut().get_mut::<MessageSender<Wire<RelayedPose>>>(link).ok_or("no relay sender")?;
    sender.send::<PoseChannel>(Wire::from_raw(bytes));
    Ok(())
}

fn player(world: &mut World, rider: u16, name: String) -> Entity {
    let lobby = world.resource::<Lobby>().0;
    world
        .spawn((
            Replicate::to_clients(NetworkTarget::All),
            Rooms::single(lobby),
            RiderNumber(rider),
            PlayerName::unchecked(name),
            PlayerKeyId([7; 8]),
            InInstance(crate::server::INSTANCE_ID),
        ))
        .id()
}

pub fn run_instance(case: InstanceCase) -> Result<InstanceOutcome, String> {
    use InstanceCase as C;
    let proxied = matches!(case, C::NeverAcknowledges | C::SilentServer);
    let (mut peers, mut proxy) = riders(proxied)?;
    let mut o = InstanceOutcome::default();
    let mut downstream = true;
    let wait = |p: &mut Peers| {
        p.server.world().resource::<ServerStats>().kicked > 0
            || p.clients[0].world().resource::<RemotePoses>().refused.is_some()
    };
    let mut seconds = 5.0;
    match case {
        C::ControlOutOfOrder => {
            let look = Appearance::new(CHARACTER, 0).map_err(|e| e.to_string())?;
            peers.clients[0].world_mut().resource_mut::<ControlOutbox>().0.push(ClientControl::Appearance(look));
        }
        C::MalformedControl => {
            let entity = peers.client_entities[0];
            let world = peers.clients[0].world_mut();
            let mut sender = world.get_mut::<MessageSender<Wire<ClientControl>>>(entity).ok_or("no control sender")?;
            sender.send::<ControlChannel>(Wire::from_raw(vec![9, 0]));
        }
        C::AbsurdCoordinate | C::AbsurdRotation | C::AbsurdSpeed => {
            let pose = edited(&peers, 3, |b| match case {
                C::AbsurdCoordinate => b.trajectory_position[0] = 1.0e6,
                C::AbsurdRotation => b.trajectory_rotation = [2.0, 0.0, 0.0, 0.0],
                _ => b.speed = ssx_formats::ghost::pack_half(100.0) | 0x8000,
            })?;
            send_wire(&mut peers, 0, Wire::pose(&pose));
        }
        C::AbsurdStep => {
            let first = edited(&peers, 4, |b| b.trajectory_position = [0.0, 0.0, 0.0])?;
            send_wire(&mut peers, 0, Wire::pose(&first));
            run(&mut peers, &mut proxy, true, Duration::from_millis(200), |_| false)?;
            let second = NetPose::new(first.tick() + 2, PoseFlags::default(), {
                let mut b = first.block().clone();
                b.trajectory_position = [1000.0, 0.0, 0.0];
                b
            })
            .map_err(|e| e.to_string())?;
            send_wire(&mut peers, 0, Wire::pose(&second));
        }
        C::NeverAcknowledges => downstream = false,
        C::MalformedRelayed => relay_raw(&mut peers, vec![1, 2, 3])?,
        C::OversizedRelayed => relay_raw(&mut peers, vec![0; 4000])?,
        C::UnknownRacer => {
            let pose = sample(&peers, 5);
            relay_raw(&mut peers, RelayedPose { rider: RiderId(999), pose }.encode().to_vec())?;
            seconds = 2.0;
        }
        C::NanRelayed => {
            let mut bytes = RelayedPose { rider: RiderId(1), pose: sample(&peers, 6) }.encode().to_vec();
            bytes[2 + 8 + 16..2 + 8 + 20].copy_from_slice(&f32::NAN.to_be_bytes());
            relay_raw(&mut peers, bytes)?;
        }
        C::AbsurdRelayed => {
            let pose = edited(&peers, 7, |b| b.trajectory_position[2] = -1.0e6)?;
            relay_raw(&mut peers, RelayedPose { rider: RiderId(1), pose }.encode().to_vec())?;
        }
        C::NameWithControls => {
            player(peers.server.world_mut(), 77, "Evil\u{7}Name".into());
        }
        C::LongName => {
            player(peers.server.world_mut(), 78, "W".repeat(10_000));
        }
        C::PlayerFloodOneFrame => {
            for k in 0..1000u16 {
                player(peers.server.world_mut(), 100 + k, format!("P{k}"));
            }
        }
        C::PlayerFlood => {
            for k in 0..1000u16 {
                player(peers.server.world_mut(), 100 + k, format!("P{k}"));
                if k % 50 == 49 {
                    run(&mut peers, &mut proxy, true, FRAME, |_| false)?;
                }
            }
        }
        C::PlayerChurn => {
            let mut batch: Vec<Entity> = Vec::new();
            let mut next = 100u16;
            let end = std::time::Instant::now() + Duration::from_secs(3);
            while std::time::Instant::now() < end {
                for e in batch.drain(..) {
                    peers.server.world_mut().despawn(e);
                }
                for _ in 0..20 {
                    batch.push(player(peers.server.world_mut(), next, format!("C{next}")));
                    next = next.wrapping_add(1).max(100);
                }
                run(&mut peers, &mut proxy, true, FRAME, |_| false)?;
            }
            for e in batch {
                peers.server.world_mut().despawn(e);
            }
            seconds = 3.0;
        }
        C::SilentServer => {
            downstream = false;
            seconds = 20.0;
        }
    }
    let silent = case == C::SilentServer;
    let ended = run(&mut peers, &mut proxy, downstream, Duration::from_secs_f64(seconds), |p| {
        if case == C::NeverAcknowledges {
            for c in 0..2 {
                if let Some(link) = p.server_link(c)
                    && let Some(mut s) = p.server.world_mut().get_mut::<MessageSender<Wire<ServerChat>>>(link)
                {
                    for k in 0..10 {
                        let line = ServerChat::server(&format!("notice {k}")).expect("a short line");
                        s.send::<ChatChannel>(Wire::server_chat(&line));
                    }
                }
            }
        }
        if silent { p.clients[0].world().get::<Disconnected>(p.client_entities[0]).is_some() } else { wait(p) }
    })?;
    o.notes.push(format!("the case ended within {seconds} s: {ended}"));
    run(&mut peers, &mut proxy, downstream, Duration::from_secs(1), |_| false)?;
    let stats = peers.server.world().resource::<ServerStats>();
    o.kicks = stats.kick_reasons.iter().copied().collect();
    o.refusal = peers.clients[0].world().resource::<RemotePoses>().refused.clone();
    o.rider0_connected = peers.connected(0);
    o.rider1_connected = peers.connected(1);
    o.unknown_samples = peers.clients[0].world().resource::<ClientRoster>().unknown_samples;
    o.roster = peers.clients[1].world().resource::<ClientRoster>().players.len();
    if let Some(d) = peers.clients[0].world().get::<Disconnected>(peers.client_entities[0]) {
        o.notes.push(format!("rider 0 disconnected: {:?}", d.reason));
    }
    Ok(o)
}
