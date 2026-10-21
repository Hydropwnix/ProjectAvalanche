use super::{Component, HudEnv};
use crate::avm1::Ctx;
use crate::game::clip::Clip;
use crate::game::values::id;

#[derive(Default)]
pub struct FinishMessage {
    container: Option<Clip>,
    layers: [(Option<Clip>, Option<String>); 3],
    bound: bool,
    shown: bool,
    animating: bool,
    was_animating: bool,
    intro: bool,
    timer: f64,
    frame: i32,
}

const BINDINGS: &[&str] = &[
    "db_hud_finishmessage_container",
    "db_hud_finishmessage_message_layer1",
    "db_hud_finishmessage_message_layer2",
    "db_hud_finishmessage_message_layer3",
];

impl Component for FinishMessage {
    fn bindings(&self) -> &'static [&'static str] {
        BINDINGS
    }

    fn bind(&mut self, _cx: &mut Ctx, _env: &mut HudEnv, name: &str, clip: Clip) {
        match name {
            "db_hud_finishmessage_container" => self.container = Some(clip),
            "db_hud_finishmessage_message_layer1" => self.layers[0] = (Some(clip), None),
            "db_hud_finishmessage_message_layer2" => self.layers[1] = (Some(clip), None),
            "db_hud_finishmessage_message_layer3" => self.layers[2] = (Some(clip), None),
            _ => return,
        }
        self.bound = true;
    }

    fn unbind(&mut self, _name: &str) {
        self.bound = false;
    }

    fn update(&mut self, cx: &mut Ctx, env: &mut HudEnv, dt: f64) {
        if !self.bound {
            return;
        }
        if !self.animating {
            self.was_animating = false;
        } else if let Some(c) = self.container.as_mut() {
            let f = c.current_frame(cx);
            let prev = std::mem::replace(&mut self.frame, f);
            self.was_animating = self.animating;
            self.animating = f != prev;
        }
        self.message(cx, env, dt);
    }
}

impl FinishMessage {
    fn message(&mut self, cx: &mut Ctx, env: &mut HudEnv, dt: f64) {
        let Some(c) = self.container.as_mut() else { return };
        if !env.values.bool(id::FINISH_MESSAGE_ACTIVE) {
            if self.shown {
                c.set_visible(cx, false);
                self.shown = false;
            }
            return;
        }
        if !self.shown {
            let text = env.values.str(id::FINISH_MESSAGE).to_string();
            for (clip, last) in &mut self.layers {
                if let Some(l) = clip.as_mut()
                    && last.as_deref() != Some(text.as_str())
                {
                    l.set_text(cx, &text);
                    *last = Some(text.clone());
                }
            }
            let c = self.container.as_mut().expect("checked");
            if !self.animating {
                c.goto_label(cx, "Intro", false);
                self.animating = true;
                self.was_animating = false;
                self.intro = true;
                self.frame = c.current_frame(cx);
                c.set_visible(cx, true);
                self.shown = true;
                env.sounds.push(0x77);
            }
            return;
        }
        if self.animating {
            return;
        }
        if self.was_animating {
            if self.intro {
                self.timer = 0.0;
                return;
            }
            c.set_visible(cx, false);
            self.shown = false;
            env.nav_events.push("finishmessage_disable".into());
            env.values.set_bool(id::FINISH_MESSAGE_ACTIVE, false);
            return;
        }
        self.timer += dt;
        if self.timer >= 1.0 {
            c.goto_label(cx, "Outro", false);
            self.was_animating = false;
            self.intro = false;
            self.animating = true;
            self.frame = c.current_frame(cx);
        }
    }
}
