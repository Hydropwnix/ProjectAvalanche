use std::sync::Arc;

use super::{ADDED_INPUTS, Gates, GestureState, PadSample, TrickHand, TrickType};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AddedTrick {
    pub slot: u8,
    pub base: u8,
    pub hand: TrickHand,
    pub points: u16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AddedAirTricks {
    pub by_input: [Option<AddedTrick>; 3],
}

pub type AddedTrickIds = [Option<Arc<str>>; 3];

impl AddedAirTricks {
    pub fn load(root: Option<&std::path::Path>) -> (Self, AddedTrickIds) {
        let (mut out, mut ids) = (Self::default(), AddedTrickIds::default());
        let Some(routes) = ssx_assets::content::tricks::tricks_for(root) else { return (out, ids) };
        for r in routes.tricks() {
            let t = &r.trick;
            let slot = t.input.index();
            let hand = match t.hand {
                ssx_content::trick::Hand::Rear => TrickHand::Backhand,
                ssx_content::trick::Hand::Front => TrickHand::Forehand,
                ssx_content::trick::Hand::Both => TrickHand::Both,
            };
            if let (Some(s), Some(id)) = (out.by_input.get_mut(slot), ids.get_mut(slot)) {
                *s = Some(AddedTrick { slot: slot as u8, base: t.base_value(), hand, points: t.points });
                *id = Some(Arc::from(r.id.to_string()));
            }
        }
        (out, ids)
    }

    pub fn any(&self) -> bool {
        self.by_input.iter().any(Option::is_some)
    }

    pub fn trick_for(&self, i: usize, goofy: bool) -> Option<TrickType> {
        let at = if goofy { [0, 2, 1].get(i).copied()? } else { i };
        self.by_input.get(at).copied().flatten().map(TrickType::Added)
    }

    fn used(&self, i: usize) -> bool {
        self.trick_for(i, false).is_some() || self.trick_for(i, true).is_some()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AddedGestures {
    pub button_state: [GestureState; 3],
    pub stick_state: [GestureState; 3],
    pub button: [bool; 3],
    pub stick: [bool; 3],
}

impl AddedGestures {
    pub(super) fn tick(&mut self, added: &AddedAirTricks, gates: &Gates, prev: &PadSample, cur: &PadSample) {
        for (i, (button, stick)) in ADDED_INPUTS.iter().enumerate() {
            if added.used(i) {
                self.button[i] = button.update(&mut self.button_state[i], gates, prev, cur);
                self.stick[i] = stick.update(&mut self.stick_state[i], gates, prev, cur);
            }
        }
    }
}
