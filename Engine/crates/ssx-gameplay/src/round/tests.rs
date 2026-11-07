use super::*;
use ssx_formats::ghost::{POSE_BLOCK_SIZE, PoseBlock};
use ssx_net_proto::{Appearance, ClientControl, GhostStyle, InstanceSpec, NetPose, PlayerStatus, PoseFlags, Rewinds};

const RULES: CycleRules =
    CycleRules { min_players: 2, lobby_wait: 30, load_timeout: 300, countdown: 90, round_limit: 3_000, results: 60 };

fn block() -> PoseBlock {
    let mut b = [0u8; POSE_BLOCK_SIZE];
    b[12..16].copy_from_slice(&1.0f32.to_be_bytes());
    b[0x48..0x4C].copy_from_slice(&1.0f32.to_be_bytes());
    PoseBlock::parse(&b).expect("240 bytes parse")
}

fn ready(round: u16) -> ClientControl {
    let sample = NetPose::new(0, PoseFlags::default(), block()).expect("a sample");
    let second = Box::new(sample.clone().with_execution(2).expect("an execution"));
    ClientControl::Ready { round, sample: Box::new(sample), second }
}

fn instance(mode: Mode, rotation: &[&str]) -> Instance {
    let spec = InstanceSpec::new(rotation[0], mode, Rewinds::On, false, GhostStyle::Character).expect("spec");
    let rotation = rotation.iter().map(|d| ssx_net_proto::instance::Level::drop(d).expect("a drop")).collect();
    Instance::rounds(5, spec, 8, RULES, rotation, 0)
}

fn look() -> Appearance {
    Appearance::new(0x405D_C1D5, 0).unwrap()
}

fn send(i: &mut Instance, rider: u16, m: ClientControl, now: u32) -> Result<Accepted, Refused> {
    i.control(rider, &m, now)
}

fn reported(r: ProgressReport) -> ClientControl {
    ClientControl::Report(Box::new(r))
}

fn report(kind: ReportKind, split: Option<u8>, clock_ms: u32, time: f64) -> ProgressReport {
    let execution = (f64::from(clock_ms) * 30.0 / 1000.0).ceil() as u32 + 21;
    report_at(kind, split, clock_ms, time, execution)
}

fn report_at(kind: ReportKind, split: Option<u8>, clock_ms: u32, time: f64, execution: u32) -> ProgressReport {
    ProgressReport {
        instance: 5,
        round: 0,
        kind,
        split,
        clock_ms,
        points: 0,
        descent_mm: 0,
        rewinds: 0,
        loops: 0,
        sample: NetPose::at(time, PoseFlags::default(), block())
            .and_then(|p| p.with_execution(execution))
            .expect("a sample"),
    }
}

fn until(i: &mut Instance, from: u32, limit: u32, want: InstanceState) -> (u32, Vec<Changes>) {
    let mut changes = Vec::new();
    for now in from..from + limit {
        changes.push(i.tick(now));
        if i.state() == want {
            return (now, changes);
        }
    }
    panic!("{want:?} not reached by tick {}: {:?}", from + limit, i.state());
}

fn running(mode: Mode) -> (Instance, u32) {
    let mut i = instance(mode, &["exp_alps_vorassay_drop_3", "exp_rockies_x_drop_1"]);
    for r in [1, 2, 3] {
        i.admit(r).unwrap();
        send(&mut i, r, ClientControl::Appearance(look()), 0).unwrap();
        send(&mut i, r, ClientControl::Loaded { round: 0 }, 0).unwrap();
    }
    assert_eq!(i.state(), InstanceState::Lobby);
    let (at, _) = until(&mut i, 1, 100, InstanceState::Loading);
    assert_eq!(at, 1 + RULES.lobby_wait, "the lobby waits its time with enough players");
    let (at, changes) = until(&mut i, at + 1, 10, InstanceState::Countdown);
    let (round, t, riders) = changes.last().and_then(|c| c.start.clone()).expect("the start");
    assert_eq!((round, t, riders), (0, at + RULES.countdown, vec![1, 2, 3]));
    for r in [1, 2, 3] {
        assert_eq!(send(&mut i, r, ready(0), t - 1).unwrap_err().rule, Refusal::EarlyStart);
        send(&mut i, r, ClientControl::StartAck { round: 0 }, at + 2).unwrap();
    }
    assert_eq!(send(&mut i, 1, ClientControl::StartAck { round: 0 }, at + 3).unwrap_err().rule, Refusal::Order);
    let (start, _) = until(&mut i, at + 1, RULES.countdown + 1, InstanceState::Running);
    assert_eq!(start, t);
    for r in [1, 2, 3] {
        send(&mut i, r, ready(0), t + 40).unwrap();
    }
    (i, t)
}

#[test]
fn a_race_round_orders_its_riders_by_their_own_clocks_and_rotates() {
    let (mut i, t) = running(Mode::Race);
    let at = |s: f64| f64::from(t) + 40.0 + s * 30.0;
    for r in [1, 2, 3] {
        for k in 0..9u8 {
            let s = f64::from(k + 1) * 5.0;
            let clock = (s * 1000.0) as u32 + u32::from(r) * 100;
            send(&mut i, r, reported(report(ReportKind::Split, Some(k), clock, at(s))), at(s) as u32 + 2)
                .unwrap_or_else(|e| panic!("rider {r} split {k}: {e}"));
        }
    }
    let finish = [(1u16, 61_000u32, 61.0), (2, 60_500, 61.5), (3, 60_200, 62.0)];
    for (r, clock, s) in finish {
        send(&mut i, r, reported(report(ReportKind::Finish, None, clock, at(s))), at(s) as u32 + 2).unwrap();
    }
    assert_eq!(i.status(3), Some(PlayerStatus::Finished));
    let (end, _) = until(&mut i, at(62.0) as u32 + 2, 5, InstanceState::Results);
    let order: Vec<u16> = i.standings().iter().map(|r| r.rider).collect();
    assert_eq!(order, [3, 2, 1], "the owners' clocks, not the order of their finishes");
    assert_eq!(crate::standings::medal(i.standings(), 0), Some(2));
    let late = report(ReportKind::Finish, None, 70_000, f64::from(end));
    assert!(send(&mut i, 1, ClientControl::Report(Box::new(late)), end + 1).unwrap().stale);
    let (lobby, changes) = until(&mut i, end + 1, RULES.results + 2, InstanceState::Lobby);
    assert!(changes.last().is_some_and(|c| c.state && c.standings && c.roles));
    assert_eq!((i.round(), i.spec().level.name()), (1, "exp_rockies_x_drop_1"));
    assert!(i.standings().is_empty());
    for r in [1, 2, 3] {
        assert_eq!(i.progress(r), Some(Progress::Dressed(look())));
        assert!(send(&mut i, r, ready(0), lobby).unwrap().stale);
        assert_eq!(send(&mut i, r, ClientControl::Loaded { round: 2 }, lobby).unwrap_err().rule, Refusal::FutureRound);
        send(&mut i, r, ClientControl::Loaded { round: 1 }, lobby).unwrap();
    }
}

#[test]
fn hostile_reports_are_refused_with_their_rule() {
    let (mut i, t) = running(Mode::Race);
    let at = |s: f64| f64::from(t) + 40.0 + s * 30.0;
    let now = at(20.0) as u32;
    let refused =
        |i: &mut Instance, r: ProgressReport| send(i, 1, ClientControl::Report(Box::new(r)), now).unwrap_err().rule;
    send(&mut i, 1, reported(report(ReportKind::Split, Some(3), 15_000, at(15.0))), now).unwrap();
    assert_eq!(refused(&mut i, report(ReportKind::Split, Some(3), 16_000, at(16.0))), Refusal::ReportOrder);
    assert_eq!(refused(&mut i, report(ReportKind::Split, Some(1), 16_000, at(16.0))), Refusal::ReportOrder);
    assert_eq!(refused(&mut i, report(ReportKind::Split, Some(4), 14_000, at(16.0))), Refusal::ReportOrder);
    let executions = 20 * 30 + 40;
    let edge = ((f64::from(executions) + REPORT_CLOCK_SLACK_TICKS) * 1000.0 / 30.0) as u32;
    let beyond = report_at(ReportKind::Split, Some(4), edge + 2, at(20.0), executions);
    assert_eq!(refused(&mut i, beyond), Refusal::ReportClock);
    send(&mut i, 1, reported(report_at(ReportKind::Split, Some(4), edge - 1, at(20.0), executions)), now).unwrap();
    assert_eq!(refused(&mut i, report(ReportKind::Points, None, edge, at(20.0))), Refusal::ReportOrder);
    let mut other = report(ReportKind::Split, Some(6), edge, at(20.0));
    other.instance = 6;
    assert_eq!(refused(&mut i, other), Refusal::ReportInstance);
    let mut future = report(ReportKind::Split, Some(6), edge, at(20.0));
    future.round = 1;
    assert_eq!(refused(&mut i, future), Refusal::FutureRound);
    send(&mut i, 1, reported(report(ReportKind::Finish, None, edge, at(20.0))), now).unwrap();
    assert_eq!(refused(&mut i, report(ReportKind::Finish, None, edge, at(20.0))), Refusal::ReportOrder);
    assert_eq!(refused(&mut i, report(ReportKind::Split, Some(8), edge, at(20.0))), Refusal::ReportOrder);
    i.admit(9).unwrap();
    send(&mut i, 9, ClientControl::Appearance(look()), now).unwrap();
    send(&mut i, 9, ClientControl::Loaded { round: 0 }, now).unwrap();
    assert_eq!(i.status(9), Some(PlayerStatus::Spectating));
    assert_eq!(i.relay_roles(9), (false, true), "a spectator takes the others' samples, sends none");
    assert_eq!(send(&mut i, 9, ready(0), now).unwrap_err().rule, Refusal::Spectator);
    assert_eq!(send(&mut i, 9, ClientControl::StartAck { round: 0 }, now).unwrap_err().rule, Refusal::Spectator);
    let r = reported(report(ReportKind::Split, Some(0), 1_000, at(1.0)));
    assert_eq!(send(&mut i, 9, r, now).unwrap_err().rule, Refusal::Spectator);
    send(&mut i, 2, reported(report(ReportKind::Split, Some(0), 5_000, at(5.0))), now).unwrap();
}

#[test]
fn the_limit_the_leavers_and_a_round_without_enough_riders() {
    let mut i = instance(Mode::Trick, &["exp_himwest_makalu_drop_1_trick"]);
    for r in [1, 2] {
        i.admit(r).unwrap();
        send(&mut i, r, ClientControl::Appearance(look()), 0).unwrap();
    }
    send(&mut i, 1, ClientControl::Loaded { round: 0 }, 0).unwrap();
    let (at, _) = until(&mut i, 1, 100, InstanceState::Loading);
    let (back, _) = until(&mut i, at + 1, RULES.load_timeout + 1, InstanceState::Lobby);
    assert_eq!(back, at + RULES.load_timeout);
    assert_eq!(i.status(2), Some(PlayerStatus::Loading), "no spectator after the round did not form");
    send(&mut i, 2, ClientControl::Loaded { round: 0 }, back).unwrap();
    let (at, _) = until(&mut i, back + 1, 100, InstanceState::Countdown);
    let t = i.start_tick().expect("the start");
    let (_, _) = until(&mut i, at + 1, RULES.countdown + 1, InstanceState::Running);
    let r = report(ReportKind::Points, None, 1_000, f64::from(t) + 30.0);
    assert_eq!(
        send(&mut i, 1, ClientControl::Report(Box::new(r.clone())), t + 31).unwrap_err().rule,
        Refusal::ReportEarly
    );
    send(&mut i, 1, ready(0), t + 40).unwrap();
    send(&mut i, 1, ClientControl::Report(Box::new(r)), t + 41).unwrap();
    assert!(i.leave(2));
    i.tick(t + 42);
    let rows = i.standings();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].state, StandingState::Left);
    let (end, _) = until(&mut i, t + 43, RULES.round_limit, InstanceState::Results);
    assert_eq!(end, t + RULES.round_limit);
    assert_eq!(i.standings()[0].state, StandingState::Racing, "no result without the finish");
}

#[test]
fn a_survive_runs_splits_start_over_with_its_loops() {
    let (mut i, t) = running(Mode::Survive);
    let at = |s: f64| f64::from(t) + 40.0 + s * 30.0;
    let looped = |k: u8, loops: u16, s: f64| {
        let mut r = report(ReportKind::Split, Some(k), (s * 1000.0) as u32, at(s));
        r.loops = loops;
        r
    };
    for (k, loops, s) in [(7u8, 0u16, 10.0), (8, 0, 12.0), (0, 1, 20.0), (1, 1, 22.0)] {
        send(&mut i, 1, ClientControl::Report(Box::new(looped(k, loops, s))), at(s) as u32 + 1).unwrap();
    }
    assert_eq!(
        send(&mut i, 1, ClientControl::Report(Box::new(looped(2, 0, 24.0))), at(24.0) as u32 + 1).unwrap_err().rule,
        Refusal::ReportOrder,
        "the loops never go back"
    );
    assert_eq!(
        send(&mut i, 1, ClientControl::Report(Box::new(looped(1, 1, 25.0))), at(25.0) as u32 + 1).unwrap_err().rule,
        Refusal::ReportOrder,
        "a split of this loop not after its last"
    );
}

#[test]
fn a_survive_to_death_loop_starts_its_clock_again() {
    let (mut i, t) = running(Mode::Survive);
    let at = |s: f64| f64::from(t) + 40.0 + s * 30.0;
    let looped = |k: u8, loops: u16, s: f64, clock_ms: u32| {
        let mut r = report_at(ReportKind::Split, Some(k), clock_ms, at(s), (s * 30.0) as u32 + 22);
        r.loops = loops;
        r
    };
    for (k, loops, s, ms) in [(7u8, 0u16, 10.0, 10_000), (8, 0, 12.0, 12_000), (0, 1, 20.0, 1_500), (1, 1, 22.0, 3_500)]
    {
        if let Err(e) = send(&mut i, 1, reported(looped(k, loops, s, ms)), at(s) as u32 + 1) {
            panic!("split {k} of loop {loops}: {e:?}");
        }
    }
    assert_eq!(
        send(&mut i, 1, reported(looped(2, 1, 24.0, 3_000)), at(24.0) as u32 + 1).unwrap_err().rule,
        Refusal::ReportOrder,
        "the clock goes back within a loop"
    );
}

#[test]
fn the_rounds_limit_ends_a_survive_to_death_run_with_its_result() {
    for results in [true, false] {
        let (mut i, t) = running(Mode::Survive);
        i.set_results_at_limit(results);
        let mut alive = report(ReportKind::Split, Some(3), 20_000, f64::from(t) + 640.0);
        alive.descent_mm = 250_000;
        send(&mut i, 1, reported(alive), t + 650).unwrap();
        let mut died = report(ReportKind::Finish, None, 15_000, f64::from(t) + 490.0);
        died.descent_mm = 100_000;
        send(&mut i, 2, reported(died), t + 500).unwrap();
        assert!(i.leave(3));
        let (end, _) = until(&mut i, t + 651, RULES.round_limit, InstanceState::Results);
        assert_eq!(end, t + RULES.round_limit);
        let rows = i.standings();
        let states: Vec<_> = rows.iter().map(|r| (r.rider, r.state)).collect();
        if results {
            assert_eq!(states[..2], [(1, StandingState::Finished), (2, StandingState::Finished)], "{states:?}");
            assert_eq!(crate::standings::medal(rows, 0), Some(2), "gold for the survivor");
        } else {
            assert_eq!(states[..2], [(2, StandingState::Finished), (1, StandingState::Racing)], "{states:?}");
            assert_eq!(crate::standings::medal(rows, 1), None, "no result without its end");
        }
    }
}

#[test]
fn a_races_failed_runs_keep_their_furthest_split() {
    let (mut i, t) = running(Mode::Race);
    let at = |s: f64| f64::from(t) + 40.0 + s * 30.0;
    let now = |s: f64| at(s) as u32 + 2;
    send(&mut i, 1, reported(report(ReportKind::Split, Some(4), 30_000, at(30.0))), now(30.0)).unwrap();
    send(&mut i, 2, reported(report(ReportKind::Split, Some(4), 29_000, at(29.0))), now(30.0)).unwrap();
    send(&mut i, 1, reported(report(ReportKind::Failed, None, 31_000, at(31.0))), now(31.0)).unwrap();
    send(&mut i, 2, reported(report(ReportKind::Failed, None, 50_000, at(50.0))), now(50.0)).unwrap();
    send(&mut i, 3, reported(report(ReportKind::Finish, None, 70_000, at(70.0))), now(70.0)).unwrap();
    until(&mut i, now(70.0), 5, InstanceState::Results);
    let rows = i.standings();
    let order: Vec<(u16, StandingState, u32)> = rows.iter().map(|r| (r.rider, r.state, r.clock_ms)).collect();
    assert_eq!(
        order,
        [(3, StandingState::Finished, 70_000), (2, StandingState::Failed, 29_000), (1, StandingState::Failed, 30_000)]
    );
    assert_eq!(crate::standings::medal(rows, 1), None);
}

#[test]
fn the_stop_ends_a_running_round_by_its_deadline_and_keeps_its_results() {
    use super::stop::StopPlan;
    let (mut i, t) = running(Mode::Race);
    let now = t + 50;
    let plan = i.stop(now, 60);
    assert_eq!(plan, StopPlan::RoundEnds { round: 0, by: now + 60 });
    assert_eq!(i.stop(now + 5, 600), plan, "a second stop keeps the first deadline");
    assert!(!i.tick(now + 59).state);
    assert_eq!(i.state(), InstanceState::Running);
    assert!(i.tick(now + 60).state);
    assert_eq!(i.state(), InstanceState::Results);
    assert!(i.standings().iter().all(|r| r.state == StandingState::Racing), "no run finished: no result");
    assert!(!i.stop_done(now + 60, 90), "the results not shown yet");
    until_none(&mut i, now + 61, RULES.results + 30);
    assert_eq!((i.state(), i.round()), (InstanceState::Results, 0));
    assert!(i.stop_done(now + 60 + 90, 90));
}

#[test]
fn the_stop_lets_a_round_whose_runs_are_over_end_before_its_deadline() {
    let (mut i, t) = running(Mode::Race);
    let at = |s: f64| f64::from(t) + 40.0 + s * 30.0;
    i.stop(t + 41, 3_000);
    for (r, clock, s) in [(1u16, 9_000u32, 9.5), (2, 9_100, 9.6), (3, 9_200, 9.7)] {
        send(&mut i, r, reported(report(ReportKind::Finish, None, clock, at(s))), at(s) as u32 + 2).unwrap();
    }
    let (end, _) = until(&mut i, at(9.7) as u32 + 2, 5, InstanceState::Results);
    assert!(end < t + 41 + 3_000);
    assert_eq!(i.standings().iter().map(|r| r.rider).collect::<Vec<_>>(), [1, 2, 3]);
    assert!(i.standings().iter().all(|r| r.state == StandingState::Finished));
}

#[test]
fn no_round_forms_after_the_stop() {
    use super::stop::StopPlan;
    let mut i = instance(Mode::Race, &["exp_alps_vorassay_drop_3"]);
    for r in [1, 2] {
        i.admit(r).unwrap();
        send(&mut i, r, ClientControl::Appearance(look()), 0).unwrap();
        send(&mut i, r, ClientControl::Loaded { round: 0 }, 0).unwrap();
    }
    assert_eq!(i.stop(0, 60), StopPlan::Idle);
    assert!(i.stop_done(0, 90), "nothing under way");
    until_none(&mut i, 1, RULES.lobby_wait + RULES.load_timeout + 10);
    assert_eq!(i.state(), InstanceState::Lobby);
    let (running, t) = running(Mode::Race);
    assert!(!running.stop_done(t, 0));
    let spec = InstanceSpec::new("exp_alps_vorassay_drop_3", Mode::Freeride, Rewinds::On, false, GhostStyle::Character)
        .expect("spec");
    let mut free = Instance::freeride(2, spec, 8);
    assert_eq!(free.stop(0, 60), StopPlan::Idle);
    assert!(free.stop_done(0, 0));
}

#[test]
fn the_stop_ends_a_survive_to_death_run_with_its_result_as_the_rounds_limit_does() {
    for results in [true, false] {
        let (mut i, t) = running(Mode::Survive);
        i.set_results_at_limit(results);
        let mut alive = report(ReportKind::Split, Some(3), 20_000, f64::from(t) + 640.0);
        alive.descent_mm = 250_000;
        send(&mut i, 1, reported(alive), t + 650).unwrap();
        let mut died = report(ReportKind::Finish, None, 15_000, f64::from(t) + 490.0);
        died.descent_mm = 100_000;
        send(&mut i, 2, reported(died), t + 500).unwrap();
        assert!(i.leave(3));
        i.stop(t + 651, 60);
        let (end, _) = until(&mut i, t + 652, 60, InstanceState::Results);
        assert_eq!(end, t + 651 + 60, "the stop's deadline, not the round's limit");
        let rows = i.standings();
        let states: Vec<_> = rows.iter().map(|r| (r.rider, r.state)).collect();
        if results {
            assert_eq!(states[..2], [(1, StandingState::Finished), (2, StandingState::Finished)], "{states:?}");
            assert_eq!(crate::standings::medal(rows, 0), Some(2), "gold for the survivor");
        } else {
            assert_eq!(states[..2], [(2, StandingState::Finished), (1, StandingState::Racing)], "{states:?}");
        }
    }
}

fn until_none(i: &mut Instance, from: u32, count: u32) {
    for now in from..from + count {
        assert!(i.tick(now).start.is_none(), "a round formed at tick {now}");
    }
}
