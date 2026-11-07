use ssx_net::harness::instance::{InstanceCase, InstanceOutcome, run_instance};
use ssx_net::server::{KickReason, PoseRule};

fn run(case: InstanceCase) -> InstanceOutcome {
    let o = run_instance(case).unwrap_or_else(|e| panic!("{case:?}: {e}"));
    println!("{case:?}: {o:?}");
    o
}

fn kicked(case: InstanceCase, reason: KickReason) {
    let o = run(case);
    assert_eq!(o.kicks, vec![reason], "{case:?}: {o:?}");
    assert!(!o.rider0_connected && o.rider1_connected, "{case:?}: {o:?}");
}

fn refused(case: InstanceCase, why: &str) -> InstanceOutcome {
    let o = run(case);
    let refusal = o.refusal.clone().unwrap_or_default();
    assert!(refusal.contains(why), "{case:?}: the reason names {why:?}: {o:?}");
    assert!(!o.rider0_connected, "{case:?}: rider 0 left: {o:?}");
    assert!(o.kicks.is_empty(), "{case:?}: the server ended nobody: {o:?}");
    o
}

#[test]
fn control_messages_out_of_order_or_malformed_are_kicked() {
    kicked(InstanceCase::ControlOutOfOrder, KickReason::ControlOrder);
    kicked(InstanceCase::MalformedControl, KickReason::MalformedControl);
}

#[test]
fn samples_no_ride_produces_are_kicked() {
    kicked(InstanceCase::AbsurdCoordinate, KickReason::AbsurdPose(PoseRule::Coordinate));
    kicked(InstanceCase::AbsurdRotation, KickReason::AbsurdPose(PoseRule::RotationLength));
    kicked(InstanceCase::AbsurdSpeed, KickReason::AbsurdPose(PoseRule::Speed));
    kicked(InstanceCase::AbsurdStep, KickReason::AbsurdPose(PoseRule::Step));
}

#[test]
fn a_connection_that_never_acknowledges_is_kicked() {
    let o = run(InstanceCase::NeverAcknowledges);
    assert_eq!(o.kicks, vec![KickReason::Unacknowledged], "{o:?}");
    assert!(o.rider1_connected && o.roster == 1, "rider 0 is gone from the server, rider 1 stays: {o:?}");
}

#[test]
fn hostile_relayed_samples_make_the_client_leave() {
    refused(InstanceCase::MalformedRelayed, "malformed sample");
    refused(InstanceCase::OversizedRelayed, "malformed sample");
    refused(InstanceCase::NanRelayed, "malformed sample");
    refused(InstanceCase::AbsurdRelayed, "relayed a sample of rider 1");
}

#[test]
fn a_sample_of_a_rider_not_on_the_roster_is_dropped() {
    let o = run(InstanceCase::UnknownRacer);
    assert_eq!(o.unknown_samples, 1, "{o:?}");
    assert!(o.rider0_connected && o.rider1_connected && o.refusal.is_none(), "{o:?}");
}

#[test]
fn player_names_against_the_rule_make_the_clients_leave() {
    refused(InstanceCase::NameWithControls, "replication data does not apply");
    refused(InstanceCase::LongName, "replication data does not apply");
}

#[test]
fn a_flood_of_players_stops_at_the_fragment_limit_or_the_entity_cap() {
    refused(InstanceCase::PlayerFloodOneFrame, "more than the limit 16384");
    let o = refused(InstanceCase::PlayerFlood, "replication data does not apply");
    let refusal = o.refusal.unwrap_or_default();
    assert!(refusal.contains("entity") && refusal.contains("limit"), "the entity cap: {refusal}");
}

#[test]
fn players_joining_and_leaving_every_frame_are_followed() {
    let o = run(InstanceCase::PlayerChurn);
    assert!(o.rider0_connected && o.rider1_connected && o.refusal.is_none(), "{o:?}");
    assert_eq!(o.roster, 2, "the churned players are gone again: {o:?}");
}

#[test]
fn a_server_that_stops_answering_ends_by_the_timeout() {
    let o = run(InstanceCase::SilentServer);
    assert!(!o.rider0_connected, "{o:?}");
    assert!(o.notes.iter().any(|n| n.contains("disconnected")), "{o:?}");
}

mod common;

#[test]
fn a_client_keeps_back_a_sample_the_server_refuses() {
    use core::time::Duration;
    use ssx_net::harness::Io;
    use ssx_net::{PoseOutbox, ServerStats};
    let mut peers = common::connected(Io::Crossbeam, 2, None);
    let tick = common::server_tick(&peers);
    let mut block = common::block(5);
    block.trajectory_position[0] = -36_027.0;
    let far = ssx_net_proto::NetPose::new(tick, ssx_net_proto::PoseFlags::default(), block).expect("encodable");
    peers.clients[0].world_mut().resource_mut::<PoseOutbox>().next = Some(far);
    peers.run_until(Duration::from_millis(500), |_| false);
    let outbox = peers.clients[0].world().resource::<PoseOutbox>();
    assert_eq!((outbox.implausible, outbox.sent), (1, 0), "kept back");
    assert!(outbox.implausible_why.as_deref().is_some_and(|w| w.contains("-36027")), "{:?}", outbox.implausible_why);
    peers.clients[0].world_mut().resource_mut::<PoseOutbox>().next = Some(common::pose(common::server_tick(&peers), 6));
    peers.run_until(Duration::from_millis(500), |_| false);
    assert_eq!(peers.clients[0].world().resource::<PoseOutbox>().sent, 1, "the next goes");
    assert_eq!(peers.server.world().resource::<ServerStats>().kicked, 0);
}

#[test]
fn a_change_of_look_while_connected() {
    use ssx_net::harness::look::{LookCase, NEW_CHARACTER, run_look};
    use ssx_net_proto::PlayerStatus;
    let o = run_look(LookCase::ChangeOfLook).unwrap_or_else(|e| panic!("{e}"));
    println!("{o:?}");
    assert!(o.kicks.is_empty() && o.rider0_connected, "{o:?}");
    assert_eq!(o.away, Some((Some(PlayerStatus::Away), None)), "{o:?}");
    assert_eq!(o.relayed_while_away, 0, "{o:?}");
    assert!(o.dropped_before_look >= 10, "the away rider's samples dropped: {o:?}");
    assert!(o.relayed_after > 0, "{o:?}");
    let (status, look) = o.end.expect("rider 0 on rider 1's roster");
    assert_eq!(status, Some(PlayerStatus::Riding), "{o:?}");
    assert_eq!(look.map(|a| a.character), Some(NEW_CHARACTER), "{o:?}");
    for case in [LookCase::LoadedWhileAway, LookCase::LevelLeftTwice] {
        let o = run_look(case).unwrap_or_else(|e| panic!("{case:?}: {e}"));
        println!("{case:?}: {o:?}");
        assert_eq!(o.kicks, vec![KickReason::ControlOrder], "{case:?}: {o:?}");
        assert!(!o.rider0_connected, "{case:?}: {o:?}");
    }
}
