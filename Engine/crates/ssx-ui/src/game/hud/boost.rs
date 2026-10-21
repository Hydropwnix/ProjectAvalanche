use super::{Component, HudEnv};
use crate::avm1::Ctx;
use crate::avm1::value::Value;
use crate::game::clip::Clip;
use crate::game::values::id;

#[derive(Default)]
pub struct BoostNotification {
    container: Option<Clip>,
    boosts: Option<Clip>,
    header: Option<Clip>,
    icons: [Option<Clip>; 4],
    loaders: Vec<Clip>,
    header_text: Option<String>,
    bound: bool,
    enabled: bool,
    shown: bool,
    strip_animating: bool,
    container_animating: bool,
    hidden: bool,
    built: bool,
    count: usize,
    container_frame: i32,
    strip_frame: i32,
}

const BINDINGS: &[&str] = &[
    "db_hud_boostnotification",
    "db_hud_boostnotification_boosts",
    "db_hud_boostnotification_header",
    "db_hud_boostnotification_icon0",
    "db_hud_boostnotification_icon1",
    "db_hud_boostnotification_icon2",
    "db_hud_boostnotification_icon3",
    "db_hud_boostnotification_icon0_loader",
    "db_hud_boostnotification_icon1_loader",
    "db_hud_boostnotification_icon2_loader",
    "db_hud_boostnotification_icon3_loader",
];

impl BoostNotification {
    fn set(&mut self, name: &str, clip: Option<Clip>) {
        match name {
            "db_hud_boostnotification" => self.container = clip,
            "db_hud_boostnotification_boosts" => self.boosts = clip,
            "db_hud_boostnotification_header" => {
                self.header = clip;
                self.header_text = None;
            }
            n => {
                if let Some(k) = n.strip_prefix("db_hud_boostnotification_icon") {
                    let (k, loader) = (k.as_bytes()[0].wrapping_sub(b'0') as usize, k.ends_with("_loader"));
                    if loader {
                        if let Some(c) = clip {
                            self.loaders.push(c);
                        }
                    } else if let Some(i) = self.icons.get_mut(k) {
                        *i = clip;
                    }
                }
            }
        }
    }
}

impl Component for BoostNotification {
    fn bindings(&self) -> &'static [&'static str] {
        BINDINGS
    }

    fn bind(&mut self, _cx: &mut Ctx, _env: &mut HudEnv, name: &str, clip: Clip) {
        self.set(name, Some(clip));
        self.bound = true;
    }

    fn unbind(&mut self, name: &str) {
        self.bound = false;
        self.set(name, None);
    }

    fn update(&mut self, cx: &mut Ctx, env: &mut HudEnv, _dt: f64) {
        if !self.bound {
            return;
        }
        self.enabled = env.values.bool(id::BOOST_NOTIFICATION_ENABLED);
        if self.container_animating
            && let Some(c) = self.container.as_mut()
        {
            let f = c.current_frame(cx);
            self.container_animating = f != std::mem::replace(&mut self.container_frame, f);
        }
        if self.strip_animating
            && let Some(c) = self.boosts.as_mut()
        {
            let f = c.current_frame(cx);
            self.strip_animating = f != std::mem::replace(&mut self.strip_frame, f);
        }
        let changed = env.values.is_dirty(id::BOOST_NOTIFICATION) || !self.built;
        self.build(cx, env, changed);
        self.show_or_hide(cx, env);
    }
}

impl BoostNotification {
    fn build(&mut self, cx: &mut Ctx, env: &mut HudEnv, changed: bool) {
        if self.container.is_none() || !changed {
            return;
        }
        self.count = 0;
        let boosts = env.values.boosts(id::BOOST_NOTIFICATION).to_vec();
        for b in boosts.iter().take(self.loaders.len()) {
            if b.id != -1 {
                let l = &mut self.loaders[self.count];
                l.invoke(cx, "LoadImage", &[Value::str(&b.image)]);
                self.count += 1;
            }
        }
        let text = env.loc.text("TXT_BOOSTNOTIFICATION_MESSAGE");
        if self.header_text.as_deref() != Some(text.as_str()) {
            if let Some(h) = self.header.as_mut() {
                h.set_text(cx, &text);
            }
            self.header_text = Some(text);
        }
        self.built = true;
    }

    fn show_or_hide(&mut self, cx: &mut Ctx, env: &mut HudEnv) {
        if !self.built {
            return;
        }
        let idle = !self.strip_animating && !self.container_animating;
        if self.enabled {
            if !self.shown && idle {
                let label = match self.count {
                    1 => Some("ShowOneIcon"),
                    2 => Some("ShowTwoIcons"),
                    3 => Some("ShowThreeIcons"),
                    4 => Some("ShowFourIcons"),
                    _ => None,
                };
                let visible = label.is_some();
                if let (Some(l), Some(s)) = (label, self.boosts.as_mut()) {
                    s.goto_label(cx, l, false);
                }
                self.strip_animating = visible;
                if let Some(s) = self.boosts.as_mut() {
                    self.strip_frame = s.current_frame(cx);
                }
                if let Some(c) = self.container.as_mut() {
                    c.set_visible(cx, visible);
                }
                self.shown = visible;
            }
            return;
        }
        if self.shown && idle {
            if let Some(c) = self.container.as_mut() {
                c.goto_label(cx, "HideBoosts", false);
            }
            self.container_animating = true;
            self.shown = false;
            self.hidden = true;
        }
        if !self.container_animating && !self.strip_animating && self.hidden {
            env.nav_events.push("boostnotification_disable".into());
            self.hidden = false;
        }
    }
}
