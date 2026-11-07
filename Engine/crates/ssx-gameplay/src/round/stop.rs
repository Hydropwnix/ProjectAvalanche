use ssx_net_proto::InstanceState;

use super::{Changes, Cycle};
use crate::instance::Instance;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopPlan {
    Idle,
    RoundEnds { round: u16, by: u32 },
}

impl Instance {
    pub fn stop(&mut self, now: u32, within: u32) -> StopPlan {
        let state = self.state;
        let round = self.round();
        let Some(c) = self.cycle.as_mut() else { return StopPlan::Idle };
        let by = *c.stop.get_or_insert(now.saturating_add(within));
        match state {
            InstanceState::Countdown | InstanceState::Running => StopPlan::RoundEnds { round, by },
            InstanceState::Lobby | InstanceState::Loading | InstanceState::Results => StopPlan::Idle,
        }
    }

    pub fn stop_done(&self, now: u32, shown: u32) -> bool {
        let Some(c) = self.cycle.as_ref() else { return true };
        if c.stop.is_none() {
            return false;
        }
        match self.state {
            InstanceState::Lobby | InstanceState::Loading => true,
            InstanceState::Countdown | InstanceState::Running => false,
            InstanceState::Results => now.saturating_sub(c.since) >= shown,
        }
    }

    pub(super) fn stop_step(&mut self, c: &mut Cycle, now: u32, out: &mut Changes) -> bool {
        let Some(by) = c.stop else { return false };
        match self.state {
            InstanceState::Lobby | InstanceState::Loading | InstanceState::Results => true,
            InstanceState::Countdown | InstanceState::Running if now >= by => {
                if self.state == InstanceState::Running {
                    self.end_at_limit(c);
                }
                self.enter(c, InstanceState::Results, now, out);
                c.rows_changed = true;
                true
            }
            InstanceState::Countdown | InstanceState::Running => false,
        }
    }
}
