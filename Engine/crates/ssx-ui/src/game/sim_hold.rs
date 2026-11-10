use super::GameUi;
use super::fe::fade::Fader;
use super::hud::HudMessage;
use super::values::UiValues;

pub struct SimHold {
    values: UiValues,
    messages: Vec<HudMessage>,
    gear: [bool; 15],
    fader: Option<Fader>,
    lent: Option<Fader>,
    faded: bool,
}

impl SimHold {
    pub fn new(ui: &GameUi) -> SimHold {
        SimHold::of(&ui.values, ui.gear, ui.fe.as_ref().map(|fe| fe.fader))
    }

    fn of(values: &UiValues, gear: [bool; 15], fader: Option<Fader>) -> SimHold {
        SimHold { values: values.held_copy(), messages: Vec::new(), gear, fader, lent: None, faded: false }
    }

    pub fn run<R>(&mut self, ui: &mut GameUi, f: impl FnOnce(&mut GameUi) -> R) -> R {
        let fader = ui.fe.as_mut().map(|fe| &mut fe.fader);
        self.swap(&mut ui.values, &mut ui.hud.messages, &mut ui.gear, fader);
        let r = f(ui);
        let fader = ui.fe.as_mut().map(|fe| &mut fe.fader);
        if fader.is_none() && self.lent.take().is_some() {
            self.fader = None;
        }
        self.swap(&mut ui.values, &mut ui.hud.messages, &mut ui.gear, fader.filter(|_| self.lent.is_some()));
        r
    }

    fn swap(
        &mut self,
        values: &mut UiValues,
        messages: &mut Vec<HudMessage>,
        gear: &mut [bool; 15],
        fader: Option<&mut Fader>,
    ) {
        std::mem::swap(values, &mut self.values);
        std::mem::swap(messages, &mut self.messages);
        std::mem::swap(gear, &mut self.gear);
        if let (Some(live), Some(held)) = (fader, self.fader.as_mut()) {
            std::mem::swap(live, held);
            match self.lent.take() {
                None => self.lent = Some(*live),
                Some(lent) => self.faded |= *held != lent,
            }
        }
    }

    pub fn step_fader(&mut self, dt: f32) {
        if let Some(f) = self.fader.as_mut() {
            f.step(dt);
        }
    }

    pub fn release(self, ui: &mut GameUi) {
        let fader = ui.fe.as_mut().map(|fe| &mut fe.fader);
        self.release_into(&mut ui.values, &mut ui.hud.messages, &mut ui.gear, fader);
    }

    fn release_into(
        self,
        values: &mut UiValues,
        messages: &mut Vec<HudMessage>,
        gear: &mut [bool; 15],
        fader: Option<&mut Fader>,
    ) {
        values.copy_written(&self.values);
        messages.extend(self.messages);
        *gear = self.gear;
        if let (Some(live), Some(held)) = (fader, self.fader.filter(|_| self.faded)) {
            *live = held;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::fe::fade::{FADE_IN, FADE_OUT};
    use crate::game::fe::pause::PAUSE_IS_IN_PAUSE_MENU;
    use crate::game::values::id;

    #[test]
    fn a_hold_hands_over_the_simulations_writes_at_its_end() {
        let mut values = UiValues::new();
        let mut messages = vec![HudMessage::BanksFlushed { captured: true }];
        let mut gear = [false; 15];
        values.set_bool(id::SCORE_ACTIVE, true);
        values.set_float(id::SCORE, 10.0);
        values.clear_dirty();
        let mut hold = SimHold::of(&values, gear, None);
        values.set_bool(id::SCORE_ACTIVE, false);
        values.clear_dirty();
        hold.swap(&mut values, &mut messages, &mut gear, None);
        assert!(values.bool(id::SCORE_ACTIVE) && messages.is_empty(), "the writer reads the table of the hold's start");
        values.set_bool(id::SCORE_ACTIVE, true);
        values.set_float(id::SCORE, 25.0);
        values.set_bool(id::FINISH_MESSAGE_ACTIVE, true);
        values.set_bool(id::FINISH_MESSAGE_ACTIVE, false);
        gear[3] = true;
        messages.push(HudMessage::BanksFlushed { captured: false });
        hold.swap(&mut values, &mut messages, &mut gear, None);
        assert!(!values.bool(id::SCORE_ACTIVE) && values.float(id::SCORE) == 10.0 && !gear[3]);
        assert!(!values.is_dirty(id::SCORE_ACTIVE) && !values.is_dirty(id::SCORE));
        assert_eq!(messages, vec![HudMessage::BanksFlushed { captured: true }]);
        values.set_bool(PAUSE_IS_IN_PAUSE_MENU, true);
        hold.release_into(&mut values, &mut messages, &mut gear, None);
        assert!(values.bool(id::SCORE_ACTIVE) && values.is_dirty(id::SCORE_ACTIVE));
        assert!(values.float(id::SCORE) == 25.0 && values.is_dirty(id::SCORE));
        assert!(!values.bool(id::FINISH_MESSAGE_ACTIVE) && values.is_dirty(id::FINISH_MESSAGE_ACTIVE));
        assert!(values.bool(PAUSE_IS_IN_PAUSE_MENU) && gear[3]);
        assert_eq!(
            messages,
            vec![HudMessage::BanksFlushed { captured: true }, HudMessage::BanksFlushed { captured: false }]
        );
    }

    #[test]
    fn a_held_fade_reaches_the_screen_at_the_holds_end() {
        let (mut values, mut messages, mut gear) = (UiValues::new(), Vec::new(), [false; 15]);
        let dt = 1.0 / 60.0;
        let mut screen = Fader::default();
        let mut hold = SimHold::of(&values, gear, Some(screen));
        hold.swap(&mut values, &mut messages, &mut gear, Some(&mut screen));
        screen.message(FADE_OUT, 1.0, 0.0);
        hold.swap(&mut values, &mut messages, &mut gear, Some(&mut screen));
        for _ in 0..30 {
            screen.step(dt);
            hold.step_fader(dt);
            hold.swap(&mut values, &mut messages, &mut gear, Some(&mut screen));
            hold.swap(&mut values, &mut messages, &mut gear, Some(&mut screen));
        }
        assert_eq!(screen.presentation()[0], 0.0, "nothing over the pause");
        hold.release_into(&mut values, &mut messages, &mut gear, Some(&mut screen));
        assert!((screen.presentation()[0] - 0.5).abs() < 1e-3, "half way out: {:?}", screen.presentation());
        let mut hold = SimHold::of(&values, gear, Some(screen));
        screen.message(FADE_IN, 2.0, 0.0);
        let own = screen;
        hold.swap(&mut values, &mut messages, &mut gear, Some(&mut screen));
        hold.swap(&mut values, &mut messages, &mut gear, Some(&mut screen));
        hold.release_into(&mut values, &mut messages, &mut gear, Some(&mut screen));
        assert_eq!(screen, own);
    }
}
