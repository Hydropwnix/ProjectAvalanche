use super::scoring::Scoring;
use super::{Component, HudEnv};
use crate::avm1::Ctx;
use crate::avm1::value::Value;
use crate::game::clip::{Clip, DI_VISIBLE, DI_X, DI_Y, DisplayInfo};
use crate::game::values::id;

#[derive(Default)]
struct Text {
    clip: Option<Clip>,
    last: Option<String>,
    visible: Option<bool>,
}

impl Text {
    fn set(&mut self, cx: &mut Ctx, text: &str, force: bool) {
        let Some(c) = self.clip.as_mut() else { return };
        if force || self.last.as_deref() != Some(text) {
            c.set_text(cx, text);
            self.last = Some(text.to_string());
        }
    }

    fn show(&mut self, cx: &mut Ctx, v: bool) {
        let Some(c) = self.clip.as_mut() else { return };
        if self.visible != Some(v) {
            c.set_visible(cx, v);
            self.visible = Some(v);
        }
    }

    fn nudge_x(&mut self, cx: &mut Ctx, dx: f64) {
        if let Some(c) = self.clip.as_mut() {
            let mut i = c.info(cx);
            i.x += dx;
            c.set_info(cx, &i, DI_X);
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ComboEnded {
    pub success: bool,
    pub trick_points: f32,
    pub bonus_points: f32,
    pub total_points: f32,
    pub combo_level: u32,
}

#[derive(Default)]
pub struct TrickIndicator {
    grab: Option<Clip>,
    combo: Option<Clip>,
    count: Option<Clip>,
    deposit_value: Option<Clip>,
    wingsuit: Option<Clip>,
    wingsuit_fill: Option<Clip>,
    wingsuit_decay: Option<Clip>,
    grab_name: Text,
    metric: Text,
    duplicate: Text,
    count_txt: Text,
    deposit_title: Text,
    deposit_txt: Text,
    names: [Text; 7],
    fails: [Text; 7],
    capture: bool,
    tier: u32,
    dupe: bool,
    grab_frame: i32,
    combo_frame: i32,
    level: u32,
    grab_ready: bool,
    combo_shown: bool,
    active: bool,
    combo_active: bool,
    pending: bool,
    pulse: bool,
    wingsuit_up: bool,
    wide: bool,
    bound: bool,
    animating: bool,
    last_list: Vec<String>,
    pub deposits: Vec<String>,
}

const BINDINGS: &[&str] = &[
    "db_hud_trick_grabindicator",
    "db_hud_trick_grabname_txt",
    "db_hud_trick_metric_description_txt",
    "db_hud_trick_duplicate_txt",
    "db_hud_trick_combo",
    "db_hud_trick_combo_name0_norm_txt",
    "db_hud_trick_combo_name1_norm_txt",
    "db_hud_trick_combo_name2_norm_txt",
    "db_hud_trick_combo_name3_norm_txt",
    "db_hud_trick_combo_name4_norm_txt",
    "db_hud_trick_combo_name5_norm_txt",
    "db_hud_trick_combo_name6_norm_txt",
    "db_hud_trick_combo_name0_fail_txt",
    "db_hud_trick_combo_name1_fail_txt",
    "db_hud_trick_combo_name2_fail_txt",
    "db_hud_trick_combo_name3_fail_txt",
    "db_hud_trick_combo_name4_fail_txt",
    "db_hud_trick_combo_name5_fail_txt",
    "db_hud_trick_combo_name6_fail_txt",
    "db_hud_trick_combo_count",
    "db_hud_trick_combo_count_txt",
    "db_hud_trick_combo_deposit_title_txt",
    "db_hud_trick_combo_deposit_value",
    "db_hud_trick_combo_deposit_value_txt",
    "db_hud_trick_wingsuit_timer",
    "db_hud_trick_wingsuit_timer_fill",
    "db_hud_trick_wingsuit_timer_decay",
];

impl Component for TrickIndicator {
    fn bindings(&self) -> &'static [&'static str] {
        BINDINGS
    }

    fn bind(&mut self, cx: &mut Ctx, _env: &mut HudEnv, name: &str, clip: Clip) {
        let text = |t: &mut Text, c: Clip| *t = Text { clip: Some(c), last: None, visible: None };
        if let Some(k) = name.strip_prefix("db_hud_trick_combo_name").and_then(|s| s.chars().next()?.to_digit(10)) {
            let k = k as usize;
            if name.ends_with("_norm_txt") {
                text(&mut self.names[k], clip);
            } else {
                text(&mut self.fails[k], clip);
            }
            self.bound = true;
            return;
        }
        match name {
            "db_hud_trick_grabindicator" => {
                clip_hidden(cx, &clip);
                self.grab = Some(clip);
            }
            "db_hud_trick_grabname_txt" => text(&mut self.grab_name, clip),
            "db_hud_trick_metric_description_txt" => text(&mut self.metric, clip),
            "db_hud_trick_duplicate_txt" => text(&mut self.duplicate, clip),
            "db_hud_trick_combo" => {
                clip_hidden(cx, &clip);
                self.combo = Some(clip);
            }
            "db_hud_trick_combo_count" => self.count = Some(clip),
            "db_hud_trick_combo_count_txt" => text(&mut self.count_txt, clip),
            "db_hud_trick_combo_deposit_title_txt" => text(&mut self.deposit_title, clip),
            "db_hud_trick_combo_deposit_value" => self.deposit_value = Some(clip),
            "db_hud_trick_combo_deposit_value_txt" => text(&mut self.deposit_txt, clip),
            "db_hud_trick_wingsuit_timer" => {
                clip_hidden(cx, &clip);
                self.wingsuit_up = false;
                self.wingsuit = Some(clip);
            }
            "db_hud_trick_wingsuit_timer_fill" => self.wingsuit_fill = Some(clip),
            "db_hud_trick_wingsuit_timer_decay" => self.wingsuit_decay = Some(clip),
            _ => {}
        }
        self.bound = true;
    }

    fn unbind(&mut self, _name: &str) {
        // TODO(guess): the unbind 0x82625F98 clears the one binding; the port stops the update instead; settle: track each binding's pointer.
        self.bound = false;
    }

    fn update(&mut self, cx: &mut Ctx, env: &mut HudEnv, _dt: f64) {
        let v = &*env.values;
        let active = v.bool(id::TRICK_INDICATOR_ACTIVE);
        let active_changed = v.is_dirty(id::TRICK_INDICATOR_ACTIVE);
        self.active = active;
        if active {
            self.level = v.int(id::COMBO_LEVEL).max(0) as u32;
            let changed = v.is_dirty(id::COMBO_LEVEL) || v.is_dirty(id::COMBO_TRICK_LIST);
            self.combo_active = v.bool(id::COMBO_INDICATOR_ACTIVE);
            if self.bound {
                self.grab_update(cx, env);
                self.combo_update(cx, env, changed);
                self.wingsuit_update(cx, env);
            }
        }
        self.visibility(cx, active_changed);
    }
}

fn clip_hidden(cx: &mut Ctx, c: &Clip) {
    c.clone().set_visible(cx, false);
}

impl TrickIndicator {
    pub fn banks_flushed(&mut self, cx: &mut Ctx, env: &mut HudEnv, captured: bool) {
        if self.grab.is_none() || self.capture {
            return;
        }
        if !captured {
            self.tier = 1;
            self.show_tier(cx, env, 1);
        } else {
            if env.values.bool(id::TRICK_IS_DUPLICATE) {
                self.tier = 1;
            }
            let label = match self.tier {
                0 => "off",
                1 => "tier1_bonus",
                2 => "tier2_bonus",
                _ => "tier3_bonus",
            };
            if let Some(g) = self.grab.as_mut() {
                g.goto_label(cx, label, false);
            }
            env.sounds.push(0x4B);
            self.capture = true;
        }
    }

    pub fn combo_ended(&mut self, cx: &mut Ctx, env: &mut HudEnv, e: ComboEnded, scoring: &mut Scoring) {
        self.pulse = true;
        if self.combo.is_some() && self.combo_active {
            let n = e.combo_level;
            let prefix = if e.success { "cash" } else { "fail" };
            let (label, sound) = match n {
                0 => (None, None),
                1..=5 => (Some(format!("{prefix}{}", n - 1)), Some(n - 1)),
                _ => (Some(format!("{prefix}5")), Some(5)),
            };
            let combo = self.combo.as_mut().expect("checked");
            match label {
                Some(l) => combo.goto_label(cx, &l, false),
                None if e.success || !self.animating => combo.goto_label(cx, "off", true),
                None => {}
            }
            if let Some(s) = sound {
                env.sounds.push(if e.success { 0x5B } else { 0x61 } + s);
            }
            if e.success || !self.animating {
                if let Some(c) = self.count.as_mut() {
                    c.goto_label(cx, prefix, false);
                }
                let count = env.loc.int(i64::from(n));
                self.count_txt.set(cx, &count, true);
                if e.success {
                    let level = env.loc.int(i64::from(self.level));
                    let title = env.loc.format("TXT_HUD_TRICK_COMBO_DEPOSIT", &[&level]);
                    self.deposit_title.set(cx, &title, false);
                    self.deposits.push(title);
                    let pts = env.loc.number(f64::from(e.trick_points), 0);
                    self.deposit_txt.set(cx, &pts, true);
                    self.shift_deposit(cx, self.level);
                } else {
                    if let Some(d) = self.deposit_value.as_mut() {
                        d.goto_label(cx, "fail", false);
                    }
                    let pts = env.loc.number(f64::from(e.total_points), 0);
                    self.deposit_txt.set(cx, &pts, true);
                    let list = self.last_list.clone();
                    self.fill_names(cx, &list, false);
                    self.shift_deposit(cx, n);
                }
            }
        }
        if e.success {
            scoring.deposit(cx, env, e.trick_points, e.bonus_points, e.combo_level);
        }
    }

    fn grab_update(&mut self, cx: &mut Ctx, env: &mut HudEnv) {
        let Some(g) = self.grab.as_mut() else { return };
        if !self.grab_ready {
            self.dupe = false;
            self.tier = 1;
            self.grab_frame = g.current_frame(cx);
            self.show_tier(cx, env, 1);
            self.grab_name.set(cx, " ", false);
            self.grab_ready = true;
            return;
        }
        let f = g.current_frame(cx);
        let prev = std::mem::replace(&mut self.grab_frame, f);
        if prev == f {
            if !self.capture {
                let tier = match env.values.int(id::TRICK_TIER) {
                    0 => 1,
                    t => t as u32,
                };
                if !env.values.bool(id::TRICK_IS_DUPLICATE) {
                    if self.dupe || tier != self.tier {
                        self.tier = tier;
                        self.show_tier(cx, env, tier);
                        self.dupe = false;
                    }
                } else if !self.dupe {
                    self.show_duplicate(cx, env);
                    self.dupe = true;
                }
            } else {
                self.capture = false;
                self.tier = 1;
                self.dupe = env.values.bool(id::TRICK_IS_DUPLICATE);
                if self.dupe {
                    self.show_duplicate(cx, env);
                } else {
                    self.show_tier(cx, env, 1);
                }
            }
        }
        if !self.capture {
            let name = env.values.str(id::TRICK_GRAB_NAME).to_string();
            self.grab_name.set(cx, &name, false);
        }
        let playing = env.values.bool(id::IS_PLAYING_SCORE_ANIM);
        if let Some(g) = self.grab.as_mut() {
            g.set_visible(cx, !playing);
        }
    }

    fn show_tier(&mut self, cx: &mut Ctx, env: &mut HudEnv, tier: u32) {
        if tier < 4 {
            let label = match tier {
                0 => "off",
                1 => "tier1_metric",
                2 => "tier2_metric",
                _ => "tier3_metric",
            };
            if let Some(g) = self.grab.as_mut() {
                g.goto_label(cx, label, false);
            }
            let text = match tier {
                2 => env.loc.text("TXT_HUD_TRICK_AIR_TIER_2"),
                3 => env.loc.text("TXT_HUD_TRICK_AIR_TIER_3"),
                _ => " ".to_string(),
            };
            self.metric.set(cx, &text, false);
        }
    }

    fn show_duplicate(&mut self, cx: &mut Ctx, env: &mut HudEnv) {
        let t = env.loc.text("TXT_DUPLICATE_TRICK");
        self.duplicate.set(cx, &t, false);
        if let Some(g) = self.grab.as_mut() {
            g.goto_label(cx, "dupe", false);
        }
    }

    fn combo_update(&mut self, cx: &mut Ctx, env: &mut HudEnv, changed: bool) {
        let Some(c) = self.combo.as_mut() else { return };
        if !self.combo_active && self.combo_shown {
            c.goto_label(cx, "off", true);
            self.combo_shown = false;
            return;
        }
        if !self.combo_shown {
            c.goto_label(cx, "off", true);
            self.combo_frame = c.current_frame(cx);
            self.combo_shown = true;
            self.pulse = true;
            return;
        }
        if !(changed || self.pending) {
            return;
        }
        let f = c.current_frame(cx);
        let prev = std::mem::replace(&mut self.combo_frame, f);
        self.animating = f != prev;
        if self.animating {
            self.pending = true;
        } else {
            let list = env.values.list(id::COMBO_TRICK_LIST).to_vec();
            let total = env.values.float(id::COMBO_TOTAL_POINTS);
            self.show_list(cx, env, &list, total);
            self.last_list = list;
            self.pending = false;
        }
        if let Some(c) = self.combo.as_mut() {
            c.set_visible(cx, true);
        }
    }

    fn show_list(&mut self, cx: &mut Ctx, env: &mut HudEnv, list: &[String], total: f32) {
        if self.combo.is_none() || !self.combo_active {
            return;
        }
        if self.pulse {
            if let Some(c) = self.count.as_mut() {
                c.goto_label(cx, "pulse", false);
            }
            if let Some(d) = self.deposit_value.as_mut() {
                d.goto_label(cx, "normal", false);
            }
            self.pulse = false;
        }
        let n = list.len().min(7);
        if self.level == 0 || n < 1 {
            if !self.animating
                && let Some(c) = self.combo.as_mut()
            {
                c.goto_label(cx, "off", true);
            }
            return;
        }
        let (label, sound) =
            if n <= 6 { (format!("add{}", n - 1), 0x54 + n as u32 - 1) } else { ("cycle".into(), 0x5A) };
        if let Some(c) = self.combo.as_mut() {
            c.goto_label(cx, &label, false);
        }
        env.sounds.push(sound);
        self.fill_names(cx, list, true);
        let count = env.loc.int(i64::from(self.level));
        self.count_txt.set(cx, &count, true);
        let title = env.loc.text(if self.level == 1 {
            "TXT_HUD_TRICK_COMBO_TITLE_SINGLE"
        } else {
            "TXT_HUD_TRICK_COMBO_TITLE"
        });
        self.deposit_title.set(cx, &title, false);
        let pts = env.loc.number(f64::from(total), 0);
        self.deposit_txt.set(cx, &pts, true);
        self.shift_deposit(cx, self.level);
    }

    fn fill_names(&mut self, cx: &mut Ctx, list: &[String], normal: bool) {
        let n = list.len().min(7);
        for i in 0..n {
            let name = &list[list.len() - 1 - i];
            let (on, off) = if normal {
                (&mut self.names[i], &mut self.fails[i])
            } else {
                (&mut self.fails[i], &mut self.names[i])
            };
            on.set(cx, name, false);
            on.show(cx, true);
            off.show(cx, false);
        }
    }

    fn shift_deposit(&mut self, cx: &mut Ctx, count: u32) {
        if count < 100 && self.wide {
            self.deposit_title.nudge_x(cx, -10.0);
            self.deposit_txt.nudge_x(cx, -10.0);
            self.wide = false;
        } else if count >= 100 && !self.wide {
            self.deposit_title.nudge_x(cx, 10.0);
            self.deposit_txt.nudge_x(cx, 10.0);
            self.wide = true;
        }
    }

    fn wingsuit_update(&mut self, cx: &mut Ctx, env: &mut HudEnv) {
        let Some(w) = self.wingsuit.as_mut() else { return };
        if !env.values.bool(id::WINGSUIT_ACTIVE) {
            if self.wingsuit_up {
                self.wingsuit_up = false;
                w.set_visible(cx, false);
            }
            return;
        }
        if !self.wingsuit_up {
            w.goto_label(cx, "intro", false);
            self.wingsuit_up = true;
        }
        let usage = env.values.float(id::WINGSUIT_USAGE);
        if let Some(f) = self.wingsuit_fill.as_mut() {
            f.set(cx, "_height", Value::Number(f64::from((1.0 - usage) * 98.0 + 2.0)));
        }
        let eff = env.values.float(id::WINGSUIT_EFFECTIVENESS);
        if let Some(d) = self.wingsuit_decay.as_mut() {
            d.set(cx, "_height", Value::Number(f64::from((1.0 - eff) * 98.0 + 2.0)));
        }
        let (x, y) = (env.values.float(id::WINGSUIT_TIMER_POS_X), env.values.float(id::WINGSUIT_TIMER_POS_Y));
        let info =
            DisplayInfo { x: f64::from(x - 640.0), y: f64::from(y - 541.0), visible: true, ..Default::default() };
        if let Some(w) = self.wingsuit.as_mut() {
            w.set_info(cx, &info, DI_X | DI_Y | DI_VISIBLE);
        }
    }

    fn visibility(&mut self, cx: &mut Ctx, changed: bool) {
        if changed {
            for c in [self.grab.as_mut(), self.combo.as_mut()].into_iter().flatten() {
                c.set_visible(cx, self.active);
            }
        }
        if !self.active && self.wingsuit_up {
            self.wingsuit_up = false;
            if let Some(w) = self.wingsuit.as_mut() {
                w.set_visible(cx, false);
            }
        }
    }
}
