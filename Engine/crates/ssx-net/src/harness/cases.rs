use core::time::Duration;

use lightyear::prelude::Authentication;
use lightyear_replication::channels::RepliconUpdatesChannel;
use ssx_net_proto::{NET_POSE_SIZE, NetPose, PoseFlags, RelayedPose, RiderId, netcode_protocol_id};

use super::Peers;
use super::craft::{data_packet, fragment_packet, varint};
use super::hostile::*;
use crate::protocol::{PoseChannel, Wire};

fn notified(peers: &mut Peers, c: usize, outcome: &mut Outcome) {
    let gone = peers.steps_for(Duration::from_secs(2), |p| !p.connected(c));
    outcome.attacker_notified = Some(outcome.attacker_notified.unwrap_or(true) && gone);
}

fn attacker_state(peers: &mut Peers, c: usize) -> String {
    format!("attacker's link on the server: {}", if peers.server_link(c).is_some() { "kept" } else { "removed" })
}

fn frames(peers: &mut Peers, n: usize) {
    for _ in 0..n {
        peers.step();
    }
}

fn try_connect(peers: &mut Peers, id: u64, auth: Authentication) -> Result<bool, String> {
    let c = peers.add_client_with(id, auth)?;
    Ok(peers.steps_for(Duration::from_secs(3), |p| p.connected(c)))
}

pub(super) fn attack(case: Case, peers: &mut Peers, outcome: &mut Outcome) -> Result<(), String> {
    let server = peers.server_addr;
    match case {
        Case::GarbageDatagrams => {
            let sockets = sockets(300)?;
            let mut noise = noise(1);
            for (i, socket) in sockets.iter().enumerate() {
                for k in 0..4 {
                    let len = (noise.draw() % 1473) as usize;
                    let mut bytes = noise.bytes(len);
                    if k == 0 && len > 0 {
                        bytes[0] = (i % 8) as u8;
                    }
                    socket.send_to(&bytes, server).map_err(|e| e.to_string())?;
                }
                if i % 30 == 29 {
                    frames(peers, 1);
                }
            }
            frames(peers, 10);
            outcome.notes.push(format!("{} link entities right after the burst", link_count(peers)));
        }
        Case::TruncatedPackets => {
            let (c, mut proxy) = proxied_client(peers, 50)?;
            for k in 0..30 {
                step_proxied(peers, &mut proxy, 2)?;
                if peers.connected(c) {
                    let p = sample(peers, k);
                    peers.clients[c].world_mut().resource_mut::<crate::PoseOutbox>().next = Some(p);
                }
            }
            if !peers.connected(c) {
                return Err("the proxied client did not connect".into());
            }
            let mut kinds: Vec<Vec<u8>> = Vec::new();
            for d in &proxy.recorded {
                if !kinds.iter().any(|k| k.len() == d.len() && k.first() == d.first()) {
                    kinds.push(d.clone());
                }
            }
            let sockets = sockets(8)?;
            let mut sent = 0usize;
            for d in &kinds {
                for len in 0..d.len() {
                    sockets[sent % sockets.len()].send_to(&d[..len], server).map_err(|e| e.to_string())?;
                    proxy.inject(&d[..len]).map_err(|e| e.to_string())?;
                    sent += 2;
                    if sent.is_multiple_of(400) {
                        step_proxied(peers, &mut proxy, 1)?;
                    }
                }
                proxy.inject(d).map_err(|e| e.to_string())?;
                sockets[0].send_to(d, server).map_err(|e| e.to_string())?;
            }
            step_proxied(peers, &mut proxy, 30)?;
            outcome.notes.push(format!(
                "{} kinds of recorded datagram ({} recorded), {sent} prefixes sent; the proxied client still \
                 connected: {}",
                kinds.len(),
                proxy.recorded.len(),
                peers.connected(c)
            ));
        }
        Case::OversizedDatagrams => {
            let sockets = sockets(5)?;
            let mut noise = noise(2);
            for (socket, len) in sockets.iter().zip([1473usize, 1500, 2048, 9000, 65_507]) {
                for _ in 0..4 {
                    socket.send_to(&noise.bytes(len), server).map_err(|e| e.to_string())?;
                }
            }
            frames(peers, 10);
        }
        Case::WrongProtocolVersion => {
            let key = peers.key();
            let auth = Authentication::Manual {
                server_addr: server,
                client_id: 60,
                private_key: key,
                protocol_id: next_version_protocol_id(),
            };
            outcome.attacker_connected = try_connect(peers, 60, auth)?;
        }
        Case::InvalidToken => {
            let other = lightyear::netcode::try_generate_key().map_err(|e| e.to_string())?;
            let auth = Authentication::Manual {
                server_addr: server,
                client_id: 61,
                private_key: other,
                protocol_id: netcode_protocol_id(),
            };
            outcome.attacker_connected = try_connect(peers, 61, auth)?;
        }
        Case::ExpiredToken => {
            let auth = token(server, 62, peers.key(), 1)?;
            std::thread::sleep(Duration::from_millis(2_100));
            outcome.attacker_connected = try_connect(peers, 62, auth)?;
        }
        Case::WrongServerAddress => {
            let elsewhere = std::net::SocketAddr::from(([127, 0, 0, 1], 9));
            let auth = token(elsewhere, 63, peers.key(), 30)?;
            let c = peers.add_client_with(63, auth)?;
            let entity = peers.client_entities[c];
            peers.clients[c].world_mut().entity_mut(entity).insert(lightyear::prelude::PeerAddr(server));
            outcome.attacker_connected = peers.steps_for(Duration::from_secs(3), |p| p.connected(c));
        }
        Case::TokenReplay => {
            let mut proxy = super::craft::Proxy::new(server).map_err(|e| e.to_string())?;
            let front = proxy.addr().map_err(|e| e.to_string())?;
            let c = peers.add_client(70)?;
            let entity = peers.client_entities[c];
            peers.clients[c].world_mut().entity_mut(entity).insert(lightyear::prelude::PeerAddr(front));
            let mut request = None;
            for _ in 0..60 {
                frames(peers, 1);
                proxy.capture().map_err(|e| e.to_string())?;
                request = proxy.recorded.iter().find(|d| d.len() == 1078 && d[0] == 0).cloned();
                if request.is_some() {
                    break;
                }
            }
            let request = request.ok_or("the client sent no connect request")?;
            let sockets = sockets(300)?;
            for (i, socket) in sockets.iter().enumerate() {
                socket.send_to(&request, server).map_err(|e| e.to_string())?;
                if i % 30 == 29 {
                    frames(peers, 1);
                    proxy.capture().map_err(|e| e.to_string())?;
                }
            }
            frames(peers, 10);
            outcome.attacker_connected = peers.connected(c);
            outcome.notes.push(format!(
                "{} link entities right after the replay; netcode {:?}",
                link_count(peers),
                netcode_sizes(peers)
            ));
        }
        Case::MalformedPoses => {
            let valid = sample(peers, 5).encode();
            let mut variants: Vec<(String, Vec<u8>)> = [0usize, 1, NET_POSE_SIZE - 1, NET_POSE_SIZE + 1, 1000]
                .iter()
                .map(|&n| (format!("{n} bytes"), vec![0u8; n]))
                .collect();
            const BLOCK: usize = NET_POSE_SIZE - ssx_formats::ghost::POSE_BLOCK_SIZE;
            let mut flags = valid.to_vec();
            flags[4..8].copy_from_slice(&0x0000_0010u32.to_be_bytes());
            variants.push(("reserved flag".into(), flags));
            let mut execution = valid.to_vec();
            execution[8..12].copy_from_slice(&(ssx_net_proto::MAX_EXECUTION + 1).to_be_bytes());
            variants.push(("execution beyond 24 bits".into(), execution));
            let mut nan = valid.to_vec();
            nan[BLOCK + 0x10..BLOCK + 0x14].copy_from_slice(&f32::NAN.to_be_bytes());
            variants.push(("NaN position".into(), nan));
            let mut weight = valid.to_vec();
            weight[BLOCK + 0x48..BLOCK + 0x4C].copy_from_slice(&0.5f32.to_be_bytes());
            variants.push(("weight 0.5".into(), weight));
            let mut unwritten = valid.to_vec();
            unwritten[BLOCK + 0x47] = 1;
            variants.push(("unwritten byte".into(), unwritten));
            for (i, (name, bytes)) in variants.into_iter().enumerate() {
                let c = attacker(peers, 100 + i as u64)?;
                send_wire(peers, c, Wire::from_raw(bytes));
                frames(peers, 10);
                outcome.notes.push(format!("{name}: {}", attacker_state(peers, c)));
                notified(peers, c, outcome);
            }
        }
        Case::UnknownMessageId => {
            let c = attacker(peers, 70)?;
            let channel = net_id::<PoseChannel>(peers)?;
            let mut message = varint(16_383);
            message.extend_from_slice(&[0u8; 10]);
            inject_channel(peers, c, channel, message);
            frames(peers, 10);
            outcome.notes.push(attacker_state(peers, c));
            notified(peers, c, outcome);
        }
        Case::UnknownChannel => {
            let c = attacker(peers, 71)?;
            inject_raw(peers, c, data_packet(16_383, &[0u8; 4]));
            frames(peers, 10);
            outcome.notes.push(attacker_state(peers, c));
            notified(peers, c, outcome);
        }
        Case::GarbagePayload => {
            let c = attacker(peers, 72)?;
            let mut noise = noise(3);
            for _ in 0..200 {
                let len = 1 + (noise.draw() % 1100) as usize;
                let bytes = noise.bytes(len);
                inject_raw(peers, c, bytes);
            }
            frames(peers, 10);
            outcome.notes.push(attacker_state(peers, c));
            notified(peers, c, outcome);
        }
        Case::WrongDirection => {
            let c = attacker(peers, 73)?;
            let pose = sample(peers, 6);
            send_relayed(peers, c, Wire::relayed(&RelayedPose { rider: RiderId(0), pose }));
            frames(peers, 10);
            outcome.notes.push(attacker_state(peers, c));
            notified(peers, c, outcome);
        }
        Case::ReplicationFromClient => {
            let c = attacker(peers, 74)?;
            let channel = net_id::<RepliconUpdatesChannel>(peers)?;
            inject_channel(peers, c, channel, vec![0u8; 8]);
            frames(peers, 10);
            outcome.notes.push(attacker_state(peers, c));
            notified(peers, c, outcome);
        }
        Case::FragmentBomb => {
            let c = attacker(peers, 75)?;
            let channel = net_id::<PoseChannel>(peers)?;
            inject_raw(peers, c, fragment_packet(1, channel, 7, 1, 1 << 40, &[0xAB]));
            for _ in 0..10 {
                frames(peers, 1);
                outcome.max_fragment_bytes_in_flight =
                    outcome.max_fragment_bytes_in_flight.max(super::hostile::fragment_bytes_in_flight(peers));
            }
            outcome.notes.push(attacker_state(peers, c));
            notified(peers, c, outcome);
        }
        Case::FragmentFlood => {
            let c = attacker(peers, 78)?;
            let link = peers.server_link(c).ok_or("no link of the attacker")?;
            let mtu =
                peers.server.world().get::<lightyear::prelude::Link>(link).map(|l| l.min_mtu()).ok_or("no link")?;
            let size = lightyear::transport::packet::fragment_size_for_min_mtu(mtu).ok_or("no fragment size")?;
            let fragments = (crate::limits::MAX_FRAGMENTED_MESSAGE / size).max(2) as u64;
            let payload = vec![0x5Au8; size];
            let channels = [net_id::<crate::protocol::ControlChannel>(peers)?, net_id::<PoseChannel>(peers)?];
            let mut packet_id = 100u16;
            for id in 1..=300u16 {
                for &channel in &channels {
                    packet_id = packet_id.wrapping_add(1);
                    inject_raw(peers, c, fragment_packet(packet_id, channel, id, 1, fragments, &payload));
                }
                if id % 10 == 0 {
                    frames(peers, 1);
                    outcome.max_fragment_bytes_in_flight =
                        outcome.max_fragment_bytes_in_flight.max(super::hostile::fragment_bytes_in_flight(peers));
                }
            }
            for _ in 0..10 {
                frames(peers, 1);
                outcome.max_fragment_bytes_in_flight =
                    outcome.max_fragment_bytes_in_flight.max(super::hostile::fragment_bytes_in_flight(peers));
            }
            outcome.notes.push(format!(
                "fragment size {size} B, {fragments} fragments declared per message; {}",
                attacker_state(peers, c)
            ));
            notified(peers, c, outcome);
        }
        Case::PoseFlood => {
            let c = attacker(peers, 76)?;
            let link = peers.server_link(c).ok_or("no link of the attacker")?;
            let meter = |peers: &Peers| {
                let sent = peers.client_meter(c).sent_datagrams;
                let got = peers.server.world().get::<super::WireMeter>(link).map_or(0, |m| m.recv_datagrams);
                (sent, got)
            };
            let before = meter(peers);
            let base = sample(peers, 7);
            for k in 0..1000u32 {
                let tick = base.tick().saturating_sub(145).saturating_add(k % 290);
                let pose = NetPose::new(tick, PoseFlags::default(), base.block().clone()).map_err(|e| e.to_string())?;
                send_wire(peers, c, Wire::pose(&pose));
            }
            frames(peers, 10);
            let after = meter(peers);
            outcome.notes.push(format!(
                "datagrams: the attacker sent {}, the server's link received {}; {}",
                after.0.saturating_sub(before.0),
                after.1.saturating_sub(before.1),
                attacker_state(peers, c)
            ));
        }
        Case::PongOverflow => {
            let c = attacker(peers, 79)?;
            peers.clients[c].world_mut().resource_mut::<super::Injector>().hijack_pings = true;
            frames(peers, 120);
            let hijacked = peers.clients[c].world().resource::<super::Injector>().hijacked;
            outcome.notes.push(format!("{hijacked} pongs of 2^31 ticks answered; {}", attacker_state(peers, c)));
        }
        Case::MetadataFromClient => {
            use lightyear::prelude::EventSender;
            use lightyear_replication::metadata::{MetadataChannel, SenderMetadata};
            let c = attacker(peers, 80)?;
            let entity = peers.client_entities[c];
            let world = peers.clients[c].world_mut();
            if world.get::<EventSender<SenderMetadata>>(entity).is_none() {
                world.entity_mut(entity).insert(EventSender::<SenderMetadata>::default());
            }
            let mut sender = world.get_mut::<EventSender<SenderMetadata>>(entity).ok_or("no metadata sender")?;
            for i in 0..1000u32 {
                let sender_entity = bevy_ecs::entity::Entity::from_raw_u32(1 + i).ok_or("entity")?;
                let send_interval = lightyear::core::time::PositiveTickDelta::lit("1");
                sender.trigger::<MetadataChannel>(SenderMetadata { send_interval, sender_entity });
            }
            frames(peers, 30);
            let mapped = peers
                .server_link(c)
                .and_then(|l| peers.server.world().get::<lightyear::prelude::MessageManager>(l))
                .map(|m| m.entity_mapper.remote_to_local.len());
            outcome.notes.push(format!("server link's remote entity map: {mapped:?}; {}", attacker_state(peers, c)));
            notified(peers, c, outcome);
        }
        Case::ProtocolMismatch
        | Case::ReplicationFromServer
        | Case::MetadataOverflowFromServer
        | Case::ReplicationUnknownFnsFromServer
        | Case::ReplicationOversizeFromServer => {}
        Case::PoseTickOutOfWindow => {
            let c = attacker(peers, 77)?;
            let base = sample(peers, 8);
            let pose = NetPose::new(base.tick() + 10_000, PoseFlags::default(), base.block().clone())
                .map_err(|e| e.to_string())?;
            send_wire(peers, c, Wire::pose(&pose));
            frames(peers, 10);
            outcome.notes.push(attacker_state(peers, c));
            notified(peers, c, outcome);
        }
        Case::PoseViewOutOfWindow => {
            let c = attacker(peers, 78)?;
            let base = sample(peers, 8);
            let pose = base.clone().with_view_time(base.time() + 10_000.0).map_err(|e| e.to_string())?;
            send_wire(peers, c, Wire::pose(&pose));
            frames(peers, 10);
            outcome.notes.push(attacker_state(peers, c));
            notified(peers, c, outcome);
        }
    }
    Ok(())
}
