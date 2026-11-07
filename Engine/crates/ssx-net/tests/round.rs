use core::time::Duration;

use ssx_gameplay::instance::Refusal;
use ssx_gameplay::round::CycleRules;
use ssx_net::harness::round::{self, ROTATION};
use ssx_net::server::{KickReason, ServerStats};
use ssx_net_proto::{ClientControl, InstanceState, Mode, ReportKind, StandingState};

const RULES: CycleRules =
    CycleRules { min_players: 2, lobby_wait: 0, load_timeout: 300, countdown: 90, round_limit: 1_800, results: 60 };

const JUMP: f64 = 45.0;

fn to_the_start(peers: &mut ssx_net::harness::Peers, riders: &[usize]) -> (u16, u32) {
    for &c in riders {
        round::send(peers, c, [ClientControl::Appearance(round::look()), ClientControl::Loaded { round: 0 }]);
    }
    let told = |p: &mut ssx_net::harness::Peers| riders.iter().all(|&c| round::start(p, c).is_some());
    assert!(peers.steps_for(Duration::from_secs(20), told), "every rider is told the start");
    let start = round::start(peers, riders[0]).expect("the start");
    assert!(riders.iter().all(|&c| round::start(peers, c) == Some(start)), "one start for all");
    start
}

fn until_time(peers: &mut ssx_net::harness::Peers, c: usize, t: f64) {
    let reached = |p: &mut ssx_net::harness::Peers| round::server_time(p, c).is_some_and(|now| now >= t);
    assert!(peers.steps_for(Duration::from_secs(30), reached), "client {c} reaches tick {t}");
}

#[test]
fn a_race_round_orders_the_riders_by_their_own_clocks_and_rotates() {
    let mut peers = round::riders(3, Mode::Race, RULES, None).expect("three riders");
    let (r, t) = to_the_start(&mut peers, &[0, 1, 2]);
    assert_eq!(r, 0);
    for c in 0..3 {
        round::send(&mut peers, c, [ClientControl::StartAck { round: r }]);
    }
    until_time(&mut peers, 0, f64::from(t) + JUMP);
    assert_eq!(round::instance(&peers, 0).map(|i| i.0), Some(InstanceState::Running));
    for c in 0..3 {
        round::send(&mut peers, c, [ssx_net::harness::ready(r)]);
    }
    for k in 0..9u8 {
        until_time(&mut peers, 0, f64::from(t) + JUMP + 15.0 * f64::from(k + 1));
        for c in 0..3 {
            let clock = round::clock(&peers, c, t, 1_600 + 100 * c as u32);
            let rep = round::report(&peers, c, r, ReportKind::Split, Some(k), clock);
            round::send(&mut peers, c, [ClientControl::Report(Box::new(rep))]);
        }
    }
    until_time(&mut peers, 0, f64::from(t) + JUMP + 150.0);
    for (c, behind) in [(1, 1_500), (0, 1_700), (2, 1_900)] {
        let clock = round::clock(&peers, c, t, behind);
        let rep = round::report(&peers, c, r, ReportKind::Finish, None, clock);
        round::send(&mut peers, c, [ClientControl::Report(Box::new(rep))]);
    }
    let results = |p: &mut ssx_net::harness::Peers| {
        (0..3).all(|c| round::instance(p, c).is_some_and(|i| i.0 == InstanceState::Results))
    };
    assert!(peers.steps_for(Duration::from_secs(5), results), "every finish in: the results");
    peers.run_until(Duration::from_millis(500), |_| false);
    let riders: Vec<u16> = (0..3).map(|c| round::rider(&peers, c).expect("a rider number")).collect();
    for c in 0..3 {
        let (round, rows) = round::standings(&peers, c).expect("the standings");
        assert_eq!(round, r);
        let order: Vec<u16> = rows.iter().map(|row| row.rider).collect();
        assert_eq!(order, [riders[2], riders[0], riders[1]], "client {c}: the riders' own clocks decide");
        assert!(rows.iter().all(|row| row.state == StandingState::Finished));
    }
    let next = |p: &mut ssx_net::harness::Peers| {
        (0..3).all(|c| round::instance(p, c).is_some_and(|i| i.1 == 1 && i.2 == ROTATION[1]))
    };
    assert!(peers.steps_for(Duration::from_secs(10), next), "round 1 on the rotation's next drop");
    let loading = |p: &mut ssx_net::harness::Peers| {
        round::instance(p, 0).is_some_and(|i| (i.0, i.1) == (InstanceState::Loading, 1))
    };
    assert!(peers.steps_for(Duration::from_secs(5), loading), "round 1 loads its drop");
    peers.run_until(Duration::from_millis(300), |_| false);
    assert!(round::standings(&peers, 0).is_some_and(|(round, rows)| round == 1 && rows.is_empty()));
    let stats = peers.server.world().resource::<ServerStats>();
    assert_eq!((stats.kicked, stats.starts_sent), (0, 3), "{stats:?}");
}

#[test]
fn hostile_round_messages_are_kicked_with_their_rule() {
    let mut peers = round::riders(8, Mode::Race, RULES, None).expect("eight riders");
    let (r, t) = to_the_start(&mut peers, &(0..8).collect::<Vec<_>>());
    round::send(&mut peers, 0, [ssx_net::harness::ready(r)]);
    let early = round::report(&peers, 1, r, ReportKind::Finish, None, 0);
    round::send(&mut peers, 1, [ClientControl::Report(Box::new(early))]);
    until_time(&mut peers, 2, f64::from(t) + JUMP);
    for c in 2..8 {
        round::send(&mut peers, c, [ssx_net::harness::ready(r)]);
    }
    until_time(&mut peers, 2, f64::from(t) + JUMP + 60.0);
    let beyond = round::clock(&peers, 2, t, 0) + 5_000;
    let rep = |p: &ssx_net::harness::Peers, c: usize, kind, split| {
        let clock = round::clock(p, c, t, 1_600);
        round::report(p, c, r, kind, split, clock)
    };
    let mut clock_beyond = rep(&peers, 2, ReportKind::Split, Some(0));
    clock_beyond.clock_ms = beyond;
    round::send(&mut peers, 2, [ClientControl::Report(Box::new(clock_beyond))]);
    let (s3, s1) = (rep(&peers, 3, ReportKind::Split, Some(3)), rep(&peers, 3, ReportKind::Split, Some(1)));
    round::send(&mut peers, 3, [ClientControl::Report(Box::new(s3)), ClientControl::Report(Box::new(s1))]);
    let mut other = rep(&peers, 4, ReportKind::Split, Some(0));
    other.instance = 99;
    round::send(&mut peers, 4, [ClientControl::Report(Box::new(other))]);
    let mut future = rep(&peers, 5, ReportKind::Split, Some(0));
    future.round = r + 1;
    round::send(&mut peers, 5, [ClientControl::Report(Box::new(future))]);
    let (f1, f2) = (rep(&peers, 6, ReportKind::Finish, None), rep(&peers, 6, ReportKind::Finish, None));
    round::send(&mut peers, 6, [ClientControl::Report(Box::new(f1)), ClientControl::Report(Box::new(f2))]);
    let honest = rep(&peers, 7, ReportKind::Split, Some(0));
    round::send(&mut peers, 7, [ClientControl::Report(Box::new(honest))]);
    let late = peers.add_client(99).expect("a newcomer");
    let placed =
        |p: &mut ssx_net::harness::Peers| p.connected(late) && p.synced(late) && round::rider(p, late).is_some();
    assert!(peers.steps_for(Duration::from_secs(10), placed), "the newcomer is placed");
    round::send(&mut peers, late, [ClientControl::Appearance(round::look()), ClientControl::Loaded { round: r }]);
    peers.run_until(Duration::from_millis(500), |_| false);
    let watching = round::report(&peers, late, r, ReportKind::Split, Some(0), 1_000);
    round::send(&mut peers, late, [ClientControl::Report(Box::new(watching))]);
    let all = |p: &mut ssx_net::harness::Peers| p.server.world().resource::<ServerStats>().kicked >= 8;
    assert!(peers.steps_for(Duration::from_secs(5), all), "eight kicks");
    peers.run_until(Duration::from_millis(500), |_| false);
    let stats = peers.server.world().resource::<ServerStats>().clone();
    let mut kicks: Vec<KickReason> = stats.kick_reasons.iter().copied().collect();
    kicks.sort_by_key(|k| format!("{k:?}"));
    let mut want = vec![
        KickReason::Round(Refusal::EarlyStart),
        KickReason::Round(Refusal::ReportEarly),
        KickReason::Round(Refusal::ReportClock),
        KickReason::Round(Refusal::ReportOrder),
        KickReason::Round(Refusal::ReportInstance),
        KickReason::Round(Refusal::FutureRound),
        KickReason::Round(Refusal::ReportOrder),
        KickReason::Round(Refusal::Spectator),
    ];
    want.sort_by_key(|k| format!("{k:?}"));
    assert_eq!(kicks, want, "{:?}", stats.refusals);
    for (_, detail) in &stats.refusals {
        println!("refused: {detail}");
    }
    assert!(peers.connected(7), "the honest rider rides on");
    let finish = rep(&peers, 7, ReportKind::Finish, None);
    round::send(&mut peers, 7, [ClientControl::Report(Box::new(finish))]);
    let over = |p: &mut ssx_net::harness::Peers| round::instance(p, 7).is_some_and(|i| i.0 == InstanceState::Results);
    assert!(peers.steps_for(Duration::from_secs(5), over), "the round ends: every other rider left");
    peers.run_until(Duration::from_millis(300), |_| false);
    let (_, rows) = round::standings(&peers, 7).expect("the standings");
    assert_eq!(rows.len(), 8, "every rider of the round has a row");
    let six =
        rows.iter().find(|row| row.state == StandingState::Finished && row.rider != round::rider(&peers, 7).unwrap());
    assert!(six.is_some(), "rider 6's first finish stays: {rows:?}");
    assert_eq!(rows[1].rider, round::rider(&peers, 7).expect("rider"), "the honest rider's later clock: second");
    assert_eq!(rows[1].state, StandingState::Finished);
    assert!(rows[2..].iter().all(|row| row.state == StandingState::Left), "{rows:?}");
}

#[test]
fn reports_the_wire_refuses_end_the_connection() {
    let mut peers = round::riders(5, Mode::Race, RULES, None).expect("five riders");
    for c in 0..5 {
        round::send(&mut peers, c, [ClientControl::Appearance(round::look()), ClientControl::Loaded { round: 0 }]);
    }
    peers.run_until(Duration::from_millis(500), |_| false);
    let encoded = |p: &ssx_net::harness::Peers, c: usize, kind, split| {
        ClientControl::Report(Box::new(round::report(p, c, 0, kind, split, 1_000))).encode()
    };
    let at = |field: usize| 3 + field;
    let mut split_nine = encoded(&peers, 0, ReportKind::Split, Some(8));
    split_nine[at(5)] = 9;
    let mut finish_split = encoded(&peers, 1, ReportKind::Finish, None);
    finish_split[at(5)] = 2;
    let mut clock = encoded(&peers, 2, ReportKind::Split, Some(0));
    clock[at(6)..at(10)].copy_from_slice(&3_600_000u32.to_be_bytes());
    let mut kind = encoded(&peers, 3, ReportKind::Split, Some(0));
    kind[at(4)] = 7;
    for (c, bytes) in [split_nine, finish_split, clock, kind].into_iter().enumerate() {
        round::send_raw(&mut peers, c, bytes).expect("sent");
    }
    let four = |p: &mut ssx_net::harness::Peers| p.server.world().resource::<ServerStats>().kicked >= 4;
    assert!(peers.steps_for(Duration::from_secs(5), four), "four kicks");
    peers.run_until(Duration::from_millis(300), |_| false);
    let stats = peers.server.world().resource::<ServerStats>().clone();
    assert_eq!(stats.kick_reasons.iter().copied().collect::<Vec<_>>(), vec![KickReason::MalformedControl; 4]);
    assert!(peers.connected(4), "the fifth rider stays");
}
