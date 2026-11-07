use glam::Vec3;

use crate::session::Session;
use crate::triggers::{PLAYER_TOUCH_RADIUS, TriggerVolume};

pub const FINISH_GROUPS: &str = "gameflow";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GateStep {
    pub passed: Option<usize>,
    pub opened: bool,
    pub time_up: bool,
}

#[derive(Clone, Debug)]
pub struct Gates {
    pub checkpoints: Vec<TriggerVolume>,
    pub next: usize,
    pub finish: Option<usize>,
    pub time_limit_ms: Option<u32>,
    pub times: Vec<u32>,
    loops: u32,
}

impl Gates {
    pub fn new(checkpoints: Vec<TriggerVolume>, finish: Option<usize>, time_limit_ms: Option<u32>) -> Self {
        Gates { checkpoints, next: 0, finish, time_limit_ms, times: Vec::new(), loops: 0 }
    }

    pub fn done(&self) -> bool {
        self.next >= self.checkpoints.len()
    }

    fn set_finish(&self, rs: &mut Session, open: bool) {
        let Some(i) = self.finish else { return };
        let closed = &mut rs.tracker.closed;
        closed.retain(|&v| v != i);
        if !open {
            closed.push(i);
        }
    }

    pub fn reset(&mut self, rs: &mut Session) {
        (self.next, self.times) = (0, Vec::new());
        self.loops = rs.event.loops;
        let open = self.done();
        self.set_finish(rs, open);
    }

    pub fn update(&mut self, rs: &mut Session, touch: Vec3) -> GateStep {
        let mut step = GateStep::default();
        if rs.event.loops != self.loops {
            self.reset(rs);
        }
        let running = rs.event.state == crate::event::EventState::Running && !rs.event.waiting;
        let in_run = running && !rs.event_ended && !rs.event.time_up;
        if in_run
            && let Some(v) = self.checkpoints.get(self.next)
            && v.overlaps_sphere(touch, PLAYER_TOUCH_RADIUS)
        {
            step.passed = Some(self.next);
            self.times.push(rs.event.clock.ms());
            self.next += 1;
            if self.done() {
                self.set_finish(rs, true);
                step.opened = true;
            }
        }
        if let Some(limit) = self.time_limit_ms
            && in_run
            && rs.event.clock.ms() >= limit
        {
            rs.event.clock.stop();
            rs.event.time_up = true;
            step.time_up = true;
        }
        step
    }
}
