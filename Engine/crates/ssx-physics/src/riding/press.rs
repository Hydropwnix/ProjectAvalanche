use std::f32::consts::PI;

use anyhow::Result;

use crate::curve::Curve;
use crate::tricks::{Button, GestureState, MatchResult, MatchType, Matcher, PadSample, Step, Test, gesture_match};
use crate::tuning::Bundle;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoardPressTuning {
    pub blend_gain: f32,
    pub stop_speed: f32,
    pub start_speed: f32,
    pub max_lean_input_curve: Curve,
    pub force_stop_spring_stiffness: f32,
}

impl BoardPressTuning {
    pub fn load(bundle: &Bundle) -> Result<Self> {
        let c = bundle.view("carve_riding", "CarvingControllerAsset")?;
        Ok(Self {
            blend_gain: c.f32("BoardPressBlendValueGain")?,
            stop_speed: c.f32("BoardPressStopSpeed")?,
            start_speed: c.f32("BoardPressStartSpeed")?,
            max_lean_input_curve: c.curve("SpeedVsMaxLeanInputAngleCurveBoardPressAsset")?,
            force_stop_spring_stiffness: c.f32("ForceStopSpringStiffness")?,
        })
    }

    pub fn data() -> Self {
        Self {
            blend_gain: 0.1,
            stop_speed: 0.046_296_295,
            start_speed: 0.092_592_59,
            max_lean_input_curve: Curve::new(vec![
                [0.0, 0.089_080_796],
                [0.148_786_7, 0.106_896_93],
                [0.925_925_9, 0.302_875_2],
            ]),
            force_stop_spring_stiffness: 0.01,
        }
    }
}

const NOSE_STEPS: [Step; 1] = [Step {
    matchers: &[Matcher { test: Test::Button(Button::X), kind: MatchType::Hold, negate: false }],
    min_ticks: 0.0,
    max_ticks: 1_073_741_824.0,
}];
const TAIL_MATCHER: Matcher = Matcher { test: Test::Button(Button::B), kind: MatchType::Hold, negate: false };

const CLASSIC_NOSE: Matcher = Matcher {
    test: Test::Stick { min_angle: 0.0, max_angle: PI, min_mag: 0.25, max_mag: 1.0 },
    kind: MatchType::Hold,
    negate: false,
};
const CLASSIC_TAIL: Matcher = Matcher {
    test: Test::Stick { min_angle: PI, max_angle: 0.0, min_mag: 0.25, max_mag: 1.0 },
    kind: MatchType::Hold,
    negate: false,
};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PressGestures {
    pub nose_state: GestureState,
    pub nose: bool,
    pub tail: bool,
}

impl PressGestures {
    pub fn tick(&mut self, prev: &PadSample, cur: &PadSample, classic: bool) {
        if classic {
            self.nose_state = GestureState::default();
            self.nose = CLASSIC_NOSE.eval(prev, cur) == MatchResult::Match;
            self.tail = CLASSIC_TAIL.eval(prev, cur) == MatchResult::Match;
            return;
        }
        self.nose = match gesture_match(&NOSE_STEPS, &mut self.nose_state, prev, cur, 1.0) {
            MatchResult::Match => true,
            MatchResult::Pending => false,
            MatchResult::Fail => {
                self.nose_state = GestureState::default();
                false
            }
        };
        self.tail = TAIL_MATCHER.eval(prev, cur) == MatchResult::Match;
    }

    pub fn inputs(&self) -> PressInputs {
        PressInputs { nose: self.nose, tail: self.tail }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PressInputs {
    pub nose: bool,
    pub tail: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CarvingPress {
    pub blend: f32,
    pub force_stop_mag: f32,
    pub force_stop_vel: f32,
    pub pressing: bool,
    pub inputs: PressInputs,
    pub is_board_press: bool,
}

impl CarvingPress {
    pub fn ctor(&mut self) {
        self.blend = 0.0;
    }
}

#[allow(clippy::too_many_arguments)]
pub fn board_press(
    t: &BoardPressTuning,
    i: PressInputs,
    grind_state: u32,
    speed: f32,
    was_pressing: bool,
    backslide: bool,
    force_stop: bool,
    wiping_out: bool,
) -> bool {
    let threshold = if was_pressing { t.stop_speed } else { t.start_speed };
    (i.nose || i.tail) && grind_state == 0 && speed > threshold && !backslide && !force_stop && !wiping_out
}

pub fn blend_update(blend: f32, pressing: bool, gain: f32, dt: f32) -> f32 {
    let target = if pressing { 1.0 } else { 0.0 };
    target + (blend - target) * (-gain * dt).exp()
}

pub fn grind_or_press(grind: f32, press: f32) -> f32 {
    if grind - press >= 0.0 { grind } else { press }
}

pub fn is_board_press_output(pressing: bool, grind_or_press_blend: f32) -> bool {
    pressing && grind_or_press_blend > 0.1
}

pub fn force_stop_update(p: &mut CarvingPress, stiffness: f32, can_brake: bool, level_complete: bool, dt: f32) {
    if can_brake && level_complete {
        (p.force_stop_mag, p.force_stop_vel) =
            crate::rider::grind::damped_spring(stiffness, p.force_stop_mag, p.force_stop_vel, 1.0, dt);
    } else {
        (p.force_stop_mag, p.force_stop_vel) = (0.0, 0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pad(x: bool, b: bool) -> PadSample {
        PadSample { x, b, ..PadSample::default() }
    }

    #[test]
    fn x_and_b_held_are_the_nose_and_tail_presses() {
        let mut g = PressGestures::default();
        g.tick(&pad(false, false), &pad(true, false), false);
        assert!(g.nose && !g.tail, "X held: nose from the first tick");
        for _ in 0..200 {
            g.tick(&pad(true, false), &pad(true, false), false);
            assert!(g.nose, "an open-ended step keeps matching while held");
        }
        g.tick(&pad(true, false), &pad(false, true), false);
        assert!(!g.nose && g.tail);
        g.tick(&pad(false, true), &pad(false, false), false);
        assert!(!g.nose && !g.tail);
    }

    #[test]
    fn classic_press_reads_the_right_sticks_upper_and_lower_halves() {
        let up = PadSample { angle: std::f32::consts::FRAC_PI_2, mag: 0.5, ..PadSample::default() };
        let down = PadSample { angle: 3.0 * std::f32::consts::FRAC_PI_2, mag: 0.5, ..PadSample::default() };
        let weak = PadSample { angle: std::f32::consts::FRAC_PI_2, mag: 0.24, ..PadSample::default() };
        let mut g = PressGestures::default();
        g.tick(&PadSample::default(), &up, true);
        assert!(g.nose && !g.tail);
        g.tick(&up, &down, true);
        assert!(!g.nose && g.tail);
        g.tick(&down, &weak, true);
        assert!(!g.nose && !g.tail, "below the classic press's 0.25 magnitude");
        g.tick(&PadSample::default(), &PadSample { x: true, ..PadSample::default() }, false);
        assert!(g.nose && !g.tail);
    }

    #[test]
    fn the_press_needs_speed_with_hysteresis_and_no_grind() {
        let t = BoardPressTuning::data();
        let nose = PressInputs { nose: true, tail: false };
        let kmh = |v: f32| v / crate::KMH_PER_M_PER_TICK;
        assert!(!board_press(&t, nose, 0, kmh(15.0), false, false, false, false), "below 20 km/h to start");
        assert!(board_press(&t, nose, 0, kmh(25.0), false, false, false, false));
        assert!(board_press(&t, nose, 0, kmh(15.0), true, false, false, false), "kept down to 10 km/h");
        assert!(!board_press(&t, nose, 0, kmh(9.0), true, false, false, false));
        assert!(!board_press(&t, nose, 2, kmh(50.0), true, false, false, false), "not while grinding");
        assert!(!board_press(&t, nose, 0, kmh(50.0), true, false, true, false), "not force-stopped");
        assert!(!board_press(&t, PressInputs::default(), 0, kmh(50.0), true, false, false, false));
    }

    #[test]
    fn blend_and_output() {
        let mut b = 0.0;
        for _ in 0..3 {
            b = blend_update(b, true, 0.1, 2.0);
        }
        assert!((b - (1.0 - (-0.6f32).exp())).abs() < 1e-6);
        assert!(!is_board_press_output(true, 0.05) && is_board_press_output(true, 0.2));
        assert_eq!(grind_or_press(0.3, 0.7), 0.7);
        let mut p = CarvingPress::default();
        for _ in 0..60 {
            force_stop_update(&mut p, 0.01, true, true, 2.0);
        }
        assert!(p.force_stop_mag > 0.9 && p.force_stop_mag <= 1.0, "{}", p.force_stop_mag);
        force_stop_update(&mut p, 0.01, false, true, 2.0);
        assert_eq!((p.force_stop_mag, p.force_stop_vel), (0.0, 0.0));
    }
}
