use std::sync::Arc;

use glam::{Quat, Vec3};

use crate::event::EventManager;
use crate::nis::IntroAssets;
use crate::outro::ScreenFade;
use crate::rider::{RiderState, World};

pub const LOOP_FADE_IN_DELAY_S: f32 = 0.5;
pub const LOOP_FADE_IN_S: f32 = 1.5;

#[derive(Clone, Debug)]
pub struct SurviveLoop {
    pub intro: Option<Arc<IntroAssets>>,
    pub outro: Option<Arc<IntroAssets>>,
    pub at: Vec3,
    pub q: Quat,
    pub screen: ScreenFade,
    pub fade_in_due: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SurviveStep {
    #[default]
    None,
    Outro,
    Loop,
}

impl SurviveLoop {
    pub fn new(intro: Option<Arc<IntroAssets>>, outro: Option<Arc<IntroAssets>>, at: Vec3, q: Quat) -> Self {
        Self { intro, outro, at, q, screen: ScreenFade::default(), fade_in_due: false }
    }

    pub fn loop_message(&mut self) {
        self.screen.start(true, 0.0, 0.0);
        self.fade_in_due = true;
    }

    pub fn update(
        &mut self,
        s: &mut RiderState,
        w: &World,
        ev: &mut EventManager,
        effects: &[(String, crate::objectives::Effect)],
        dt_s: f32,
    ) -> SurviveStep {
        // TODO(guess): the loop's fade-in starts at the head of the frame after the loop, before the FE meta update's fader step (0x82399330, 0x82399544); settle: where `FMWorldManager`'s update 0x824D1998 (static 0x838C82D0 made by 0x8369FBF8, manager list 0x838F1E3C walked in dependency order 0x82896230 / 0x82896330) runs in the frame.
        if std::mem::take(&mut self.fade_in_due) {
            crate::guess_hit!();
            self.screen.start(false, LOOP_FADE_IN_S, LOOP_FADE_IN_DELAY_S);
        }
        self.screen.update(effects, dt_s);
        if ev.request_loop_to_top {
            ev.request_loop_to_top = false;
            if let Some(n) = s.loop_to_top(w, self.at, self.q, self.intro.clone()) {
                *s = n;
                return SurviveStep::Loop;
            }
            return SurviveStep::None;
        }
        let play = effects.iter().any(|(_, e)| {
            matches!(e, crate::objectives::Effect::PlayNis(r) if self.outro.as_ref().is_some_and(|o| o.nis.binding == r.name))
        });
        if let Some(outro) = self.outro.clone()
            && play
            && let Ok(mut n) = crate::outro::start(w, outro)
        {
            n.carry_game_state(s);
            *s = n;
            return SurviveStep::Outro;
        }
        SurviveStep::None
    }

    pub fn fade(&self) -> f32 {
        self.screen.level
    }
}
