use super::*;
use crate::DT_TICKS;
use crate::testutil::{plane, world};
use RewindState::*;
use glam::Vec3;

const OFF: RewindPad = RewindPad { rewind_held: false, takeover: false, flip_accel: 0.0 };
const HELD: RewindPad = RewindPad { rewind_held: true, ..OFF };

fn rider(w: &World) -> RiderState {
    RiderState::spawn(w, Vec3::new(0.0, 1.0, 0.0), Vec3::Z).expect("ground")
}

struct Sim {
    w: World,
    s: RiderState,
    buf: RewindBuffer,
    m: RewindManager,
    ctx: ScoringContext,
    dt: f32,
}

impl Sim {
    fn new(records: usize) -> Self {
        let w = world(plane(500.0, 0.0));
        let s = rider(&w);
        let m = RewindManager::new(RewindTuning::default());
        let mut sim = Self { w, s, buf: RewindBuffer::new(), m, ctx: ScoringContext::default(), dt: DT_TICKS };
        (0..records).for_each(|_| _ = sim.exec(OFF));
        sim
    }

    fn exec(&mut self, pad: RewindPad) -> RewindStep {
        let r = self.m.step(&mut self.buf, &mut self.s, &self.w, &pad, &self.ctx, self.dt);
        if r.physics {
            self.s.position.z += 1.0;
            let mut c = self.ctx;
            r.apply_to(&mut c);
            let (t, p) = (&self.w.tuning.scoring, &self.w.props);
            self.s.score.update_with(t, p, &ScoringInputs::default(), &c, r.delta_ticks);
        }
        self.buf.record_with(&self.s, &RecordFlags { delta_ticks: r.delta_ticks, ..RecordFlags::default() });
        self.dt = DT_TICKS * r.next_dilation;
        r
    }

    fn press(&mut self) -> RewindStep {
        (0..8).for_each(|_| assert_eq!(self.exec(HELD).state, PreRewind));
        self.exec(HELD)
    }
}

#[test]
fn the_buffer_arms_on_the_first_grounded_execution() {
    let w = world(plane(500.0, 0.0));
    let mut s = rider(&w);
    let mut buf = RewindBuffer::new();
    s.state = RideState::Airborne;
    (0..5).for_each(|_| {
        buf.record_with(&s, &RecordFlags::default());
    });
    assert!(buf.is_empty());
    s.state = RideState::Riding;
    buf.record_with(&s, &RecordFlags::default());
    s.state = RideState::Airborne;
    buf.record_with(&s, &RecordFlags::default());
    assert_eq!(buf.len(), 2);
    let mut b = RewindBuffer::new();
    s.state = RideState::Riding;
    b.record_with(&s, &RecordFlags { track_started: false, ..RecordFlags::default() });
    assert!(b.is_empty());
    b.record_with(&s, &RecordFlags::default());
    assert_eq!(b.len(), 1);
    (0..500).for_each(|_| {
        buf.record_with(&s, &RecordFlags::default());
    });
    assert_eq!(buf.len(), REWIND_SNAPSHOTS - 1);
}

#[test]
fn rewinds_left_and_sessions() {
    let mut c = RewindCount { rewinds_left: 1, initial: 1, session_start: 0.0, enabled: true, infinite: false };
    assert!(c.can_rewind(500.0, false));
    c.consume(500.0);
    assert_eq!((c.rewinds_left, c.session_start), (0, 500.0));
    assert!(c.can_rewind(530.0, false));
    c.consume(530.0);
    assert_eq!((c.rewinds_left, c.session_start), (0, 500.0));
    assert!(!c.can_rewind(600.0, false));
    c.enabled = false;
    assert!(!c.can_rewind(530.0, false) && c.can_rewind(530.0, true));
    c.infinite = true;
    assert!(!c.can_rewind(1e4, false) && c.can_rewind(1e4, true));
    c.consume(1e4);
    assert_eq!(c.rewinds_left, 0);
}

#[test]
fn the_count_is_initialised_from_the_resolved_properties() {
    let mut w = world(plane(500.0, 0.0));
    let mut s = rider(&w);
    let (mut buf, ctx) = (RewindBuffer::new(), ScoringContext::default());
    let mut m = RewindManager::new(RewindTuning::default());
    w.props.max_rewind_count = 5.7;
    s.props_resolved = false;
    m.step(&mut buf, &mut s, &w, &OFF, &ctx, DT_TICKS);
    assert_eq!((m.count.rewinds_left, m.count.initial, m.count_initialised), (0, 0, false));
    s.props_resolved = true;
    m.step(&mut buf, &mut s, &w, &OFF, &ctx, DT_TICKS);
    assert_eq!((m.count.rewinds_left, m.count.initial, m.count_initialised), (5, 5, true));
    (m.count.rewinds_left, m.count.session_start, w.props.max_rewind_count) = (4, 300.0, 2.0);
    m.step(&mut buf, &mut s, &w, &OFF, &ctx, DT_TICKS);
    assert_eq!(m.count.rewinds_left, 4);
    m.restart(&mut buf);
    assert!(!m.count_initialised);
    m.step(&mut buf, &mut s, &w, &OFF, &ctx, DT_TICKS);
    assert_eq!((m.count.rewinds_left, m.count.initial, m.count.session_start), (2, 2, 0.0));
}

#[test]
fn a_restart_gives_the_fresh_start_s_node() {
    let mut sim = Sim::new(0);
    sim.w.props.max_rewind_count = 3.0;
    sim.s.props_resolved = true;
    let ride = |sim: &mut Sim, n: usize| (0..n).for_each(|_| _ = sim.exec(OFF));
    let rewind = |sim: &mut Sim| {
        let seen: Vec<_> = (0..20).map(|_| sim.exec(HELD).state).collect();
        assert!(seen.contains(&Enter), "the rewind is granted: {seen:?}");
        let back = (0..1000).position(|_| sim.exec(OFF).state == Idle);
        assert!(back.is_some(), "the replay ends");
        ride(sim, 100);
    };
    sim.m.count.infinite = false;
    ride(&mut sim, 400);
    rewind(&mut sim);
    rewind(&mut sim);
    assert_eq!(sim.m.count.rewinds_left, 1, "two rewinds spent");
    assert!(sim.m.count.session_start > 600.0, "the last session's start: {}", sim.m.count.session_start);
    let fresh = RewindManager::new(sim.m.tuning.clone());
    sim.m.restart(&mut sim.buf);
    assert_eq!(sim.m, fresh, "the restart's node is the constructor's");
    assert!(sim.buf.is_empty() && sim.buf.time == 0.0, "a new buffer");
    sim.m.count.infinite = false;
    ride(&mut sim, 100);
    assert_eq!((sim.m.count.rewinds_left, sim.m.count.initial), (3, 3), "refilled");
    rewind(&mut sim);
    assert_eq!(sim.m.count.rewinds_left, 2, "the first rewind after the restart costs one");
}

#[test]
fn a_live_limit_replaces_the_resolved_count() {
    let w = world(plane(500.0, 0.0));
    let mut s = rider(&w);
    s.props_resolved = true;
    let (mut buf, ctx) = (RewindBuffer::new(), ScoringContext::default());
    let mut m = RewindManager::new(RewindTuning::default());
    m.set_live_rule(LiveRewindRule::Limit(1));
    assert!(!m.count.infinite);
    m.step(&mut buf, &mut s, &w, &OFF, &ctx, DT_TICKS);
    assert_eq!((m.count.rewinds_left, m.count.initial), (1, 1));
    assert_eq!(w.props.max_rewind_count, 3.0);
    m.restart(&mut buf);
    assert!(!m.count.infinite && !m.count_initialised);
    m.step(&mut buf, &mut s, &w, &OFF, &ctx, DT_TICKS);
    assert_eq!((m.count.rewinds_left, m.count.initial), (1, 1));
    m.set_live_rule(LiveRewindRule::Off);
    m.restart(&mut buf);
    assert!(!m.count.enabled && m.count.infinite);
}

#[test]
fn the_legacy_record_and_rewind() {
    let w = world(plane(500.0, 0.0));
    let mut s = rider(&w);
    let mut buf = RewindBuffer::new();
    for i in 0..10 {
        (s.position.z, s.score.score) = (i as f32, 10.0 * i as f32);
        buf.record(&s, DT_TICKS);
    }
    assert_eq!((buf.len(), buf.span_ticks()), (10, 18.0));
    s.score.air_bank = 50.0;
    assert_eq!(buf.rewind(&mut s, 4.0), Some(4.0));
    assert_eq!((s.position.z, s.score.score, s.score.air_bank), (7.0, 70.0, 50.0));
    assert_eq!(buf.len(), 8);
    buf.record(&s, DT_TICKS);
    assert_eq!((buf.len(), buf.span_ticks()), (9, 16.0));
}

#[test]
fn rewind_release_replay_and_restore() {
    let k = RewindTuning::default();
    let mut g = Sim::new(100);
    assert_eq!((g.buf.len(), g.s.position.z), (100, 100.0));
    g.s.score.score = 999.0;
    let r = g.exec(HELD);
    assert_eq!((r.state, r.physics, r.next_dilation), (PreRewind, true, 1.0));
    assert_eq!(g.m.start_index, 100);
    for i in 1..=7 {
        let r = g.exec(HELD);
        let x = (2.0 * i as f32 / 60.0) / k.pre_time;
        assert_eq!((r.state, r.physics), (PreRewind, true));
        assert!((r.next_dilation - k.pre_curve.eval(x)).abs() < 1e-6 && r.next_dilation < 1.0);
    }
    assert!((g.dt - DT_TICKS * k.pre_curve.eval(14.0 / 60.0 / 0.25)).abs() < 1e-6);
    assert_eq!(g.buf.len(), 108);
    let dt = g.dt;
    (g.s.score.score, g.s.score.air_bank) = (5555.0, 123.0);
    let r = g.exec(HELD);
    assert_eq!((r.state, r.physics, r.active, r.next_dilation), (Enter, false, true, 1.0));
    assert!((r.speed + 2.0 / dt).abs() < 1e-5, "step −2 over the dilated dt");
    assert_eq!(g.s.position.z, 107.0);
    assert_eq!((g.s.score.score, g.s.score.air_bank), (999.0, 123.0));
    assert!(g.s.score.rewind_bank < 0.0 && g.s.score.is_rewinding_backwards);
    for i in 1..=11 {
        let r = g.exec(HELD);
        let f = k.enter_curve.eval((2.0 * i as f32 / 60.0) / k.enter_time);
        assert_eq!((r.state, r.next_dilation), (Enter, 1.0));
        assert!((r.speed + f).abs() < 1e-5, "{} vs {f}", r.speed);
    }
    let r = g.exec(HELD);
    assert_eq!((r.state, r.speed), (Rewinding, -1.0));
    for _ in 0..20 {
        let z = g.s.position.z;
        assert_eq!(g.exec(HELD).state, Rewinding);
        assert_eq!(g.s.position.z, z - 1.0);
    }
    assert_eq!(g.s.score.time_since_rewind, 0.0);
    assert_eq!((g.exec(OFF).state, g.buf.speed), (Decelerate, -1.0));
    for i in 1..=10 {
        let r = g.exec(OFF);
        let f = k.transition_a_curve.eval((2.0 * i as f32 / 60.0) / k.transition_a_time).max(0.01);
        assert_eq!(r.state, Decelerate);
        assert!((r.speed + f).abs() < 1e-5);
    }
    let r = g.exec(OFF);
    assert_eq!(r.state, Accelerate);
    assert!((r.speed + 0.01).abs() < 1e-6, "the last backwards step at the 0.01 floor");
    for i in 1..=9 {
        let r = g.exec(OFF);
        let f = k.transition_b_curve.eval((2.0 * i as f32 / 60.0) / k.transition_b_time);
        assert_eq!((r.state, r.physics), (Accelerate, false));
        assert!((r.speed - f).abs() < 1e-5);
    }
    let r = g.exec(OFF);
    assert_eq!((r.state, r.speed), (Replay, 1.0));
    let mut n = 0;
    let r = loop {
        let z = g.s.position.z;
        let r = g.exec(OFF);
        if r.terminating {
            break r;
        }
        assert_eq!((r.state, g.s.position.z), (Replay, z + 1.0));
        n += 1;
        assert!(n < 100);
    };
    assert_eq!((r.state, r.physics, r.active), (Idle, true, false));
    assert_eq!((g.s.position.z, g.buf.len()), (102.0, 101));
    assert_eq!(g.s.score.air_bank, 123.0);
}

#[test]
fn the_playback_scores_on_the_snapshot_s_ticks() {
    let k = RewindTuning::default();
    let mut g = Sim::new(100);
    let pre = |timer: f32| k.pre_curve.eval(timer / 60.0 / k.pre_time);
    let per_tick = -1000.0 * 0.10 / 60.0;
    let r = g.press();
    assert_eq!(r.state, Enter);
    assert!((r.delta_ticks - DT_TICKS * pre(10.0)).abs() < 1e-5, "{}", r.delta_ticks);
    assert!((g.s.score.rewind_bank - per_tick * DT_TICKS * pre(10.0)).abs() < 1e-4, "{}", g.s.score.rewind_bank);
    assert!((g.s.score.rewind_bank + 1.7828).abs() < 1e-3, "the original's first bank: {}", g.s.score.rewind_bank);
    let shown = [10.0, 10.0, 8.0, 8.0, 6.0, 6.0, 4.0, 2.0, 2.0, 0.0, 0.0];
    for t in shown {
        let r = g.exec(HELD);
        assert_eq!(r.state, Enter);
        assert!((r.delta_ticks - DT_TICKS * pre(t)).abs() < 1e-5, "timer {t}: {}", r.delta_ticks);
    }
    assert!((g.s.score.rewind_bank + 29.7665).abs() < 1e-3, "the original's enter total: {}", g.s.score.rewind_bank);
    let mut n = 12;
    loop {
        let before = g.s.score.rewind_bank;
        let r = g.exec(HELD);
        n += 1;
        assert_eq!((r.state, r.delta_ticks), (Rewinding, DT_TICKS));
        if before - g.s.score.rewind_bank > -per_tick * DT_TICKS * 1.5 {
            break;
        }
        assert!(n < 30);
    }
    assert_eq!(n, 19);
}

#[test]
fn rewinding_stops_after_the_oldest_snapshot() {
    let mut g = Sim::new(10);
    assert_eq!(g.press().state, Enter);
    let mut stopped = None;
    for _ in 0..60 {
        let r = g.exec(HELD);
        if r.stopped_at_start {
            stopped = Some(r);
            break;
        }
    }
    let r = stopped.expect("stopped at the start of the buffer");
    assert_eq!((r.state, r.speed, r.active), (Decelerate, 0.0, true));
    assert_eq!(g.s.position.z, 1.0);
    for _ in 0..30 {
        let r = g.exec(HELD);
        assert!(r.stopped_at_start && r.state == Decelerate && g.s.position.z == 1.0);
    }
    let r = (0..80).map(|_| g.exec(OFF)).find(|r| r.terminating).expect("replay ends");
    assert_eq!((r.state, g.s.position.z), (Idle, 12.0));
}

#[test]
fn a_takeover_input_ends_the_replay() {
    let mut g = Sim::new(60);
    g.press();
    (0..20).for_each(|_| _ = g.exec(HELD));
    let r = (0..40).map(|_| g.exec(OFF)).find(|r| r.state == Replay).expect("replaying");
    assert!(!r.physics);
    g.exec(OFF);
    let z = g.s.position.z;
    let r = g.exec(RewindPad { takeover: true, ..OFF });
    assert_eq!((r.state, r.terminating, r.physics, r.active), (Idle, true, true, false));
    assert_eq!(g.s.position.z, z + 2.0);
    assert_eq!(g.exec(OFF).state, Idle);
}

#[test]
fn a_tap_aborts_with_the_slow_down() {
    let k = RewindTuning::default();
    let mut g = Sim::new(20);
    (0..4).for_each(|_| _ = g.exec(HELD));
    (0..4).for_each(|_| assert_eq!(g.exec(OFF).state, PreRewind));
    let r = g.exec(OFF);
    assert_eq!((r.state, r.physics), (Abort, true));
    assert!((r.next_dilation - k.pre_curve.eval(1.0)).abs() < 1e-6);
    for i in 1..=30 {
        let r = g.exec(HELD);
        let f = k.abort_curve.eval(2.0 * i as f32 / 60.0 / k.abort_time).max(0.01);
        assert_eq!((r.state, r.physics), (Abort, true));
        assert!((r.next_dilation - f).abs() < 1e-6);
    }
    let r = g.exec(HELD);
    assert_eq!((r.state, r.next_dilation), (WaitRelease, 1.0));
    assert_eq!(g.exec(HELD).state, WaitRelease);
    assert_eq!(g.exec(OFF).state, Idle);
    assert_eq!(g.buf.len(), 20 + 4 + 5 + 30 + 3, "recording all along");
}

#[test]
fn without_rewinds_left_only_a_janked_rider_rewinds() {
    let mut g = Sim::new(50);
    (g.m.count.infinite, g.m.count.rewinds_left) = (false, 0);
    assert_eq!(g.press().state, Abort);
    (0..31).for_each(|_| _ = g.exec(OFF));
    assert_eq!(g.exec(OFF).state, Idle);
    g.s.janked.anim_complete = true;
    g.exec(OFF);
    let n = g.buf.len();
    let r = g.press();
    assert_eq!((r.state, g.buf.len()), (Enter, n));
    assert_eq!((g.m.count.rewinds_left, g.m.count.session_start), (0, g.buf.target_time));
    assert!(!g.s.janked.anim_complete, "puppeted by an earlier snapshot");
}

#[test]
fn the_level_start_does_not_arm_the_buffer() {
    let w = world(plane(500.0, 0.0));
    let mut s = rider(&w);
    s.start = Some(crate::levelstart::LevelStart::new(&w, s.traj, glam::Quat::IDENTITY).expect("ground"));
    assert_eq!(s.state, RideState::Riding);
    let mut buf = RewindBuffer::new();
    let flags = RecordFlags { nis_active: true, ..RecordFlags::default() };
    (0..10).for_each(|_| _ = buf.record_with(&s, &flags));
    assert!(buf.is_empty() && !buf.race_proper_started());
    s.start = None;
    buf.record_with(&s, &RecordFlags::default());
    assert!(buf.race_proper_started());
    assert_eq!(buf.len(), 1);
}
