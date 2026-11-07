mod common;

use core::time::Duration;

use ssx_net::harness::{Io, Peers};
use ssx_net::lightyear::prelude::Disconnect;
use ssx_net::{ClientRoster, RemotePoses};
use ssx_net_proto::keys::PlayerSecret;

fn ready(p: &mut Peers, c: usize) -> bool {
    p.connected(c) && p.knows(c, 1) && p.clients[c].world().resource::<ClientRoster>().instance.is_some()
}

fn leave(peers: &mut Peers, c: usize) {
    let entity = peers.client_entities[c];
    peers.clients[c].world_mut().trigger(Disconnect { entity });
    assert!(peers.steps_for(Duration::from_secs(5), |p| !p.connected(c)));
    let w = peers.clients[c].world_mut();
    let left = w.query_filtered::<(), bevy_ecs::query::With<ssx_net::roster::RiderNumber>>().iter(w).count();
    assert_eq!(left, 0, "lightyear despawns the replicated players with the link");
}

#[test]
fn a_second_join_starts_from_nothing() {
    let mut peers = Peers::new(Io::Crossbeam, None, Duration::from_millis(16)).expect("peers");
    let player = PlayerSecret::generate().public();
    let c = peers.add_player(player, "Elise").expect("a client");
    assert!(peers.steps_for(Duration::from_secs(5), |p| ready(p, c)));
    leave(&mut peers, c);
    peers.rejoin(c, player, "Elise", true).expect("joins again");
    assert!(peers.clients[c].world().resource::<ClientRoster>().own.is_none(), "forgotten");
    assert!(peers.steps_for(Duration::from_secs(5), |p| ready(p, c)), "the second connection's roster came");
    assert_eq!(peers.clients[c].world().resource::<ClientRoster>().players.len(), 1);
    assert_eq!(peers.clients[c].world().resource::<RemotePoses>().refused, None);
    leave(&mut peers, c);
    peers.rejoin(c, player, "Elise", false).expect("joins again");
    let refused =
        peers.steps_for(Duration::from_secs(5), |p| p.clients[c].world().resource::<RemotePoses>().refused.is_some());
    let why = peers.clients[c].world().resource::<RemotePoses>().refused.clone();
    assert!(refused && why.as_deref().is_some_and(|w| w.contains("twice")), "{why:?}");
}
