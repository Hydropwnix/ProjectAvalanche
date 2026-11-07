mod common;

use core::time::Duration;

use common::*;
use ssx_net::harness::{Io, Peers};
use ssx_net::{RemotePoses, ServerStats};

const SPACING: u32 = 4;

const WITHIN: Duration = Duration::from_secs(5);

fn holds(peers: &Peers, receiver: usize, sender: ssx_net_proto::RiderId, p: &ssx_net_proto::NetPose) -> bool {
    peers.clients[receiver].world().resource::<RemotePoses>().riders.get(&sender).is_some_and(|r| r.pose == *p)
}

fn relay_reaches_the_other_rider(io: Io) {
    let mut peers = connected(io, 2, None);
    let sender = peers.rider(0).expect("client 0's rider number");
    let mut sent = Vec::new();
    let mut last: Option<u32> = None;
    for k in 0..20u32 {
        if let Some(t) = last {
            assert!(peers.steps_for(WITHIN, |p| server_tick(p) >= t + SPACING), "the server's tick advances");
        }
        let tick = server_tick(&peers).max(1);
        let p = pose(tick, u64::from(k) + 7);
        assert!(peers.send_pose(0, &p, WITHIN), "sample {k} sent");
        assert!(peers.steps_for(WITHIN, |peers| holds(peers, 1, sender, &p)), "sample {k} relayed");
        sent.push(p);
        last = Some(tick);
    }
    let remote = peers.clients[1].world().resource::<RemotePoses>();
    let rider = remote.riders.get(&sender).expect("client 0's samples");
    assert_eq!(rider.samples, 20);
    assert_eq!(rider.pose.encode(), sent.last().unwrap().encode(), "the newest sample, byte for byte");
    assert!(peers.clients[0].world().resource::<RemotePoses>().riders.is_empty(), "no echo to the sender");
    let stats = peers.server.world().resource::<ServerStats>().clone();
    assert_eq!((stats.poses_received, stats.poses_relayed, stats.kicked), (20, 20, 0), "{stats:?}");
}

#[test]
fn relay_over_crossbeam() {
    relay_reaches_the_other_rider(Io::Crossbeam);
}

#[test]
fn relay_over_udp() {
    relay_reaches_the_other_rider(Io::Udp);
}

fn a_senders_stamps_falling_back_keep_its_samples(io: Io) {
    let mut peers = connected(io, 2, None);
    let sender = peers.rider(0).expect("client 0's rider number");
    let mut last: Option<u32> = None;
    for k in 0..20u32 {
        if let Some(t) = last {
            assert!(peers.steps_for(WITHIN, |p| server_tick(p) >= t + SPACING), "the server's tick advances");
        }
        let now = server_tick(&peers).max(60);
        let tick = if k < 10 { now } else { now - 52 };
        let p = pose(tick, u64::from(k) + 7).with_execution(1 + k * SPACING).expect("a count");
        assert!(peers.send_pose(0, &p, WITHIN), "sample {k} sent");
        assert!(peers.steps_for(WITHIN, |peers| holds(peers, 1, sender, &p)), "sample {k} relayed and kept");
        last = Some(now);
    }
    let remote = peers.clients[1].world().resource::<RemotePoses>();
    let rider = remote.riders.get(&sender).expect("client 0's samples");
    assert_eq!((rider.samples, rider.stale), (20, 0));
    let stats = peers.server.world().resource::<ServerStats>().clone();
    assert_eq!((stats.poses_received, stats.poses_relayed, stats.kicked), (20, 20, 0), "{stats:?}");
}

#[test]
fn stamps_falling_back_over_crossbeam() {
    a_senders_stamps_falling_back_keep_its_samples(Io::Crossbeam);
}

#[test]
fn stamps_falling_back_over_udp() {
    a_senders_stamps_falling_back_keep_its_samples(Io::Udp);
}
