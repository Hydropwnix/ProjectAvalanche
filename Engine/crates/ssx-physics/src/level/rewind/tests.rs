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
            self.s.score.update_with(t, p, &ScoringInputs::default(), &c, self.dt);
        }
        self.buf.record_with(&self.s, &RecordFlags::default());
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
    let mut c = RewindCount { rewinds_left: 1, session_start: 0.0, enabled: true, infinite: false };
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
