use std::sync::Arc;

use glam::{Quat, Vec3};
use ssx_formats::ghost::{POSE_BLOCK_SIZE, PoseBlock, pack_half, pack_vector, unpack_half};
use ssx_net_proto::{Mode, NetPose, PoseFlag, PoseFlags, ProgressReport, ReportKind};
use ssx_rules::course::Course;
use ssx_rules::events::EventType;
use ssx_rules::intro::START_FIRST;
use ssx_rules::subset::TuningLimits;
use ssx_rules::triggers::{TriggerScope, TriggerVolume, VolumeShape};

use super::*;

const S: u32 = 1000;
const V: Vec3 = Vec3::new(0.0, -0.3, 0.6);
const P0: Vec3 = Vec3::new(10.0, 100.0, -20.0);
const RACE_START: u32 = 22;

fn tuning() -> TuningLimits {
    TuningLimits {
        fall: 1.1713,
        horizontal: 1.2731,
        spin: 0.2618,
        flip: 0.2269,
        grind_spin: 0.1105,
        trick_point_bonus: 1.03,
        heli_shut_out: 1200.0,
    }
}

fn time_at_y(y: f32) -> f64 {
    f64::from(S + RACE_START - 1) + f64::from((P0.y - y) / (-V.y * TICKS_PER_STEP))
}

fn place(time: f64) -> Vec3 {
    let moving = (time - f64::from(S + RACE_START - 1)).max(0.0) as f32;
    P0 + V * TICKS_PER_STEP * moving
}

fn velocity(time: f64) -> Vec3 {
    if time >= f64::from(S + RACE_START) { V } else { Vec3::ZERO }
}

fn end_volume() -> TriggerVolume {
    TriggerVolume {
        name: "end".into(),
        type_text: String::new(),
        groups: 0,
        interacts: 0,
        shape: VolumeShape::Box { half_extents: Vec3::splat(2.0) },
        centre: place(time_at_y(20.0)),
        rotation: Quat::IDENTITY,
        object: 0,
        instance: 0,
        placement: 0,
        scope: TriggerScope::Track,
    }
}

fn limits(event_type: EventType) -> Arc<Limits> {
    let t = tuning();
    let fall = t.fall * (1.0 + VELOCITY_ENCODING);
    Arc::new(Limits {
        drop: "test".into(),
        event_type: Some(event_type),
        splits: Course { start_y: P0.y + 5.0, end_y: 20.0 }.splits(),
        starts: vec![P0],
        end: vec![end_volume()],
        fall,
        horizontal: t.horizontal * (1.0 + VELOCITY_ENCODING) * HORIZONTAL_MARGIN,
        split_depth: SPLIT_EXECUTIONS * TICKS_PER_STEP * fall,
        tuning: t,
        start: Start {
            handover: f64::from(RACE_START - 1),
            slot: P0,
            rotation: Quat::IDENTITY,
            place: Vec3::ZERO,
            margin: 0.18,
            first: std::array::from_fn(|i| V * TICKS_PER_STEP * (i + 1) as f32),
            first_margins: std::array::from_fn(|_| 0.05),
        },
        rider_starts: Vec::new(),
        gates: Vec::new(),
        time_limit_ms: None,
        time_up: ssx_rules::gamemode::TimeUp::of(event_type),
        map: false,
    })
}

fn ready(v: &mut RiderValidation) -> Vec<Finding> {
    let t = f64::from(S + RACE_START - 1);
    v.ready(&honest_pose(t), &honest_pose(t + 2.0), arrival(t))
}

fn block(pos: Vec3, vel: Vec3) -> PoseBlock {
    let mut raw = [0u8; POSE_BLOCK_SIZE];
    raw[12..16].copy_from_slice(&1.0f32.to_be_bytes());
    raw[0x48..0x4C].copy_from_slice(&1.0f32.to_be_bytes());
    let mut b = PoseBlock::parse(&raw).expect("240 bytes parse");
    b.trajectory_position = pos.to_array();
    if vel != Vec3::ZERO {
        b.speed = pack_half(vel.length() * 1.001) | 0x8000;
        let s = unpack_half(b.speed & 0x7FFF);
        b.velocity = pack_vector(vel.to_array(), [s; 3]) & 0x3FFF_FFFF;
    }
    b
}

fn pose(time: f64, pos: Vec3, vel: Vec3, flags: PoseFlags) -> NetPose {
    let execution = ((time - f64::from(S)).max(0.0) as u32).min(ssx_net_proto::MAX_EXECUTION);
    NetPose::at(time, flags, block(pos, vel)).and_then(|p| p.with_execution(execution)).expect("a sample")
}

const HORIZON: f64 = 3.0;

fn arrival(time: f64) -> Arrival {
    Arrival { elapsed: time - f64::from(S), window: 1.0, horizon: HORIZON, pace: 1.0 }
}

fn honest_pose(time: f64) -> NetPose {
    pose(time, place(time), velocity(time), PoseFlags::default())
}

fn clock_ms(time: f64) -> u32 {
    let executions = (time - f64::from(S + RACE_START - 1)).max(0.0);
    (executions * 1000.0 / 30.0).floor() as u32
}

fn report(kind: ReportKind, split: Option<u8>, time: f64, clock_ms: u32, points: u32) -> ProgressReport {
    ProgressReport::new(ProgressReport {
        instance: 1,
        round: 0,
        kind,
        split,
        clock_ms,
        points,
        descent_mm: 0,
        rewinds: 0,
        loops: 0,
        sample: honest_pose(time),
    })
    .expect("a report")
}

type Edit<'a> = &'a mut dyn FnMut(f64, &mut Option<NetPose>, &mut Vec<ProgressReport>);

fn ride(event_type: EventType, mode: Mode, edit: Edit) -> (Vec<Finding>, Counts) {
    ride_with(&limits(event_type), mode, edit)
}

fn ride_with(lim: &Arc<Limits>, mode: Mode, edit: Edit) -> (Vec<Finding>, Counts) {
    let mut v = RiderValidation::new(Arc::clone(lim), mode, None);
    let mut out = Vec::new();
    let finish = (time_at_y(20.0) - 1.0).ceil();
    let (mut split, mut finished) = (0usize, false);
    out.extend(ready(&mut v));
    for k in (RACE_START - 1)..(finish as u32 + 4) {
        let t = f64::from(S + k);
        let sample = (k % 2 == 1).then(|| honest_pose(t));
        let mut reports = Vec::new();
        while k >= RACE_START && split < lim.splits.len() && place(t).y < lim.splits[split] {
            reports.push(report(ReportKind::Split, Some(split as u8), t, clock_ms(t), 0));
            split += 1;
        }
        if !finished && lim.end_distance(place(t)).is_some_and(|d| d <= ssx_rules::triggers::PLAYER_TOUCH_RADIUS) {
            reports.push(report(ReportKind::Finish, None, t, clock_ms(t), 0));
            finished = true;
        }
        let mut sample = sample;
        edit(t, &mut sample, &mut reports);
        if let Some(s) = &sample {
            out.extend(v.sample(s, arrival(t)));
        }
        for r in &reports {
            out.extend(v.report(r, arrival(t)));
        }
        if v.is_over() {
            break;
        }
    }
    out.extend(v.end());
    (out, v.counts)
}

fn checks(f: &[Finding]) -> Vec<Check> {
    let mut c: Vec<Check> = f.iter().map(|x| x.check).collect();
    c.sort();
    c.dedup();
    c
}

#[test]
fn an_honest_run_passes_every_check() {
    let (findings, counts) = ride(EventType::RaceClock, Mode::Race, &mut |_, _, _| {});
    assert!(findings.is_empty(), "{findings:#?}");
    for m in [Measure::Fall, Measure::Horizontal, Measure::Step, Measure::StepHorizontal, Measure::SplitOrder] {
        assert!(counts.of(m).made > 0, "{m:?} never measured");
    }
    for m in [Measure::SplitHeight, Measure::Finish, Measure::ClockStart, Measure::ClockReady, Measure::ClockBehind] {
        assert!(counts.of(m).made > 0, "{m:?} never measured");
    }
    assert_eq!(counts.check(Check::Splits).0, 2 * 9 + 1, "nine splits, order and height each, and the finish's");
}

#[test]
fn a_speed_beyond_the_tuning_is_found() {
    let lim = limits(EventType::RaceClock);
    let fast = Vec3::new(0.0, 0.0, lim.horizontal * 1.01);
    let (findings, _) = ride(EventType::RaceClock, Mode::Race, &mut |t, s, _| {
        if t == f64::from(S + 61) {
            *s = Some(pose(t, place(t), fast, PoseFlags::default()));
        }
    });
    assert!(checks(&findings).contains(&Check::Speed), "{findings:#?}");
    let (findings, _) = ride(EventType::RaceClock, Mode::Race, &mut |t, s, _| {
        if t == f64::from(S + 61) {
            *s = Some(pose(t, place(t), Vec3::new(0.0, -lim.fall * 1.01, 0.0), PoseFlags::default()));
        }
    });
    assert!(checks(&findings).contains(&Check::Speed), "{findings:#?}");
}

#[test]
fn a_teleport_is_a_step_and_a_flagged_one_must_land_where_the_run_rode() {
    let jump = Vec3::new(0.0, 0.0, 3.0);
    let (findings, _) = ride(EventType::RaceClock, Mode::Race, &mut |t, s, _| {
        if t >= f64::from(S + 61) && s.is_some() {
            *s = Some(pose(t, place(t) + jump, velocity(t), PoseFlags::default()));
        }
    });
    assert!(checks(&findings).contains(&Check::Step), "{findings:#?}");
    let far = Vec3::new(0.0, 0.0, REACH_M + 2.0 * GRID_CELL_M);
    let (findings, _) = ride(EventType::RaceClock, Mode::Race, &mut |t, s, _| {
        if t == f64::from(S + 61) {
            let f = PoseFlags::default().with(PoseFlag::Teleport).with_warp_epoch(1);
            *s = Some(pose(t, place(t) + far, velocity(t), f));
        }
    });
    assert!(checks(&findings).contains(&Check::Step), "{findings:#?}");
    let at = f64::from(S + 61);
    let (findings, _) = ride(EventType::RaceClock, Mode::Race, &mut |t, s, _| {
        if t == at {
            let f = PoseFlags::default().with(PoseFlag::Rewind).with_warp_epoch(1);
            *s = Some(pose(t, place(t - 20.0), velocity(t), f));
        }
    });
    assert!(!findings.iter().any(|f| f.check == Check::Step && f.execution == at - f64::from(S)), "{findings:#?}");
}

#[test]
fn splits_come_in_order_at_their_heights() {
    let (findings, _) = ride(EventType::RaceClock, Mode::Race, &mut |_, _, r| {
        r.retain(|x| x.split != Some(3));
    });
    assert!(checks(&findings).contains(&Check::Splits), "{findings:#?}");
    let lim = limits(EventType::RaceClock);
    let early = time_at_y(lim.splits[3]).ceil() - 10.0;
    let (findings, _) = ride(EventType::RaceClock, Mode::Race, &mut |t, _, r| {
        r.retain(|x| x.split != Some(3));
        if t == early {
            r.push(report(ReportKind::Split, Some(3), t, clock_ms(t), 0));
        }
    });
    assert!(checks(&findings).contains(&Check::Splits), "{findings:#?}");
}

#[test]
fn a_split_whose_line_lies_above_the_start_comes_at_the_first_execution() {
    let base = limits(EventType::RaceClock);
    let lim = Arc::new(Limits { splits: Course { start_y: P0.y + 13.0, end_y: 20.0 }.splits(), ..(*base).clone() });
    assert!(lim.splits[0] - P0.y > 7.9 && lim.splits[1] < P0.y, "{:?}", lim.splits);
    let first = f64::from(S + RACE_START);
    let (findings, _) = ride_with(&lim, Mode::Race, &mut |t, _, r| {
        if t == first {
            assert!(r.iter().any(|x| x.split == Some(0)), "split 0 at the first execution");
        }
    });
    assert!(findings.is_empty(), "{findings:#?}");
    let deep = time_at_y(P0.y - lim.split_depth - 1.0).ceil();
    let mut v = RiderValidation::new(Arc::clone(&lim), Mode::Race, None);
    let mut findings = ready(&mut v);
    for k in RACE_START..(deep - f64::from(S)) as u32 + 6 {
        let t = f64::from(S + k);
        if k % 2 == 1 {
            findings.extend(v.sample(&honest_pose(t), arrival(t)));
        }
        if t == deep {
            findings.extend(v.report(&report(ReportKind::Split, Some(0), t, clock_ms(t), 0), arrival(t)));
        }
    }
    assert!(findings.iter().any(|f| f.check == Check::Splits && f.detail.contains("below the start")), "{findings:#?}");
    let (findings, _) = ride_with(&lim, Mode::Race, &mut |t, _, r| {
        for x in r.iter_mut().filter(|x| t == first && x.split == Some(0)) {
            x.loops = 1;
        }
    });
    assert!(findings.iter().any(|f| f.check == Check::Splits && f.detail.contains("below it)")), "{findings:#?}");
}

#[test]
fn a_finish_outside_the_end_volume_is_found() {
    let (findings, _) = ride(EventType::RaceClock, Mode::Race, &mut |t, _, r| {
        r.retain(|x| x.kind != ReportKind::Finish);
        if t == f64::from(S + 80) {
            r.push(report(ReportKind::Finish, None, t, clock_ms(t), 0));
        }
    });
    assert_eq!(checks(&findings), vec![Check::Splits, Check::Finish], "{findings:#?}");
}

#[test]
fn a_clock_ahead_or_behind_is_found() {
    for (shift, name) in [(1000i64, "ahead 1 s"), (100, "ahead 0.1 s"), (-1000, "behind 1 s"), (-100, "behind 0.1 s")] {
        let (findings, _) = ride(EventType::RaceClock, Mode::Race, &mut |_, _, r| {
            for x in r.iter_mut() {
                x.clock_ms = (i64::from(x.clock_ms) + shift).max(0) as u32;
            }
        });
        assert!(checks(&findings).contains(&Check::Clock), "{name}: {findings:#?}");
    }
}

#[test]
fn trick_points_beyond_the_table_are_found() {
    let lim = limits(EventType::TrickScore);
    let at = f64::from(S + 61);
    let most = lim.trick_points_max(at - f64::from(S)).floor() as u32;
    for (points, found) in [(most, false), (most + 1, true)] {
        let (findings, _) = ride(EventType::TrickScore, Mode::Trick, &mut |t, _, r| {
            if t == at {
                r.push(report(ReportKind::Points, None, t, clock_ms(t), points));
            }
            for x in r.iter_mut().filter(|x| x.kind == ReportKind::Finish) {
                x.points = points;
            }
        });
        assert_eq!(checks(&findings).contains(&Check::Tricks), found, "{points}: {findings:#?}");
    }
}

#[test]
fn hostile_samples_and_reports_never_panic() {
    let lim = limits(EventType::RaceClock);
    let mut v = RiderValidation::new(Arc::clone(&lim), Mode::Race, None);
    let _ = v.ready(&honest_pose(f64::from(S)), &honest_pose(f64::from(S) + 2.0), arrival(f64::from(S)));
    let all = PoseFlags::default()
        .with(PoseFlag::Teleport)
        .with(PoseFlag::Rewind)
        .with(PoseFlag::Respawn)
        .with(PoseFlag::ImpactLatched)
        .with_warp_epoch(255);
    for (t, f) in [(1500.0, all), (1100.0, PoseFlags::default()), (1500.0, all), (4.0e9, all), (1001.0, all)] {
        let _ = v.sample(&pose(t, Vec3::splat(1.0e30), Vec3::new(1.0e4, -1.0e4, 0.0), f), arrival(t));
    }
    for k in 0..200u32 {
        let t = 2000.0 + f64::from(k);
        let (clock, points) = (ssx_net_proto::round::MAX_CLOCK_MS, ssx_net_proto::round::MAX_POINTS);
        let _ = v.report(&report(ReportKind::Split, Some((k % 9) as u8), t, clock, points), arrival(t));
    }
    let _ = v.end();
    assert!(v.is_over());
    assert_eq!(v.counts.check(Check::Splits).0, 2 * 200);
}

#[test]
fn the_clock_rounding_bound_covers_the_f32_sum() {
    let d = TICKS_PER_STEP / 60.0;
    let mut sum = 0.0f32;
    let mut marks = Vec::new();
    for n in 1..=108_000u32 {
        sum += d;
        let error = f64::from(sum) - f64::from(n) * f64::from(d);
        assert!(error.abs() <= clock_rounding_s(f64::from(n)) + 1e-12, "{n}: {error} beyond the bound");
        if n % 997 == 0 {
            marks.push((f64::from(n), error));
        }
    }
    for w in marks.windows(3) {
        let ((na, ea), (nb, eb)) = (w[0], w[2]);
        let moved = (eb - ea).abs() * 30.0;
        assert!(moved <= clock_rounding_between_ticks(na, nb) + 1e-9, "{na}..{nb}: {moved}");
    }
    assert_eq!(clock_rounding_s(0.0), 0.0);
    assert_eq!(clock_rounding_s(-5.0), 0.0);
}

#[test]
fn counts_keep_the_three_largest_excesses() {
    let mut c = Counts::default();
    for (v, l) in [(1.0, 2.0), (3.0, 2.0), (0.5, 2.0), (2.5, 2.0), (f64::NAN, 2.0)] {
        c.made(Measure::Step, v, l);
    }
    let s = c.of(Measure::Step);
    assert_eq!((s.made, s.found), (5, 3), "two above and the NaN");
    assert_eq!(s.top, [Some(1.0), Some(0.5), Some(-1.0)]);
    let mut d = Counts::default();
    d.made(Measure::Step, 4.0, 2.0);
    d.add(&c);
    assert_eq!(d.of(Measure::Step).top, [Some(2.0), Some(1.0), Some(0.5)]);
    assert_eq!(d.of(Measure::Step).worst, Some((4.0, 2.0)));
}

#[test]
fn actions_and_check_codes_round_trip() {
    for a in [Action::Warn, Action::Invalidate, Action::Kick] {
        assert_eq!(Action::parse(a.name()), Some(a));
    }
    assert_eq!(Action::parse("Kick"), None);
    for c in Check::ALL {
        assert_eq!(Check::from_code(c.code()), Some(c));
    }
    assert_eq!(Check::from_code(0), None);
    assert_eq!(Check::from_code(7), None);
    assert_eq!(Policy::default().of(Check::Clock), Action::Invalidate);
}

fn samples(seq: &[(f64, f64, f64)], moved: Option<(usize, Vec3)>) -> Vec<Finding> {
    let lim = limits(EventType::RaceClock);
    let mut v = RiderValidation::new(lim, Mode::Race, None);
    let mut out = ready(&mut v);
    for (i, &(exec, at, window)) in seq.iter().enumerate() {
        let t = f64::from(S) + exec;
        let shift = moved.filter(|(from, _)| i >= *from).map_or(Vec3::ZERO, |(_, d)| d);
        let p = pose(t, place(t) + shift, velocity(t), PoseFlags::default());
        out.extend(v.sample(&p, Arrival { elapsed: at, window, horizon: HORIZON, pace: 1.0 }));
    }
    out
}

#[test]
fn a_stalled_servers_bunched_samples_keep_their_executions() {
    let seq = [
        (30.0, 30.0, 1.0),
        (32.0, 32.0, 1.0),
        (34.0, 40.0, 8.0),
        (38.0, 40.0, 8.0),
        (40.0, 40.0, 8.0),
        (42.0, 41.0, 1.0),
        (44.0, 43.0, 1.0),
    ];
    let findings = samples(&seq, None);
    assert!(findings.is_empty(), "{findings:#?}");
    let seq = [(30.0, 30.0, 1.0), (32.0, 32.0, 1.0), (52.0, 52.5, 1.0), (54.0, 54.5, 1.0)];
    assert!(samples(&seq, None).is_empty());
}

#[test]
fn an_execution_beyond_the_servers_time_is_found() {
    assert!(samples(&[(30.0, 30.0, 1.0), (32.0, 32.0, 1.0), (36.0, 33.0, 1.0)], None).is_empty());
    let seq = [(30.0, 30.0, 1.0), (32.0, 32.0, 1.0), (40.0, 33.0, 1.0)];
    let findings = samples(&seq, None);
    assert!(findings.iter().any(|f| f.check == Check::Step && f.detail.contains("ahead")), "{findings:#?}");
}

#[test]
fn claimed_executions_do_not_widen_a_step_beyond_its_arrivals() {
    let jump = Vec3::new(0.0, 0.0, 10.0);
    let honest = samples(&[(30.0, 30.0, 1.0), (32.0, 32.0, 1.0), (34.0, 34.0, 1.0)], Some((2, jump)));
    assert!(honest.iter().any(|f| f.check == Check::Step), "the jump at 2 executions: {honest:#?}");
    let claimed = samples(&[(30.0, 30.0, 1.0), (32.0, 34.0, 1.0), (36.0, 36.0, 1.0)], Some((2, jump)));
    assert!(claimed.iter().any(|f| f.check == Check::Step), "the jump claimed as 4 in 2 ticks: {claimed:#?}");
    let gap = samples(&[(30.0, 30.0, 1.0), (32.0, 32.0, 1.0), (44.0, 44.0, 1.0)], Some((2, jump * 0.5)));
    assert!(gap.iter().all(|f| f.check != Check::Step), "{gap:#?}");
}

#[test]
fn the_clock_is_exact_against_its_executions() {
    let lim = limits(EventType::RaceClock);
    for (extra, found) in [(0u32, false), (1, true)] {
        let mut v = RiderValidation::new(Arc::clone(&lim), Mode::Race, None);
        assert!(ready(&mut v).is_empty());
        let t = f64::from(S + 61);
        for k in (21..=61).step_by(2) {
            let tk = f64::from(S + k);
            let _ = v.sample(&honest_pose(tk), arrival(tk));
        }
        let r = report(ReportKind::Split, Some(0), t, clock_ms(t) + extra, 0);
        let mut f = v.report(&r, arrival(t));
        f.extend(v.end());
        assert_eq!(f.iter().any(|x| x.check == Check::Clock), found, "{extra} ms: {f:#?}");
    }
}

#[test]
fn readys_sample_is_the_drops_start() {
    let lim = limits(EventType::RaceClock);
    let t = f64::from(S + RACE_START - 1);
    let at = |exec: u32, moved: Vec3| {
        let mut v = RiderValidation::new(Arc::clone(&lim), Mode::Race, None);
        let tk = f64::from(S + exec);
        let second = pose(tk + 2.0, place(tk + 2.0) + moved, V, PoseFlags::default());
        let mut f = v.ready(&pose(tk, P0 + moved, Vec3::ZERO, PoseFlags::default()), &second, arrival(t));
        f.extend(v.sample(&pose(t + 2.0, place(t + 2.0) + moved, V, PoseFlags::default()), arrival(t + 2.0)));
        f
    };
    assert!(at(RACE_START - 1, Vec3::ZERO).is_empty());
    assert!(at(RACE_START - 1, Vec3::new(0.0, 0.1, 0.1)).is_empty(), "within the margin");
    for exec in [RACE_START - 2, RACE_START] {
        let f = at(exec, Vec3::ZERO);
        assert!(f.iter().any(|x| x.check == Check::Clock && x.detail.contains("hands over")), "{exec}: {f:#?}");
    }
    let f = at(RACE_START - 1, Vec3::new(0.0, 0.0, 0.3));
    assert!(f.iter().any(|x| x.check == Check::Step && x.detail.contains("hand-over")), "{f:#?}");
    let mut v = RiderValidation::new(Arc::clone(&lim), Mode::Race, None);
    assert!(ready(&mut v).is_empty());
    let f =
        v.sample(&pose(t + 3.0, place(t + 3.0) + Vec3::new(0.0, -2.5, 5.0), V, PoseFlags::default()), arrival(t + 3.0));
    assert!(f.iter().any(|x| x.check == Check::Step), "{f:#?}");
    let far = pose(t, P0 + Vec3::splat(9.0), Vec3::ZERO, PoseFlags::default());
    assert!(v.ready(&far, &far, arrival(t)).is_empty());
}

#[test]
fn readys_sample_is_the_riders_way() {
    const OWN: u32 = 0x1234_5678;
    const ALONE: u32 = 0x0BAD_CAFE;
    let base = limits(EventType::RaceClock);
    let way = |handover: f64, place: Vec3| Start { handover, place, ..base.start };
    let mut lim = (*base).clone();
    let own = way(base.start.handover + 20.0, Vec3::new(0.0, -1.0, 3.0));
    let alone = way(base.start.handover + 10.0, Vec3::new(0.0, -0.5, 1.5));
    lim.rider_starts = vec![
        RiderStart { character: OWN, start: own, every_path: false },
        RiderStart { character: ALONE, start: alone, every_path: true },
    ];
    let lim = Arc::new(lim);
    assert_eq!(lim.starts_of(None), vec![base.start]);
    assert_eq!(lim.starts_of(Some(7)), vec![base.start]);
    assert_eq!(lim.starts_of(Some(OWN)), vec![base.start, own]);
    assert_eq!(lim.starts_of(Some(ALONE)), vec![alone]);
    assert_eq!(lim.earliest_handover(Some(OWN)), base.start.handover);
    assert_eq!(lim.earliest_handover(Some(ALONE)), alone.handover);
    let at = |character: Option<u32>, w: Start| {
        let mut v = RiderValidation::new(Arc::clone(&lim), Mode::Race, character);
        let t = f64::from(S) + w.handover;
        let second = pose(t + 2.0, P0 + w.place + w.first[1], V, PoseFlags::default());
        v.ready(&pose(t, P0 + w.place, Vec3::ZERO, PoseFlags::default()), &second, arrival(t))
    };
    let start_found = |f: &[Finding]| f.iter().any(|x| matches!(x.check, Check::Clock | Check::Step));
    for (character, w, found) in [
        (None, base.start, false),
        (None, own, true),
        (Some(OWN), base.start, false),
        (Some(OWN), own, false),
        (Some(OWN), alone, true),
        (Some(ALONE), alone, false),
        (Some(ALONE), base.start, true),
        (Some(7), own, true),
    ] {
        let f = at(character, w);
        assert_eq!(start_found(&f), found, "{character:?} at {}: {f:#?}", w.handover);
    }
    let mut v = RiderValidation::new(Arc::clone(&lim), Mode::Race, Some(OWN));
    let t = f64::from(S) + own.handover;
    let second = pose(t + 2.0, P0 + base.start.place + own.first[1], V, PoseFlags::default());
    let f = v.ready(&pose(t, P0 + base.start.place, Vec3::ZERO, PoseFlags::default()), &second, arrival(t));
    assert!(f.iter().any(|x| x.check == Check::Step && x.detail.contains("hand-over")), "{f:#?}");
}

#[test]
fn the_first_executions_follow_the_intros_way() {
    let lim = limits(EventType::RaceClock);
    let t = f64::from(S + RACE_START - 1);
    let first = |ahead: f64, k: u32, flags: PoseFlags| {
        let mut v = RiderValidation::new(Arc::clone(&lim), Mode::Race, None);
        let tk = t + f64::from(k);
        let sample = pose(tk, place(tk + ahead), V, flags);
        let second = if k == 2 { sample.clone() } else { honest_pose(t + 2.0) };
        assert!(v.ready(&honest_pose(t), &second, arrival(t)).is_empty());
        v.sample(&sample, arrival(tk))
    };
    for k in 1..=START_FIRST as u32 {
        assert!(first(0.0, k, PoseFlags::default()).is_empty(), "{k}");
        let f = first(1.0, k, PoseFlags::default());
        assert!(
            f.iter().any(|x| x.check == Check::Step && x.detail.contains("intro's way")),
            "{k} executions after: {f:#?}"
        );
    }
    for f in [
        first(1.0, START_FIRST as u32 + 1, PoseFlags::default()),
        first(1.0, 2, PoseFlags::default().with_warp_epoch(1)),
    ] {
        assert!(!f.iter().any(|x| x.detail.contains("intro's way")), "{f:#?}");
    }
}

#[test]
fn readys_second_sample_is_two_executions_later() {
    let lim = limits(EventType::RaceClock);
    let t = f64::from(S + RACE_START - 1);
    let new = || RiderValidation::new(Arc::clone(&lim), Mode::Race, None);
    for e in [1.0, 3.0, 0.0] {
        let mut v = new();
        let f = v.ready(&honest_pose(t), &honest_pose(t + e), arrival(t));
        assert!(f.iter().any(|x| x.check == Check::Clock && x.detail.contains("second sample")), "{e}: {f:#?}");
    }
    let mut v = new();
    let mut f = v.ready(&honest_pose(t), &honest_pose(t + 2.0), arrival(t + 2.0));
    for k in [1.0, 3.0, 5.0] {
        f.extend(v.sample(&honest_pose(t + k), arrival(t + 2.0 + k)));
    }
    f.extend(v.end());
    assert!(f.is_empty(), "{f:#?}");
    assert_eq!(v.counts.measures[Measure::StartFirst.index()].made, 3);
    let mut v = new();
    let ahead = |k: f64| pose(t + k, place(t + k + 1.0), V, PoseFlags::default());
    let mut f = v.ready(&honest_pose(t), &ahead(2.0), arrival(t + 2.0));
    assert!(f.is_empty(), "{f:#?}");
    f.extend(v.end());
    assert!(f.iter().any(|x| x.check == Check::Step && x.detail.contains("intro's way")), "{f:#?}");
}

#[test]
fn a_late_clock_is_found_against_the_drops_start() {
    let lim = limits(EventType::RaceClock);
    for (late_ms, found) in [(0u32, false), (50, true), (100, true)] {
        let mut v = RiderValidation::new(Arc::clone(&lim), Mode::Race, None);
        assert!(ready(&mut v).is_empty());
        let t = f64::from(S + 61);
        for k in (21..=61).step_by(2) {
            let tk = f64::from(S + k);
            let _ = v.sample(&honest_pose(tk), arrival(tk));
        }
        let r = report(ReportKind::Split, Some(0), t, clock_ms(t) - late_ms, 0);
        let mut f = v.report(&r, arrival(t));
        f.extend(v.end());
        assert_eq!(f.iter().any(|x| x.check == Check::Clock), found, "{late_ms} ms late: {f:#?}");
    }
}

fn gate(y: f32, aside: f32) -> TriggerVolume {
    TriggerVolume {
        name: String::new(),
        shape: VolumeShape::Sphere { radius: 1.0 },
        centre: place(time_at_y(y)) + Vec3::X * aside,
        ..end_volume()
    }
}

fn map_limits(gates: Vec<TriggerVolume>, time_limit_ms: Option<u32>) -> Arc<Limits> {
    Arc::new(Limits { gates, time_limit_ms, ..(*limits(EventType::RaceClock)).clone() })
}

#[test]
fn a_maps_gates_are_passed_in_order_before_the_finish() {
    let on = map_limits(vec![gate(80.0, 0.0), gate(50.0, 0.5)], None);
    let (findings, counts) = ride_with(&on, Mode::Race, &mut |_, _, _| {});
    assert!(findings.is_empty(), "{findings:#?}");
    assert_eq!(counts.of(Measure::Gates).made, 1);
    for aside in [2.0f32, 5.0, 50.0] {
        let skipped = map_limits(vec![gate(80.0, 0.0), gate(50.0, aside)], None);
        let (findings, counts) = ride_with(&skipped, Mode::Race, &mut |_, _, _| {});
        assert_eq!(checks(&findings), vec![Check::Finish], "{aside}: {findings:#?}");
        assert_eq!(counts.of(Measure::Gates).found, 1, "{aside}");
        assert_eq!(counts.of(Measure::Finish).found, 1, "{aside}: the finish closed");
    }
    let near = map_limits(vec![gate(50.0, 1.0 + 0.8)], None);
    assert!(ride_with(&near, Mode::Race, &mut |_, _, _| {}).0.is_empty());
    let order = map_limits(vec![gate(50.0, 0.0), gate(80.0, 0.0)], None);
    let (findings, counts) = ride_with(&order, Mode::Race, &mut |_, _, _| {});
    assert_eq!(checks(&findings), vec![Check::Finish], "{findings:#?}");
    assert_eq!(counts.of(Measure::Gates).worst.map(|w| w.0), Some(1.0));
}

#[test]
fn a_finish_beyond_the_maps_time_limit_is_found() {
    let mut finish = 0;
    ride(EventType::RaceClock, Mode::Race, &mut |_, _, r| {
        if let Some(f) = r.iter().find(|x| x.kind == ReportKind::Finish) {
            finish = f.clock_ms;
        }
    });
    assert!(finish > 2000, "{finish}");
    for limit in [finish + 5000, finish, finish - 20] {
        let lim = map_limits(Vec::new(), Some(limit));
        let (findings, counts) = ride_with(&lim, Mode::Race, &mut |_, _, _| {});
        assert!(findings.is_empty(), "{limit}: {findings:#?}");
        assert_eq!(counts.of(Measure::TimeLimit).made, 1);
    }
    for limit in [finish - 40, finish - 1000, 1] {
        let lim = map_limits(Vec::new(), Some(limit));
        let (findings, counts) = ride_with(&lim, Mode::Race, &mut |_, _, _| {});
        assert!(checks(&findings).contains(&Check::Clock), "{limit}: {findings:#?}");
        assert_eq!(counts.of(Measure::TimeLimit).found, 1, "{limit}");
    }
    let lim = map_limits(Vec::new(), Some(8000));
    let max = lim.time_limit_finish_ms().expect("a limit");
    assert!((8033.33..8033.4).contains(&max), "{max}");
}

#[test]
fn a_run_the_time_limit_ends_passes_and_a_claimed_one_is_found() {
    use ssx_rules::gamemode::TimeUp;
    let mut finish = 0;
    ride(EventType::SurviveToBottom, Mode::Survive, &mut |_, _, r| {
        if let Some(f) = r.iter().find(|x| x.kind == ReportKind::Finish) {
            finish = f.clock_ms;
        }
    });
    let limit = finish / 2;
    let timed = |time_up: TimeUp| {
        Arc::new(Limits {
            gates: vec![gate(25.0, 0.0)],
            time_limit_ms: Some(limit),
            time_up,
            ..(*limits(EventType::SurviveToBottom)).clone()
        })
    };
    for (time_up, kind) in [(TimeUp::Fail, ReportKind::Failed), (TimeUp::Result, ReportKind::Finish)] {
        let lim = timed(time_up);
        let mut stop: Option<f64> = None;
        let (findings, counts) = ride_with(&lim, Mode::Survive, &mut |t, _, reports| {
            if stop.is_none() && clock_ms(t) >= limit {
                stop = Some(t);
            }
            if let Some(s) = stop.filter(|&s| t > s) {
                reports.clear();
                if t == s + 1.0 {
                    reports.push(report(kind, None, t, clock_ms(s), 0));
                }
            }
        });
        assert!(stop.is_some(), "{time_up:?}: the limit {limit} ms reached");
        assert!(findings.is_empty(), "{time_up:?}: {findings:#?}");
        if kind == ReportKind::Finish {
            assert_eq!(counts.of(Measure::TimeLimit).made, 1);
        }
    }
    let lim = timed(TimeUp::Result);
    let mut claimed = false;
    let (findings, _) = ride_with(&lim, Mode::Survive, &mut |t, _, reports| {
        if !claimed && clock_ms(t) >= limit / 2 {
            reports.push(report(ReportKind::Finish, None, t, clock_ms(t), 0));
            claimed = true;
        }
    });
    assert!(checks(&findings).contains(&Check::Finish), "{findings:#?}");
}

#[test]
fn a_maps_limits_come_from_its_definition() {
    let t = tuning();
    let map = MapLimits {
        id: "slope:map/course".into(),
        event_type: Some(EventType::RaceClock),
        spawns: vec![P0, P0 + Vec3::X * 3.0],
        end: vec![end_volume()],
        splits: Course { start_y: P0.y, end_y: 20.0 }.splits(),
        gates: vec![gate(80.0, 0.0)],
        time_limit_ms: Some(90_000),
        time_up: ssx_rules::gamemode::TimeUp::Fail,
        handover: 21,
        slot: P0,
        rotation: Quat::from_rotation_y(0.3),
        place: Vec3::new(0.0, 0.1, 0.4),
        first: [Vec3::Z; START_FIRST],
        place_spread: 0.25,
        first_spread: [0.1, 0.2, 0.3, 0.4],
    };
    let l = Limits::of_map(map.clone(), &t).expect("limits");
    let result = MapLimits { time_up: ssx_rules::gamemode::TimeUp::Result, ..map.clone() };
    assert!(Limits::of_map(result, &t).expect_err("a race's result at the limit").contains("no result"));
    assert_eq!(
        (l.drop.as_str(), l.starts.len(), l.gates.len(), l.time_limit_ms),
        ("slope:map/course", 2, 1, Some(90_000))
    );
    let first = std::array::from_fn(|k| MAP_FIRST_MARGINS_M[k] + map.first_spread[k]);
    assert_eq!((l.start.handover, l.start.margin, l.start.first_margins), (21.0, MAP_HANDOVER_MARGIN_M + 0.25, first));
    assert_eq!(l.horizontal, limits(EventType::RaceClock).horizontal);
    assert!(Limits::of_map(MapLimits { spawns: Vec::new(), ..map.clone() }, &t).is_err());
    assert!(Limits::of_map(MapLimits { place: Vec3::NAN, ..map.clone() }, &t).is_err());
    assert!(Limits::of_map(MapLimits { rotation: Quat::from_xyzw(0.0, 0.0, 0.0, 0.0), ..map.clone() }, &t).is_err());
    assert!(Limits::of_map(MapLimits { place_spread: f32::NAN, ..map.clone() }, &t).is_err());
    assert!(Limits::of_map(MapLimits { first_spread: [0.1, -0.2, 0.3, 0.4], ..map.clone() }, &t).is_err());
    assert!(Limits::of_map(MapLimits { spawns: vec![P0, Vec3::NAN], ..map.clone() }, &t).is_err());
    let bad = |f: &dyn Fn(&mut TriggerVolume)| {
        let mut v = gate(80.0, 0.0);
        f(&mut v);
        v
    };
    for v in [
        bad(&|v| v.centre.y = f32::NAN),
        bad(&|v| v.rotation = Quat::from_xyzw(0.0, 0.0, 0.0, 0.0)),
        bad(&|v| v.shape = VolumeShape::Sphere { radius: -1.0 }),
        bad(&|v| v.shape = VolumeShape::Box { half_extents: Vec3::new(1.0, f32::INFINITY, 1.0) }),
    ] {
        assert!(Limits::of_map(MapLimits { gates: vec![v.clone()], ..map.clone() }, &t).is_err(), "{v:?}");
        assert!(Limits::of_map(MapLimits { end: vec![v.clone()], ..map.clone() }, &t).is_err(), "{v:?}");
    }
    let mut splits = map.splits;
    splits[3] = f32::INFINITY;
    assert!(Limits::of_map(MapLimits { splits, ..map }, &t).is_err());
}
