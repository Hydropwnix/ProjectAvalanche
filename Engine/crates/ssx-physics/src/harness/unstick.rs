use glam::Vec3;

use super::{CourseLine, locate, steer_towards};
use crate::rider::{RideState, RiderState};

const OFF_KMH: f32 = 50.0;
const AHEAD_M: f32 = 25.0;

#[derive(Clone, Debug)]
pub struct Unstick {
    big_root: std::path::PathBuf,
    start: Vec3,
    end: Option<Vec3>,
    line: Option<Option<CourseLine>>,
    cursor: usize,
    active: bool,
    pub log: Vec<String>,
}

impl Unstick {
    pub fn new(big_root: &std::path::Path, start: Vec3, end: Option<Vec3>) -> Self {
        Unstick { big_root: big_root.into(), start, end, line: None, cursor: 0, active: false, log: Vec::new() }
    }

    pub fn controls(&mut self, s: &RiderState, flick: f32) -> Option<(f32, f32)> {
        let kmh = s.speed() * crate::KMH_PER_M_PER_TICK;
        let ground = s.state == RideState::Riding && s.start.is_none() && !s.janked.janked;
        let was = self.active;
        self.active = ground && !s.landing.is_bailing() && (s.stopped || (self.active && kmh < OFF_KMH));
        if self.active != was {
            self.log.push(format!(
                "standstill help {} at {:.1}, {kmh:.0} km/h{}",
                if self.active { "on" } else { "off" },
                s.traj,
                if s.stopped { " (stopped)" } else { "" }
            ));
        }
        if !self.active {
            return None;
        }
        let (start, end, root) = (self.start, self.end, &self.big_root);
        let line = self
            .line
            .get_or_insert_with(|| end.and_then(|e| CourseLine::nearest_disc(root, start, e).ok()).map(|(l, _, _)| l));
        let steer = match line {
            Some(l) => {
                let (_, along, _) = locate(l, &mut self.cursor, s.traj);
                let (target, _) = l.at(along + AHEAD_M);
                steer_towards(target - s.traj, s.velocity, s.heading)
            }
            None => 0.0,
        };
        Some((steer, if s.stopped { flick } else { 1.0 }))
    }
}
