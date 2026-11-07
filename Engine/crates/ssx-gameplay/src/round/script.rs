use ssx_net_proto::InstanceState;

use crate::instance::Instance;

impl Instance {
    pub fn end_round(&mut self, round: u16, now: u32) -> bool {
        if self.round() != round || !matches!(self.state, InstanceState::Countdown | InstanceState::Running) {
            return false;
        }
        let Some(mut c) = self.cycle.take() else { return false };
        let mut out = std::mem::take(&mut c.carry);
        self.enter(&mut c, InstanceState::Results, now, &mut out);
        c.rows_changed = true;
        c.carry = out;
        self.cycle = Some(c);
        true
    }
}
