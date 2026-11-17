use super::*;
use crate::event::EventClock;
use ssx_formats::ghost::{self, POSE_BLOCK_SIZE};

fn block(p: Vec3) -> PoseBlock {
    let mut b = [0u8; POSE_BLOCK_SIZE];
    b[12..16].copy_from_slice(&1.0f32.to_be_bytes());
    for (k, v) in p.to_array().into_iter().enumerate() {
        b[0x10 + 4 * k..0x14 + 4 * k].copy_from_slice(&v.to_be_bytes());
    }
    PoseBlock::parse(&b).expect("a pose block")
}

struct Run {
    rec: GhostRecorder,
    clock: EventClock,
    executions: u32,
}

impl Run {
    fn new() -> Self {
        Self { rec: GhostRecorder::default(), clock: EventClock::default(), executions: 0 }
    }

    fn execute(&mut self, race_start: bool, y: f32) {
        if race_start {
            self.rec.on_race_start();
            self.clock.restart();
        }
        let p = Vec3::new(1.0, y, 2.0);
        self.rec.sample(p, -y);
        self.rec.after_execution(Some(block(p)));
        self.clock.advance(crate::DT_TICKS / crate::TICKS_PER_SECOND);
        self.rec.feed(self.clock.ms());
        self.executions += 1;
    }

    fn times(&self) -> Vec<f32> {
        self.rec.file.streams[0].times.clone()
    }
}

#[test]
fn the_samples_fall_every_third_of_a_second_on_the_truncated_clock() {
    let mut run = Run::new();
    run.execute(true, 100.0);
    for k in 1..300 {
        run.execute(false, 100.0 - k as f32);
    }
    let times = run.times();
    assert_eq!(times[0], 0.0);
    for w in times.windows(2) {
        let d = w[1] - w[0];
        assert!((0.333..0.368).contains(&d), "step {d}");
    }
    assert!((28..=30).contains(&times.len()), "{} samples", times.len());
    for s in &run.rec.file.streams {
        assert_eq!(s.times, times, "{:?}", s.tag);
    }
    let f = run.rec.recording(UserHeader { gamertag: "Me".into(), gear_flags: 0, score: 1, build: RECORDER_BUILD });
    let f = f.expect("complete");
    let pos = f.positions().expect("POSI");
    let scores = f.scores().expect("SCOR");
    assert_eq!(pos[1], [1.0, 100.0 - 11.0, 2.0]);
    assert_eq!(scores[1], -(100.0 - 11.0));
}

#[test]
fn the_results_stop_it_three_seconds_on_with_a_last_sample_of_every_stream() {
    let mut run = Run::new();
    run.execute(true, 0.0);
    for _ in 0..59 {
        run.execute(false, 0.0);
    }
    let now = run.rec.now();
    run.rec.on_results();
    assert!(!run.rec.halted());
    while run.rec.now() < now + STOP_AFTER_RESULTS_S {
        run.execute(false, 0.0);
    }
    let before = run.times().len();
    run.execute(false, 0.0);
    let times = run.times();
    assert_eq!(times.len(), before + 1);
    assert!((times[times.len() - 1] - (now + 3.0)).abs() < 0.04);
    assert!(run.rec.halted());
    for _ in 0..60 {
        run.execute(false, 0.0);
    }
    assert_eq!(run.times().len(), before + 1);
    assert!(run.rec.recording(empty_header()).is_some());
}

fn empty_header() -> UserHeader {
    UserHeader { gamertag: String::new(), gear_flags: 0, score: 0, build: RECORDER_BUILD }
}

#[test]
fn the_failed_results_halt_it_without_a_last_sample() {
    let mut run = Run::new();
    run.execute(true, 0.0);
    for _ in 0..30 {
        run.execute(false, 0.0);
    }
    run.rec.on_failed();
    let n = run.times().len();
    for _ in 0..200 {
        run.execute(false, 0.0);
    }
    assert_eq!(run.times().len(), n);
    assert!(run.rec.halted());
}

#[test]
fn a_second_race_start_keeps_the_recording_until_the_results_clear_the_latch() {
    let mut run = Run::new();
    run.execute(true, 0.0);
    for _ in 0..30 {
        run.execute(false, 0.0);
    }
    let n = run.times().len();
    run.execute(true, 0.0);
    assert!(run.times().len() >= n, "the latch holds: no new start");
    run.rec.on_results();
    run.execute(true, 0.0);
    assert_eq!(run.times(), vec![0.0], "the results cleared the latch: a new recording");
}

#[test]
fn the_pause_holds_it_and_a_restart_starts_over() {
    let mut run = Run::new();
    run.execute(true, 0.0);
    run.rec.pause(true);
    for _ in 0..40 {
        run.execute(false, 0.0);
    }
    assert_eq!(run.times().len(), 1);
    assert!(run.rec.halted(), "the save task's wait ends in a pause too");
    run.rec.pause(false);
    run.execute(false, 0.0);
    assert_eq!(run.times().len(), 2);
    run.rec.on_restart();
    assert!(run.times().is_empty());
    run.execute(true, 0.0);
    assert_eq!(run.times(), vec![0.0]);
}

#[test]
fn a_restart_before_the_stop_leaves_the_save_waiting_for_the_next_run() {
    let mut run = Run::new();
    run.execute(true, 0.0);
    for _ in 0..30 {
        run.execute(false, 0.0);
    }
    run.rec.on_results();
    run.execute(false, 0.0);
    run.rec.on_restart();
    assert!(!run.rec.halted());
    run.execute(true, 5.0);
    for _ in 0..200 {
        run.execute(false, 5.0);
    }
    assert!(!run.rec.halted());
    assert_eq!(run.times()[0], 0.0);
    run.rec.pause(true);
    assert!(run.rec.halted());
    let f = run.rec.recording(empty_header()).expect("complete");
    assert!(f.positions().expect("POSI").iter().all(|p| p[1] == 5.0), "the restarted run's samples only");
}

#[test]
fn a_sample_without_its_block_leaves_the_recording_unsaved() {
    let mut rec = GhostRecorder::default();
    rec.on_race_start();
    rec.sample(Vec3::ZERO, 0.0);
    assert!(rec.wants_block());
    rec.after_execution(None);
    assert!(rec.recording(empty_header()).is_none());
    let mut rec = GhostRecorder::default();
    rec.after_execution(Some(block(Vec3::ONE)));
    rec.on_race_start();
    rec.sample(Vec3::ZERO, 0.0);
    rec.feed(400);
    rec.sample(Vec3::ZERO, 0.0);
    assert!(rec.recording(empty_header()).is_none());
}

#[test]
fn an_execution_without_a_rewind_record_samples_the_last_block() {
    let mut rec = GhostRecorder::default();
    rec.after_execution(Some(block(Vec3::new(5.0, 6.0, 7.0))));
    rec.on_race_start();
    rec.sample(Vec3::ZERO, 0.0);
    rec.after_execution(None);
    let f = rec.recording(empty_header()).expect("complete");
    assert_eq!(f.poses().expect("GMPY")[0].trajectory_position, [5.0, 6.0, 7.0]);
}

#[test]
fn a_restart_samples_the_new_rewind_node_only() {
    let mut rec = GhostRecorder::default();
    rec.after_execution(Some(block(Vec3::new(5.0, 6.0, 7.0))));
    rec.on_restart();
    rec.rewind_node_rebuilt();
    rec.on_race_start();
    rec.sample(Vec3::ZERO, 0.0);
    rec.after_execution(None);
    assert!(rec.recording(empty_header()).is_none(), "the old node's block is not the new node's");

    let mut rec = GhostRecorder::default();
    rec.after_execution(Some(block(Vec3::new(5.0, 6.0, 7.0))));
    rec.on_restart();
    rec.rewind_node_rebuilt();
    rec.on_race_start();
    rec.sample(Vec3::ZERO, 0.0);
    rec.after_execution(Some(block(Vec3::new(1.0, 2.0, 3.0))));
    let f = rec.recording(empty_header()).expect("complete");
    assert_eq!(f.poses().expect("GMPY")[0].trajectory_position, [1.0, 2.0, 3.0]);
}

#[test]
fn a_recording_round_trips_through_the_save_format() {
    let mut run = Run::new();
    run.execute(true, 500.0);
    for k in 1..900 {
        run.execute(false, 500.0 - 0.5 * k as f32);
    }
    run.rec.on_results();
    for _ in 0..120 {
        run.execute(false, 0.0);
    }
    let header = UserHeader { gamertag: "Rider".into(), gear_flags: 3, score: 30_000, build: RECORDER_BUILD };
    let f = run.rec.recording(header).expect("complete");
    let image = f.write().expect("the image");
    let container = ghost::compress(&image).expect("GCMP");
    let back = ghost::read_any(&container).expect("read back");
    assert_eq!(back, GhostFile { crc: back.crc, ..f });
    assert_eq!(back.write().expect("again"), image);
}
