use crate::avm1::Ctx;
use crate::avm1::value::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Start,
    DigitalOne,
    DigitalTwo,
    DigitalThree,
    DigitalFour,
    LeftTrigger,
    RightTrigger,
    LeftBumper,
    RightBumper,
    Select,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
    Disconnect,
}

impl Button {
    pub fn code_name(self) -> &'static str {
        match self {
            Button::Start => "START",
            Button::DigitalOne => "DIGITAL_ONE",
            Button::DigitalTwo => "DIGITAL_TWO",
            Button::DigitalThree => "DIGITAL_THREE",
            Button::DigitalFour => "DIGITAL_FOUR",
            Button::LeftTrigger => "LEFT_TRIGGER",
            Button::RightTrigger => "RIGHT_TRIGGER",
            Button::LeftBumper => "LEFT_BUMPER",
            Button::RightBumper => "RIGHT_BUMPER",
            Button::Select => "SELECT",
            Button::DpadUp => "DPAD_UP",
            Button::DpadDown => "DPAD_DOWN",
            Button::DpadLeft => "DPAD_LEFT",
            Button::DpadRight => "DPAD_RIGHT",
            Button::Disconnect => "DISCONNECT",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Input {
    Button { button: Button, down: bool },
}

pub const PADS: usize = 4;

pub const SCRIPT_DEVICE: u8 = 0;

// TODO(guess): a port choice — the PC keyboard is device 4, the first keyboard after the four pads (0x831789E8; the UI manager's keyboard handler 0x82576E58 is not read); settle: none (the player's keyboard is the port's).
pub const KEYBOARD_DEVICE: u8 = 4;

#[derive(Clone, Debug, Default)]
pub struct InputProvider {
    pub subscribers: Vec<String>,
    queue: Vec<(Input, u8)>,
}

impl InputProvider {
    pub fn subscribe(&mut self, path: &str) {
        if !self.subscribers.iter().any(|p| p == path) {
            self.subscribers.push(path.to_string());
        }
    }

    pub fn unsubscribe(&mut self, path: &str) {
        self.subscribers.retain(|p| p != path);
    }

    pub fn push(&mut self, i: Input) {
        self.queue.push((i, SCRIPT_DEVICE));
    }

    pub fn push_from(&mut self, i: Input, device: u8) {
        self.queue.push((i, device));
    }

    pub fn clear(&mut self) {
        self.queue.clear();
    }

    pub fn deliver(&mut self, cx: &mut Ctx) -> usize {
        let mut n = 0;
        for (i, device) in std::mem::take(&mut self.queue) {
            let obj = Value::Object(cx.new_object());
            let constant = |cx: &mut Ctx, class: &str, member: &str| -> Value {
                let g = Value::Object(cx.vm.global);
                let ux = cx.get_member(&g, "ux").unwrap_or_default();
                let input = cx.get_member(&ux, "input").unwrap_or_default();
                let c = cx.get_member(&input, class).unwrap_or_default();
                let v = cx.get_member(&c, member).unwrap_or_default();
                if !v.is_nullish() {
                    return v;
                }
                let proto = cx.get_member(&c, "prototype").unwrap_or_default();
                cx.get_member(&proto, member).unwrap_or_default()
            };
            let Input::Button { button, down } = i;
            let ty = constant(cx, "InputManager", "INPUT_TYPE_DIGTIAL");
            let code = constant(cx, "InputCode", button.code_name());
            let state = constant(cx, "InputManager", if down { "KEY_STATE_DOWN" } else { "KEY_STATE_UP" });
            let device = Value::Number(f64::from(device));
            for (k, v) in [("type", ty), ("keyCode", code), ("keyState", state), ("deviceId", device)] {
                let _ = cx.set_member(&obj, k, v);
            }
            for p in self.subscribers.clone() {
                if super::invoke_path(cx, &format!("{p}.ProcessInput"), std::slice::from_ref(&obj)).is_some() {
                    n += 1;
                }
            }
        }
        n
    }
}
