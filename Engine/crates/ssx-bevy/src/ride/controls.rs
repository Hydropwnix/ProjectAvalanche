use bevy::prelude::*;
use ssx_physics::rewind::RewindPad;
use ssx_physics::rider::{Input, TrickInput};

use super::{Ride, TRIGGER_THRESHOLD};
use crate::camera::CameraMode;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct PadLock {
    prev: bool,
    seen: bool,
    at_pause: bool,
    locked: bool,
}

impl PadLock {
    fn pause(&mut self) {
        self.at_pause = self.seen;
    }

    fn resume(&mut self) {
        self.locked = !self.at_pause;
        self.seen &= !self.locked;
    }

    fn update(&mut self, held: bool) -> bool {
        if held && !self.prev {
            self.locked = false;
        }
        self.prev = held;
        self.seen = held && !self.locked;
        self.seen
    }
}

impl Ride {
    pub fn pause(&mut self) {
        self.hold = true;
        self.paused = true;
        self.a_lock.pause();
        self.session.pause_begin();
    }

    pub fn resume(&mut self) {
        self.hold = false;
        self.paused = false;
        self.session.pause_end();
        self.a_lock.resume();
        if !self.a_lock.seen {
            (self.input.jump, self.tricks.a) = (false, false);
        }
        if self.a_lock.locked {
            info!("Ride step {}: the pause ends, A is locked until its next press", self.steps + 1);
        }
    }

    fn take_live_controls(&mut self, mut input: Input, mut tricks: TrickInput) {
        if !self.a_lock.update(input.jump || tricks.a) {
            (input.jump, tricks.a) = (false, false);
        }
        (self.input, self.tricks) = (input, tricks);
    }

    pub(super) fn locked_controls(&mut self) -> (Input, TrickInput) {
        let (mut input, mut tricks) = (self.input, self.tricks);
        if self.scripted() && !self.a_lock.update(input.jump || tricks.a) {
            (input.jump, tricks.a) = (false, false);
        }
        (input, tricks)
    }
}

pub(super) fn read_controls(
    keys: Res<ButtonInput<KeyCode>>,
    gamepads: Query<&Gamepad>,
    mode: Res<CameraMode>,
    mut ride: ResMut<Ride>,
) {
    if keys.just_pressed(KeyCode::KeyR) && ride.viewer_keys {
        ride.respawn();
    }
    if keys.just_pressed(KeyCode::KeyN) && ride.viewer_keys {
        ride.rail_debug = !ride.rail_debug;
    }
    if ride.scripted() {
        return;
    }
    read_live_controls(&mut ride, &keys, &gamepads, *mode == CameraMode::Chase);
}

pub fn read_live_controls(ride: &mut Ride, keys: &ButtonInput<KeyCode>, gamepads: &Query<&Gamepad>, chase: bool) {
    let held = keys.pressed(KeyCode::Backspace) || gamepads.iter().any(|p| p.pressed(GamepadButton::LeftTrigger));
    ride.back_held = keys.pressed(KeyCode::KeyC) || gamepads.iter().any(|p| p.pressed(GamepadButton::Select));
    ride.rewind_pad = RewindPad { rewind_held: held, takeover: rewind_takeover(gamepads), flip_accel: 0.0 };
    let mut input = live_input(keys, gamepads, chase);
    input.jump |= ride.state.start.is_some() && keys.pressed(KeyCode::Enter);
    let tricks = live_tricks(keys, gamepads, chase, input.jump);
    ride.l2 = live_l2(keys, gamepads, chase);
    ride.r1 = live_r1(keys, gamepads, chase);
    let pads = gamepads.iter().map(|p| p.get(GamepadButton::RightTrigger2).unwrap_or(0.0)).fold(0.0, f32::max);
    ride.r2 = if tricks.tweak && pads < TRIGGER_THRESHOLD { 1.0 } else { pads };
    ride.take_live_controls(input, tricks);
}

pub(super) fn late_controls(
    keys: Res<ButtonInput<KeyCode>>,
    gamepads: Query<&Gamepad>,
    mode: Res<CameraMode>,
    mut ride: ResMut<Ride>,
) {
    if ride.scripted() {
        return;
    }
    let mut input = live_input(&keys, &gamepads, *mode == CameraMode::Chase);
    input.jump |= ride.state.start.is_some() && keys.pressed(KeyCode::Enter);
    let a = input.jump || live_tricks(&keys, &gamepads, *mode == CameraMode::Chase, input.jump).a;
    if a != ride.a_lock.prev {
        ride.a_lock.update(a);
    }
}

fn rewind_takeover(gamepads: &Query<&Gamepad>) -> bool {
    use std::f32::consts::{FRAC_PI_4, TAU};
    let direction = |s: Vec2, thresholds: [f32; 4]| {
        let a = s.y.atan2(s.x).rem_euclid(TAU);
        let sector = if (FRAC_PI_4..=3.0 * FRAC_PI_4).contains(&a) {
            0
        } else if (3.0 * FRAC_PI_4..=5.0 * FRAC_PI_4).contains(&a) {
            1
        } else if (5.0 * FRAC_PI_4..=7.0 * FRAC_PI_4).contains(&a) {
            2
        } else {
            3
        };
        s.length().min(1.0) >= thresholds[sector]
    };
    gamepads.iter().any(|p| {
        let buttons = [
            GamepadButton::West,
            GamepadButton::North,
            GamepadButton::South,
            GamepadButton::RightTrigger,
            GamepadButton::RightTrigger2,
            GamepadButton::LeftTrigger2,
        ];
        buttons.iter().any(|&b| p.pressed(b))
            || direction(p.left_stick(), [0.57, 0.47, 0.47, 0.46])
            || direction(p.right_stick(), [0.54, 0.49, 0.49, 0.52])
    })
}

fn live_input(keys: &ButtonInput<KeyCode>, gamepads: &Query<&Gamepad>, wasd: bool) -> Input {
    let held =
        |arrow: KeyCode, letter: KeyCode| f32::from(u8::from(keys.pressed(arrow) || (wasd && keys.pressed(letter))));
    let mut steer = held(KeyCode::ArrowRight, KeyCode::KeyD) - held(KeyCode::ArrowLeft, KeyCode::KeyA);
    let mut brake = held(KeyCode::ArrowDown, KeyCode::KeyS);
    let mut tuck = held(KeyCode::ArrowUp, KeyCode::KeyW);
    let mut jump = keys.pressed(KeyCode::Space);
    for pad in gamepads {
        let stick = pad.left_stick();
        steer = (steer + stick.x).clamp(-1.0, 1.0);
        tuck = tuck.max(stick.y);
        brake = brake.max(-stick.y);
        jump |= pad.pressed(GamepadButton::South);
    }
    Input { steer, brake, tuck, jump }
}

fn live_l2(keys: &ButtonInput<KeyCode>, gamepads: &Query<&Gamepad>, chase: bool) -> bool {
    let ctrl = chase && keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    ctrl || gamepads.iter().any(|p| {
        p.get(GamepadButton::LeftTrigger2).unwrap_or(0.0) >= TRIGGER_THRESHOLD || p.pressed(GamepadButton::LeftTrigger2)
    })
}

fn live_r1(keys: &ButtonInput<KeyCode>, gamepads: &Query<&Gamepad>, chase: bool) -> bool {
    (chase && keys.pressed(KeyCode::KeyE)) || gamepads.iter().any(|p| p.pressed(GamepadButton::RightTrigger))
}

fn live_tricks(keys: &ButtonInput<KeyCode>, gamepads: &Query<&Gamepad>, chase: bool, jump: bool) -> TrickInput {
    let shift = chase && keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let ctrl = chase && keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    let mut t = TrickInput {
        right_stick: Vec2::ZERO,
        y: keys.pressed(KeyCode::KeyI),
        x: keys.pressed(KeyCode::KeyJ),
        b: keys.pressed(KeyCode::KeyL),
        a: keys.pressed(KeyCode::KeyK) || jump,
        tweak: shift,
        signature: shift && ctrl,
    };
    for pad in gamepads {
        let stick = pad.right_stick();
        if stick.length() > t.right_stick.length() {
            t.right_stick = stick;
        }
        let trigger = |b: GamepadButton| pad.get(b).unwrap_or(0.0) >= TRIGGER_THRESHOLD || pad.pressed(b);
        let (l2, r2) = (trigger(GamepadButton::LeftTrigger2), trigger(GamepadButton::RightTrigger2));
        t.y |= pad.pressed(GamepadButton::North);
        t.x |= pad.pressed(GamepadButton::West);
        t.b |= pad.pressed(GamepadButton::East);
        t.tweak |= r2;
        t.signature |= l2 && r2;
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pause_locks_a_button_pressed_during_it() {
        let mut l = PadLock::default();
        assert!(!l.update(false) && l.update(true) && l.update(true), "no pause: A as given");
        assert!(!l.update(false));
        l.pause();
        assert!(l.update(true), "the popup's A, read during the pause: its edge is spent here");
        l.resume();
        assert!(!l.seen, "out of the held word at the pause's end");
        assert!(!l.update(true) && !l.update(true), "held through the end: locked");
        assert!(!l.update(false));
        assert!(l.update(true), "the next press edge is seen");
        let mut held = PadLock::default();
        assert!(held.update(true));
        held.pause();
        held.resume();
        assert!(held.seen && held.update(true), "held when the pause began: not locked");
        let mut late = PadLock::default();
        late.pause();
        late.resume();
        assert!(!late.update(false));
        assert!(late.update(true), "first read after the end: seen at once");
    }

    fn popup_answer(hold: f64, fps: f64, injected: bool, fe_frames: u32, late: bool) -> (Vec<u32>, bool) {
        #[derive(Clone, Copy, Default)]
        struct Key {
            down: bool,
            just: bool,
        }
        let set = |k: &mut Key, down: bool| {
            (k.just, k.down) = (k.just || (down && !k.down), down);
        };
        let (press, release) = (1.0, 1.0 + hold);
        let (again, end) = (release + 0.5, release + 1.0);
        let want = |t: f64| (press..release).contains(&t) || (again..end).contains(&t);
        let (mut lock, mut key) = (PadLock::default(), Key::default());
        let (mut popup, mut held, mut answered) = (true, false, None::<u32>);
        let (mut early, mut second, mut steps, mut carry) = (Vec::new(), false, 0u32, 0.0);
        for frame in 0.. {
            let t = f64::from(frame) / fps;
            if t > end + 0.2 {
                break;
            }
            key.just = false;
            if !injected {
                set(&mut key, want(t));
            }
            if frame == 2 {
                lock.pause();
                held = true;
            }
            if held && !popup {
                lock.resume();
                held = false;
            }
            carry += 1.0 / fps;
            while carry >= 1.0 / 30.0 {
                carry -= 1.0 / 30.0;
                if !held {
                    steps += 1;
                    if lock.seen && t < again {
                        early.push(steps);
                    }
                    second |= lock.seen && t >= again;
                }
            }
            lock.update(key.down);
            if injected {
                set(&mut key, want(t));
            }
            if popup && key.just {
                answered.get_or_insert(frame + fe_frames);
            }
            popup &= answered.is_none_or(|f| frame < f);
            if late && key.down != lock.prev {
                lock.update(key.down);
            }
        }
        assert!(!popup && !held && steps > 30, "the popup was answered and the ride went on");
        (early, second)
    }

    #[test]
    fn the_popups_answer_does_not_reach_the_ride() {
        for hold in [0.02, 0.1, 0.6] {
            for fps in [20.0, 30.0, 60.0, 75.0, 144.0, 300.0] {
                for (injected, fe_frames) in [(false, 0), (false, 1), (false, 3), (true, 0), (true, 1), (true, 3)] {
                    let (early, second) = popup_answer(hold, fps, injected, fe_frames, true);
                    let case = format!("{hold} s at {fps} fps, injected {injected}, the front end {fe_frames} later");
                    assert!(early.is_empty(), "{case}: A seen on steps {early:?}");
                    assert!(second, "{case}: the second press was not seen");
                }
            }
        }
        let (early, _) = popup_answer(0.1, 60.0, true, 0, false);
        assert_eq!(early.first(), Some(&3), "one read per frame: {early:?}");
    }
}
