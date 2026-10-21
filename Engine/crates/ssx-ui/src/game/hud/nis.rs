use super::text::HudText;
use super::{Component, HudEnv};
use crate::avm1::Ctx;
use crate::avm1::value::Value;
use crate::game::clip::Clip;
use crate::game::fe::pause::reason;
use crate::game::fe::{FeEnv, FrontEnd, screens};
use crate::game::values::id;

const LIVE_EVENT_FINISHED: u16 = 0x169;

pub fn change_gear(fe: &mut FrontEnd, env: &mut FeEnv, active: bool) {
    if !active {
        fe.note("A_GameChangeGear: the prompt is not active (+0xA8): nothing");
        return;
    }
    fe.game_pause.set(reason::LEAVING, true);
    env.values.set_bool(id::NIS_PROMPT_POPUP_IS_ACTIVE, true);
    fe.note("A_GameChangeGear: FE manager +0x80(1) (0x34A67757 {1}: pause reason 0x10), 0x115 on");
    env.nav_events.push("go_to_event_equipment_quit".into());
}

pub fn equipment_quit_yes(fe: &mut FrontEnd, env: &mut FeEnv) {
    env.values.set_bool(id::NIS_PROMPT_POPUP_IS_ACTIVE, false);
    fe.game_pause.set(reason::LEAVING, false);
    fe.note("do_event_equipment_quit_yes: 0x115 off, FE manager +0x80(0) (0x34A67757 {0}: pause reason 0x10 off)");
    if fe.game_mode == 3 && env.values.bool(LIVE_EVENT_FINISHED) {
        return;
    }
    fe.change_gear = true;
    fe.note("FE manager +0x7C(1) (+0x2E: change gear), save (FE manager +0xC4), GoToScreen(0x40)");
    fe.requests.push(crate::game::fe::pause::GameRequest::Save);
    fe.go_to_screen(env, screens::id::QUIT_STAGE);
}

pub fn equipment_quit_no(fe: &mut FrontEnd, env: &mut FeEnv) {
    env.values.set_bool(id::NIS_PROMPT_POPUP_IS_ACTIVE, false);
    fe.game_pause.set(reason::LEAVING, false);
    fe.note("do_event_equipment_quit_no: 0x115 off, FE manager +0x80(0) (0x34A67757 {0}: pause reason 0x10 off)");
}

#[derive(Default)]
struct Panel {
    clip: Option<Clip>,
    shown: bool,
    animating: bool,
    hidden: bool,
    frame: i32,
}

impl Panel {
    fn watch(&mut self, cx: &mut Ctx) {
        if self.animating
            && let Some(c) = self.clip.as_mut()
        {
            let f = c.current_frame(cx);
            self.animating = f != std::mem::replace(&mut self.frame, f);
        }
    }

    fn play(&mut self, cx: &mut Ctx, label: &str, visible: Option<bool>) {
        let Some(c) = self.clip.as_mut() else { return };
        c.goto_label(cx, label, false);
        self.animating = true;
        self.frame = c.current_frame(cx);
        if let Some(v) = visible {
            c.set_visible(cx, v);
        }
    }
}

pub struct NisPrompt {
    panel: Panel,
    callout: Option<Clip>,
    message: Option<Clip>,
    message_text: Option<String>,
    active: bool,
    bound: bool,
    pending: bool,
}

impl NisPrompt {
    pub fn destroyed(values: &mut crate::game::values::UiValues) {
        values.set_bool(id::NIS_PROMPT_ACTIVE, false);
    }

    pub fn is_active(&self) -> bool {
        self.active
    }
}

impl Default for NisPrompt {
    fn default() -> Self {
        NisPrompt {
            panel: Panel::default(),
            callout: None,
            message: None,
            message_text: None,
            active: false,
            bound: false,
            pending: true,
        }
    }
}

const PROMPT_BINDINGS: &[&str] =
    &["db_hud_nisprompt_container", "db_hud_nisprompt_callout", "db_hud_nisprompt_message"];

impl Component for NisPrompt {
    fn bindings(&self) -> &'static [&'static str] {
        PROMPT_BINDINGS
    }

    fn bind(&mut self, _cx: &mut Ctx, _env: &mut HudEnv, name: &str, clip: Clip) {
        match name {
            "db_hud_nisprompt_container" => self.panel.clip = Some(clip),
            "db_hud_nisprompt_callout" => self.callout = Some(clip),
            "db_hud_nisprompt_message" => {
                self.message = Some(clip);
                self.message_text = None;
            }
            _ => return,
        }
        self.bound = true;
    }

    fn unbind(&mut self, _name: &str) {
        self.bound = false;
    }

    fn update(&mut self, cx: &mut Ctx, env: &mut HudEnv, _dt: f64) {
        if !self.bound {
            return;
        }
        self.panel.watch(cx);
        let v = &*env.values;
        let finished_live = false;
        self.active = (v.bool(id::NIS_PROMPT_ACTIVE) && !finished_live) || v.bool(id::NIS_PROMPT_POPUP_IS_ACTIVE);
        let paused = v.bool(crate::game::fe::pause::PAUSE_IS_IN_PAUSE_MENU);
        let pause_changed = v.is_dirty(crate::game::fe::pause::PAUSE_IS_IN_PAUSE_MENU);
        if !self.active {
            if self.panel.shown && !self.panel.animating {
                self.panel.play(cx, "Outro", Some(false));
                (self.panel.shown, self.panel.hidden) = (false, true);
            }
            if !self.panel.animating {
                self.pending = true;
                if self.panel.hidden {
                    env.nav_events.push("nisprompt_disable".into());
                }
            }
        } else if !paused {
            if !self.panel.shown && !self.panel.animating {
                self.panel.play(cx, "Intro", Some(true));
                self.panel.shown = true;
            }
            if self.pending {
                let text = env.values.str(id::NIS_PROMPT_MESSAGE).to_string();
                if self.message_text.as_deref() != Some(text.as_str())
                    && let Some(m) = self.message.as_mut()
                {
                    m.set_text(cx, &text);
                    self.message_text = Some(text);
                }
                if let Some(c) = self.callout.as_mut() {
                    let icon = env.values.int(id::NIS_PROMPT_CALLOUT);
                    c.invoke(cx, "SetButtonIcon", &[Value::Number(f64::from(icon))]);
                }
                self.pending = false;
            }
        } else if pause_changed {
            if let Some(c) = self.panel.clip.as_mut() {
                c.set_visible(cx, false);
            }
            self.panel.shown = false;
        }
    }
}

#[derive(Default)]
pub struct NisTitle {
    panel: Panel,
    message: Option<HudText>,
    active: bool,
    timer: f64,
    bound: bool,
    pending: bool,
}

const TITLE_BINDINGS: &[&str] = &["db_hud_nistitle_container", "db_hud_nistitle_message"];

const TITLE_SECONDS: f64 = 4.0;

impl Component for NisTitle {
    fn bindings(&self) -> &'static [&'static str] {
        TITLE_BINDINGS
    }

    fn bind(&mut self, cx: &mut Ctx, _env: &mut HudEnv, name: &str, clip: Clip) {
        match name {
            "db_hud_nistitle_container" => self.panel.clip = Some(clip),
            "db_hud_nistitle_message" => {
                let mut t = HudText::new(cx, clip);
                t.set_rate(25.0);
                self.message = Some(t);
            }
            _ => return,
        }
        self.bound = true;
    }

    fn unbind(&mut self, name: &str) {
        self.bound = false;
        self.panel.shown = false;
        self.panel.animating = false;
        if name == "db_hud_nistitle_message" {
            self.message = None;
        }
    }

    fn update(&mut self, cx: &mut Ctx, env: &mut HudEnv, dt: f64) {
        if !self.bound {
            return;
        }
        self.panel.watch(cx);
        self.active = env.values.bool(id::NIS_TITLE_ACTIVE);
        if self.active {
            if !self.panel.shown && !self.panel.animating {
                if let Some(m) = self.message.as_mut() {
                    m.reveal(cx, 250.0, true);
                }
                self.panel.play(cx, "Intro", Some(true));
                self.panel.shown = true;
                self.timer = TITLE_SECONDS;
            }
            self.timer -= dt;
            if self.timer - dt <= 0.0 {
                env.values.set_bool(id::NIS_TITLE_ACTIVE, false);
            }
            if self.pending {
                let text = env.values.str(id::NIS_TITLE_MESSAGE).to_string();
                if let Some(m) = self.message.as_mut() {
                    m.set_text(cx, &text, false);
                }
                self.pending = false;
            }
            if let Some(m) = self.message.as_mut() {
                m.tick(cx, dt);
            }
            return;
        }
        if self.panel.shown && !self.panel.animating {
            self.panel.play(cx, "Outro", None);
            (self.panel.shown, self.panel.hidden) = (false, true);
        }
        if !self.panel.animating {
            self.pending = true;
            if self.panel.hidden {
                env.nav_events.push("nistitle_disable".into());
                self.panel.hidden = false;
            }
        }
    }
}
