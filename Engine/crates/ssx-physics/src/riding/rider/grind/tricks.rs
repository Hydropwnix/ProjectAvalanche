use crate::tricks::TrickType;
use crate::tricks::{
    Button, Gesture, GestureState, MatchResult, MatchType, Matcher, PadSample, Step, Test, gesture_match,
};

const OPEN: f32 = 1_073_741_824.0;

const fn held(b: Button) -> Matcher {
    Matcher { test: Test::Button(b), kind: MatchType::Hold, negate: false }
}
const fn on(b: Button, kind: MatchType) -> Matcher {
    Matcher { test: Test::Button(b), kind, negate: false }
}

macro_rules! double_tap {
    ($b:expr) => {
        [
            Step { matchers: &[on($b, MatchType::Enter)], min_ticks: 0.0, max_ticks: 0.0 },
            Step { matchers: &[on($b, MatchType::Exit)], min_ticks: 0.0, max_ticks: 4.0 },
            Step { matchers: &[held($b)], min_ticks: 0.0, max_ticks: 4.0 },
            Step { matchers: &[held($b)], min_ticks: 0.0, max_ticks: OPEN },
        ]
    };
}
const X2: [Step; 4] = double_tap!(Button::X);
const B2: [Step; 4] = double_tap!(Button::B);
const Y2: [Step; 4] = double_tap!(Button::Y);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GrindEntry {
    pub gesture: Gesture,
    pub len: i32,
    pub plain: u8,
    pub group: i32,
}

pub const GRIND_TRICKS: [GrindEntry; 6] = [
    GrindEntry { gesture: Gesture::Matcher(held(Button::X)), len: 0, plain: 14, group: 0 },
    GrindEntry { gesture: Gesture::Matcher(held(Button::B)), len: 0, plain: 17, group: 1 },
    GrindEntry { gesture: Gesture::Matcher(held(Button::Y)), len: 0, plain: 20, group: 2 },
    GrindEntry { gesture: Gesture::Steps(&X2), len: 1, plain: 23, group: 0 },
    GrindEntry { gesture: Gesture::Steps(&B2), len: 1, plain: 26, group: 1 },
    GrindEntry { gesture: Gesture::Steps(&Y2), len: 1, plain: 29, group: 2 },
];

impl GrindEntry {
    pub fn trick(&self, awesome: bool, super_awesome: bool) -> TrickType {
        let k = if super_awesome {
            2
        } else if awesome {
            1
        } else {
            0
        };
        TrickType::grind(self.plain + k).unwrap_or_default()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GrindNode {
    #[default]
    None,
    Enter,
    DoubleTapEnter,
    Loop,
    Exit,
}

pub const ENTER_END_TICKS: f32 = 40.0;
pub const DOUBLE_TAP_WINDOW_TICKS: f32 = 15.0;
pub const EXIT_END_TICKS: f32 = 30.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GrindTricks {
    pub states: [GestureState; 6],
    pub button: [bool; 6],
    pub group: i32,
    pub group_locked: i32,
    pub gesture_len_locked: i32,
    pub index: i32,
    pub starting: bool,
    pub enabled: bool,
    pub node: GrindNode,
    pub node_ticks: f32,
    pub exit_for_wingsuit: bool,
}

impl Default for GrindTricks {
    fn default() -> Self {
        Self {
            states: [GestureState::default(); 6],
            button: [false; 6],
            group: -1,
            group_locked: -1,
            gesture_len_locked: -1,
            index: -1,
            starting: false,
            enabled: true,
            node: GrindNode::None,
            node_ticks: 0.0,
            exit_for_wingsuit: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FlowEvents {
    pub grab_reset: bool,
    pub ground_trick_reset: bool,
}

impl GrindTricks {
    pub fn is_grind_trick(&self) -> bool {
        self.node != GrindNode::None
    }

    pub fn is_grind_trick_enter(&self) -> bool {
        matches!(self.node, GrindNode::Enter | GrindNode::DoubleTapEnter)
    }

    pub fn ending(&self) -> bool {
        self.node == GrindNode::Exit
    }

    pub fn tick_gestures(&mut self, prev: &PadSample, cur: &PadSample, grinding: bool) {
        for (i, e) in GRIND_TRICKS.iter().enumerate() {
            let s = &mut self.states[i];
            let r = if grinding {
                match e.gesture {
                    Gesture::Matcher(m) => m.eval(prev, cur),
                    Gesture::Steps(steps) => gesture_match(steps, s, prev, cur, 1.0),
                }
            } else {
                MatchResult::Fail
            };
            if r == MatchResult::Fail {
                *s = GestureState::default();
            }
            self.button[i] = r == MatchResult::Match;
        }
    }

    pub fn enter(&mut self) {
        (self.node, self.node_ticks) = (GrindNode::Enter, 0.0);
    }

    pub fn flow(&mut self, grinding: bool, double: bool, pad_grab_end: bool, dt: f32) -> FlowEvents {
        let mut ev = FlowEvents::default();
        let mut rest = dt;
        for _ in 0..crate::tricks::MAX_TRANSITIONS_PER_UPDATE {
            match self.node {
                GrindNode::None => break,
                GrindNode::Enter if double && self.node_ticks <= DOUBLE_TAP_WINDOW_TICKS => {
                    self.node = GrindNode::DoubleTapEnter;
                    continue;
                }
                GrindNode::Loop if pad_grab_end || !grinding => {
                    (self.node, self.node_ticks) = (GrindNode::Exit, 0.0);
                    ev.grab_reset = true;
                    self.exit_for_wingsuit = true;
                    continue;
                }
                _ => {}
            }
            let end = match self.node {
                GrindNode::Enter | GrindNode::DoubleTapEnter => ENTER_END_TICKS,
                GrindNode::Exit => EXIT_END_TICKS,
                _ => f32::INFINITY,
            };
            let due = end - self.node_ticks;
            if due >= rest {
                self.node_ticks += rest;
                break;
            }
            rest -= due.max(0.0);
            match self.node {
                GrindNode::Exit => {
                    (self.node, self.node_ticks) = (GrindNode::None, 0.0);
                    ev.ground_trick_reset = true;
                    break;
                }
                _ => (self.node, self.node_ticks) = (GrindNode::Loop, 0.0),
            }
        }
        ev
    }

    pub fn abort(&mut self) -> bool {
        let was = self.node != GrindNode::None;
        (self.node, self.node_ticks) = (GrindNode::None, 0.0);
        was
    }
}
