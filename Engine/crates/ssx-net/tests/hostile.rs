use ssx_net::harness::ServerKind;
use ssx_net::harness::hostile::{Case, NetcodeSizes, Outcome, run};
use ssx_net::limits::{MAX_FRAGMENTS_IN_FLIGHT, MAX_PENDING_LINKS};
use ssx_net::server::KickReason;

fn survives_on(case: Case, kind: ServerKind) -> Outcome {
    let outcome = run(case, kind).unwrap_or_else(|e| panic!("{case:?}: {e}"));
    assert!(outcome.riders_connected, "{case:?}: the riders stay connected: {outcome:?}");
    assert!(outcome.newcomer_connected, "{case:?}: a newcomer connects: {outcome:?}");
    if kind == ServerKind::Guarded {
        assert_eq!(outcome.relay_after, Some(true), "{case:?}: the relay still works: {outcome:?}");
    }
    outcome
}

fn survives(case: Case) -> Outcome {
    survives_on(case, ServerKind::Guarded)
}

fn kicked(case: Case, reasons: Vec<KickReason>) {
    let o = survives(case);
    assert_eq!(o.kicks, reasons, "{case:?}: {o:?}");
    assert_eq!(o.attacker_notified, Some(true), "{case:?}: the kicked client learns it at once: {o:?}");
}

const LINKS: usize = 3;

#[test]
fn garbage_datagrams() {
    let o = survives(Case::GarbageDatagrams);
    assert!(o.stats.links >= 300, "every source address spawned a link: {o:?}");
    assert_eq!(o.links_after_timeout, LINKS, "the unconnected links are removed: {o:?}");
    assert!(o.stats.pending_reaped >= 300, "{o:?}");
    assert_eq!(o.addresses_after_timeout, Some(LINKS), "the endpoint forgets their addresses: {o:?}");
}

#[test]
fn a_replayed_token_holds_one_pending_connection() {
    let three = NetcodeSizes { connections: 3, link_entities: 3, replay_protections: 3, send_queues: 0 };
    let o = survives(Case::TokenReplay);
    assert!(!o.attacker_connected, "{o:?}");
    assert!(o.stats.links >= 300, "every source address spawned a link: {o:?}");
    assert!(o.links_after_attack <= LINKS + MAX_PENDING_LINKS, "the cap holds: {o:?}");
    assert_eq!(o.netcode_after_attack, Some(three), "{o:?}");
    assert_eq!(o.links_after_timeout, LINKS, "{o:?}");
    assert_eq!(o.addresses_after_timeout, Some(LINKS), "{o:?}");
    assert_eq!(o.netcode_after_timeout, Some(three), "{o:?}");
    let bare = survives_on(Case::TokenReplay, ServerKind::Bare);
    assert!(bare.links_after_timeout >= 300, "{bare:?}");
    assert_eq!(bare.netcode_after_attack, Some(three), "{bare:?}");
    assert_eq!(bare.netcode_after_timeout, Some(three), "{bare:?}");
}

#[test]
fn truncated_packets() {
    let o = survives(Case::TruncatedPackets);
    assert!(o.kicks.is_empty(), "{o:?}");
}

#[test]
fn oversized_datagrams() {
    survives(Case::OversizedDatagrams);
}

#[test]
fn refused_connects() {
    for case in [Case::WrongProtocolVersion, Case::InvalidToken, Case::ExpiredToken, Case::WrongServerAddress] {
        let o = survives(case);
        assert!(!o.attacker_connected, "{case:?}: the client does not connect: {o:?}");
        assert_eq!(o.stats.connected, LINKS as u64, "{case:?}: no connection but the riders' and the newcomer's");
    }
}

#[test]
fn malformed_poses_are_kicked() {
    kicked(Case::MalformedPoses, vec![KickReason::MalformedPose; 10]);
}

#[test]
fn malformed_messages_and_packets_are_kicked() {
    kicked(Case::UnknownMessageId, vec![KickReason::MalformedMessage]);
    kicked(Case::UnknownChannel, vec![KickReason::MalformedPacket]);
    kicked(Case::GarbagePayload, vec![KickReason::MalformedPacket]);
}

#[test]
fn wrong_direction_is_kicked() {
    kicked(Case::WrongDirection, vec![KickReason::WrongDirection]);
}

#[test]
fn replication_from_a_client_is_kicked() {
    kicked(Case::ReplicationFromClient, vec![KickReason::MalformedPacket]);
    survives_on(Case::ReplicationFromClient, ServerKind::Bare);
}

#[test]
fn replication_from_the_server_is_refused_by_the_client() {
    let o = run(Case::ReplicationFromServer, ServerKind::Guarded).unwrap_or_else(|e| panic!("{e}"));
    assert!(o.riders_connected, "rider 1 stays connected: {o:?}");
    assert!(o.newcomer_connected, "{o:?}");
    assert_eq!(o.client_refusals.len(), 1, "{o:?}");
    assert!(o.client_refusals[0].contains("refused"), "{o:?}");
}

#[test]
fn fragment_bomb_is_refused() {
    kicked(Case::FragmentBomb, vec![KickReason::MalformedPacket]);
    let bare = survives_on(Case::FragmentBomb, ServerKind::Bare);
    assert_eq!(bare.max_fragment_bytes_in_flight, 0, "nothing is reserved for it: {bare:?}");
}

#[test]
fn fragment_flood_is_bounded() {
    kicked(Case::FragmentFlood, vec![KickReason::MalformedPacket]);
    let bare = survives_on(Case::FragmentFlood, ServerKind::Bare);
    assert!(bare.max_fragment_bytes_in_flight > 0, "{bare:?}");
    assert!(bare.max_fragment_bytes_in_flight <= 2 * MAX_FRAGMENTS_IN_FLIGHT, "{bare:?}");
}

#[test]
fn pose_flood_is_cut_to_the_budget() {
    let o = survives(Case::PoseFlood);
    assert!(o.kicks.is_empty(), "{o:?}");
    let accepted = o.stats.poses_received - o.stats.poses_over_rate - o.stats.poses_stale;
    assert!(accepted <= 30 + 2 + 1, "{accepted} accepted: {o:?}");
    assert!(o.stats.poses_over_rate > 0, "{o:?}");
}

#[test]
fn pose_tick_out_of_window_is_kicked() {
    kicked(Case::PoseTickOutOfWindow, vec![KickReason::PoseTick]);
}

#[test]
fn pose_view_out_of_window_is_kicked() {
    kicked(Case::PoseViewOutOfWindow, vec![KickReason::PoseView]);
}

#[test]
fn a_mismatched_server_disconnects_its_clients() {
    let o = run(Case::ProtocolMismatch, ServerKind::Guarded).unwrap_or_else(|e| panic!("{e}"));
    assert!(!o.riders_connected, "{o:?}");
    assert_eq!(o.client_refusals.len(), 2, "{o:?}");
    assert!(o.client_refusals.iter().all(|r| r.contains("protocol differs")), "{o:?}");
}

#[test]
fn pongs_of_2_pow_31_ticks_do_not_crash_the_server() {
    for kind in [ServerKind::Guarded, ServerKind::Bare] {
        let o = survives_on(Case::PongOverflow, kind);
        assert!(o.notes.iter().any(|n| !n.starts_with("0 pongs")), "the hijack answered pings: {o:?}");
    }
}

#[test]
fn replication_metadata_naming_many_senders_is_kicked() {
    kicked(Case::MetadataFromClient, vec![KickReason::Metadata]);
    let bare = survives_on(Case::MetadataFromClient, ServerKind::Bare);
    assert!(bare.notes.iter().any(|n| n.contains("Some(1000)")), "{bare:?}");
}

#[test]
fn hostile_replication_from_the_server_is_refused_without_a_crash() {
    for (case, refusal) in [
        (Case::ReplicationUnknownFnsFromServer, "is not registered"),
        (Case::ReplicationOversizeFromServer, "exceeds remaining message length"),
    ] {
        let o = run(case, ServerKind::Guarded).unwrap_or_else(|e| panic!("{case:?}: {e}"));
        assert!(o.riders_connected && o.newcomer_connected, "{case:?}: rider 1 and a newcomer: {o:?}");
        assert_eq!(o.client_refusals.len(), 1, "{case:?}: {o:?}");
        assert!(o.client_refusals[0].contains(refusal), "{case:?}: {o:?}");
    }
    let o = run(Case::MetadataOverflowFromServer, ServerKind::Guarded).unwrap_or_else(|e| panic!("{e}"));
    assert!(o.riders_connected && o.newcomer_connected, "{o:?}");
}
