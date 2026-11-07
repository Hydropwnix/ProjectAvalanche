#![allow(dead_code)]

use core::time::Duration;

use bevy_ecs::prelude::*;
use ssx_formats::ghost::{POSE_BLOCK_SIZE, PoseBlock};
use ssx_net::harness::{Io, Peers};
use ssx_net::lightyear::prelude::{RoomAllocator, Rooms};
use ssx_net_proto::{NetPose, PoseFlags};

pub const CHARACTER: u32 = ssx_formats::attrib::fnv1(b"elise");

pub const FRAME: Duration = Duration::from_micros(16_667);

pub fn block(seed: u64) -> PoseBlock {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut b = [0u8; POSE_BLOCK_SIZE];
    for (i, v) in [0.0f32, 0.0, 0.0, 1.0, seed as f32, 2.0, -3.0].iter().enumerate() {
        b[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    for (start, end) in [(0x1C, 0x24), (0x30, 0x47), (0x4C, POSE_BLOCK_SIZE)] {
        for byte in &mut b[start..end] {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            *byte = (x >> 32) as u8;
        }
    }
    b[0x48..0x4C].copy_from_slice(&1.0f32.to_be_bytes());
    b[0x22..0x24].copy_from_slice(&(ssx_formats::ghost::pack_half(0.5) | 0x8000).to_be_bytes());
    PoseBlock::parse(&b).expect("240 bytes parse")
}

pub fn pose(tick: u32, seed: u64) -> NetPose {
    NetPose::new(tick, PoseFlags::default(), block(seed)).expect("a packer-shaped block")
}

pub fn connected(io: Io, n: usize, conditions: Option<ssx_net::harness::Conditions>) -> Peers {
    let mut peers = Peers::new(io, conditions, FRAME).expect("server");
    for id in 1..=n as u64 {
        peers.add_client(id).expect("client");
    }
    assert!(peers.steps_for(Duration::from_secs(10), |p| (0..n).all(|i| p.connected(i))), "clients connect");
    assert!(peers.steps_for(Duration::from_secs(10), |p| (0..n).all(|i| p.synced(i))), "clients sync their clocks");
    assert!(ready(&mut peers, &(0..n).collect::<Vec<_>>(), Duration::from_secs(10)), "the riders are ready");
    let room = peers.server.world_mut().resource_mut::<RoomAllocator>().allocate();
    put_in_room(&mut peers, (0..n).collect(), room);
    peers
}

pub fn ready(peers: &mut Peers, clients: &[usize], within: Duration) -> bool {
    let n = clients.len();
    if !peers.steps_for(within, |p| clients.iter().all(|&c| p.knows(c, n))) {
        return false;
    }
    for &c in clients {
        if peers.dress(c, CHARACTER).is_err() {
            return false;
        }
    }
    peers.steps_for(within, |p| clients.iter().all(|&c| p.riding(c)))
}

pub fn put_in_room(peers: &mut Peers, clients: Vec<usize>, room: ssx_net::lightyear::prelude::RoomId) {
    let lobby = peers.server.world().resource::<ssx_net::server::Lobby>().0;
    for c in clients {
        let link = peers.server_link(c).expect("server link of a connected client");
        peers.server.world_mut().entity_mut(link).insert(Rooms::from([lobby, room].into_iter()));
    }
}

pub fn server_tick(peers: &Peers) -> u32 {
    peers.server.world().resource::<ssx_net::lightyear::prelude::LocalTimeline>().tick().0
}

pub fn client_tick(peers: &Peers, client: usize) -> u32 {
    peers.clients[client].world().resource::<ssx_net::lightyear::prelude::LocalTimeline>().tick().0
}

pub fn entity_count<F: bevy_ecs::query::QueryFilter>(world: &mut World) -> usize {
    world.query_filtered::<Entity, F>().iter(world).count()
}
