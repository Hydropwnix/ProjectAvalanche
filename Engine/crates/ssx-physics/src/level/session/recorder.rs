use glam::Vec3;
use ssx_formats::ghost::{
    GMPY, GhostFile, POSI, PoseBlock, RECORDER_BUILD, RECORDER_STREAMS, SCOR, StreamConfig, UserHeader,
};

pub const STOP_AFTER_RESULTS_S: f32 = 3.0;

pub fn feed_seconds(ms: u32) -> f32 {
    ms as f32 * 0.001
}

#[derive(Clone, Debug)]
pub struct GhostRecorder {
    file: GhostFile,
    next: [f32; 3],
    start: f32,
    paused: i32,
    stop: f32,
    now: f32,
    latched: bool,
    rewind_block: Option<PoseBlock>,
    pending_pose: Option<f32>,
    incomplete: bool,
}

impl Default for GhostRecorder {
    fn default() -> Self {
        let header = UserHeader { gamertag: String::new(), gear_flags: 0, score: 0, build: RECORDER_BUILD };
        let file = GhostFile::empty(header, &RECORDER_STREAMS).expect("the recorder's streams fit its buffer");
        Self {
            file,
            next: [0.0; 3],
            start: 0.0,
            paused: 0,
            stop: -1.0,
            now: 0.0,
            latched: false,
            rewind_block: None,
            pending_pose: None,
            incomplete: false,
        }
    }
}

impl GhostRecorder {
    pub fn feed(&mut self, ui_ghost_clock_ms: u32) {
        self.now = feed_seconds(ui_ghost_clock_ms);
    }

    pub fn now(&self) -> f32 {
        self.now
    }

    pub fn on_race_start(&mut self) {
        if !self.latched {
            self.restart();
        }
        self.latched = true;
    }

    pub fn on_results(&mut self) {
        self.stop = self.now + STOP_AFTER_RESULTS_S;
        self.latched = false;
    }

    pub fn on_failed(&mut self) {
        self.paused += 1;
        self.latched = false;
    }

    pub fn on_restart(&mut self) {
        self.restart();
        self.latched = false;
    }

    pub fn rewind_node_rebuilt(&mut self) {
        self.rewind_block = None;
    }

    pub fn pause(&mut self, begin: bool) {
        self.paused += if begin { 1 } else { -1 };
    }

    fn restart(&mut self) {
        for s in &mut self.file.streams {
            (s.count, s.times, s.data) = (0, Vec::new(), Vec::new());
        }
        self.next = [0.0; 3];
        self.start = self.now;
        (self.paused, self.stop) = (0, -1.0);
        (self.pending_pose, self.incomplete) = (None, false);
    }

    pub fn sample(&mut self, position: Vec3, points: f32) {
        if self.pending_pose.take().is_some() {
            self.incomplete = true;
        }
        if 0.0 < self.stop && self.stop <= self.now {
            self.stop = -1.0;
            self.final_sample_with(position, points);
        }
        if self.paused != 0 {
            return;
        }
        for k in 0..self.file.streams.len() {
            let s = &self.file.streams[k];
            if self.next[k] <= self.now && s.count < s.capacity {
                self.take(k, position, points);
            }
        }
    }

    fn final_sample_with(&mut self, position: Vec3, points: f32) {
        if self.paused == 0 {
            for k in 0..self.file.streams.len() {
                if self.file.streams[k].count < self.file.streams[k].capacity {
                    self.take(k, position, points);
                }
            }
        }
        self.paused = -1;
    }

    fn config(&self, k: usize) -> StreamConfig {
        RECORDER_STREAMS[k]
    }

    fn take(&mut self, k: usize, position: Vec3, points: f32) {
        let time = self.now - self.start;
        let s = &mut self.file.streams[k];
        match s.tag {
            POSI => {
                let mut e = [0u8; 12];
                for (c, v) in e.as_chunks_mut::<4>().0.iter_mut().zip(position.to_array()) {
                    *c = v.to_be_bytes();
                }
                s.push(time, &e);
            }
            SCOR => {
                s.push(time, &points.to_be_bytes());
            }
            GMPY => self.pending_pose = Some(time),
            _ => {}
        }
        self.next[k] = self.config(k).interval + self.now;
    }

    pub fn after_execution(&mut self, recorded: Option<PoseBlock>) {
        if let Some(b) = recorded {
            self.rewind_block = Some(b);
        }
        let Some(time) = self.pending_pose.take() else { return };
        let Some(gmpy) = self.file.streams.iter_mut().find(|s| s.tag == GMPY) else { return };
        match &self.rewind_block {
            Some(b) => {
                gmpy.push(time, &b.to_bytes());
            }
            None => self.incomplete = true,
        }
    }

    pub fn block_lost(&mut self) {
        self.rewind_block = None;
        self.after_execution(None);
    }

    pub fn incomplete(&self) -> bool {
        self.incomplete
    }

    pub fn wants_block(&self) -> bool {
        self.pending_pose.is_some()
    }

    pub fn halted(&self) -> bool {
        self.paused != 0
    }

    pub fn recording(&self, header: UserHeader) -> Option<GhostFile> {
        if self.incomplete || self.pending_pose.is_some() {
            return None;
        }
        Some(GhostFile { header, ..self.file.clone() })
    }

    pub fn samples(&self) -> u32 {
        self.file.streams.first().map_or(0, |s| s.count)
    }
}

#[cfg(test)]
mod tests;
