use super::{Component, HudEnv};
use crate::avm1::Ctx;
use crate::avm1::value::Value;
use crate::game::clip::{Clip, DI_VISIBLE, DI_X, DisplayInfo};
use crate::game::values::{LeaderboardRow, id};

#[derive(Default)]
struct Text {
    clip: Option<Clip>,
    last: Option<String>,
}

impl Text {
    fn set(&mut self, cx: &mut Ctx, s: &str) {
        if self.last.as_deref() != Some(s)
            && let Some(c) = self.clip.as_mut()
        {
            c.set_text(cx, s);
            self.last = Some(s.to_string());
        }
    }
}

struct Row {
    clip: Clip,
    frame: i32,
    content: Clip,
    gamerpic: Clip,
    score_clip: Clip,
    place: Text,
    score: Text,
    glow: Text,
}

impl Row {
    fn new(cx: &mut Ctx, clip: Clip) -> Row {
        let child = |cx: &mut Ctx, path: &str| Clip::bind(cx, path);
        let p = clip.path.clone();
        let content = child(cx, &format!("{p}.mcRowContent"));
        let gamerpic = child(cx, &format!("{p}.mcRowContent.mcGamerpicContainer.mcGamerpic"));
        let score_clip = child(cx, &format!("{p}.mcRowContent.txtPlayerScore"));
        let text = |cx: &mut Ctx, path: String| Text { clip: Some(Clip::bind(cx, &path)), last: None };
        let place = text(cx, format!("{p}.mcRowContent.PlaceNumber.txt"));
        let score = text(cx, format!("{p}.mcRowContent.txtPlayerScore.txt"));
        let glow = text(cx, format!("{p}.mcRowContent.txtPlayerScoreGlow.txt"));
        Row { clip, frame: 0, content, gamerpic, score_clip, place, score, glow }
    }

    fn set_visible(&mut self, cx: &mut Ctx, v: bool) {
        self.clip.set_visible(cx, v);
    }

    fn label(&mut self, cx: &mut Ctx, label: &str) {
        self.content.goto_label(cx, label, true);
        self.content.set_visible(cx, true);
    }

    fn text_width(&mut self, cx: &mut Ctx) -> i32 {
        match self.score.clip.as_mut() {
            Some(c) => c.get_number(cx, "textWidth") as i32,
            None => 0,
        }
    }

    fn right_edge(&mut self, cx: &mut Ctx) -> i32 {
        self.clip.get_number(cx, "_x") as i32
            + self.content.get_number(cx, "_x") as i32
            + self.score_clip.get_number(cx, "_x") as i32
            + self.score_clip.get_number(cx, "_width") as i32
    }
}

#[derive(Default)]
pub struct DynamicLeaderboard {
    container: Option<Clip>,
    large: Option<Clip>,
    large_number: Option<Clip>,
    large_text: Text,
    rows: Vec<(String, Row)>,
    shown_rows: Vec<LeaderboardRow>,
    bound: bool,
    enabled: bool,
    built: bool,
    shown: bool,
    animating: bool,
    moving: bool,
    hidden: bool,
    frame: i32,
}

const BINDINGS: &[&str] = &[
    "db_hud_dynamic_leaderboard",
    "db_hud_dynamic_leaderboard_row1",
    "db_hud_dynamic_leaderboard_row2",
    "db_hud_dynamic_leaderboard_row3",
    "db_hud_dynamic_leaderboard_row4",
    "db_hud_dynamic_leaderboard_row5",
    "db_hud_dynamic_leaderboard_large_number",
];

impl Component for DynamicLeaderboard {
    fn bindings(&self) -> &'static [&'static str] {
        BINDINGS
    }

    fn bind(&mut self, cx: &mut Ctx, _env: &mut HudEnv, name: &str, clip: Clip) {
        match name {
            "db_hud_dynamic_leaderboard" => self.container = Some(clip),
            "db_hud_dynamic_leaderboard_large_number" => {
                let n = Clip::bind(cx, &format!("{}.mcLargeNumber", clip.path));
                let t = Clip::bind(cx, &format!("{}.mcLargeNumber.mcPlaceNumber.txt", clip.path));
                (self.large, self.large_number, self.large_text) =
                    (Some(clip), Some(n), Text { clip: Some(t), last: None });
            }
            n if n.starts_with("db_hud_dynamic_leaderboard_row") => {
                let row = Row::new(cx, clip);
                self.rows.retain(|(k, _)| k != n);
                self.rows.push((n.to_string(), row));
            }
            _ => return,
        }
        self.bound = true;
    }

    fn unbind(&mut self, name: &str) {
        self.bound = false;
        match name {
            "db_hud_dynamic_leaderboard" => self.container = None,
            "db_hud_dynamic_leaderboard_large_number" => {
                (self.large, self.large_number, self.large_text) = (None, None, Text::default());
            }
            n => self.rows.retain(|(k, _)| k != n),
        }
    }

    fn update(&mut self, cx: &mut Ctx, env: &mut HudEnv, _dt: f64) {
        if !self.bound {
            return;
        }
        self.enabled = env.values.bool(id::DYNAMIC_LEADERBOARD_ENABLED);
        if self.animating
            && let Some(c) = self.container.as_mut()
        {
            let f = c.current_frame(cx);
            self.animating = f != std::mem::replace(&mut self.frame, f);
        }
        if self.moving {
            self.moving = false;
            for k in 0..self.rows.len() {
                let r = &mut self.rows[k].1;
                let f = r.clip.current_frame(cx);
                if f != std::mem::replace(&mut r.frame, f) {
                    self.moving = true;
                }
                let label = r.clip.get(cx, "_currentlabel");
                let label = cx.to_string(&label).map(|s| s.to_string()).unwrap_or_default();
                if label == "MovingUpMidpoint" || label == "MovingDownMidpoint" {
                    self.fill_row(cx, env, k);
                }
            }
        }
        let rows = env.values.rows(id::LEADERBOARD).to_vec();
        self.build(cx, env, rows);
        self.show_or_hide(cx, env);
    }
}

impl DynamicLeaderboard {
    fn build(&mut self, cx: &mut Ctx, env: &mut HudEnv, rows: Vec<LeaderboardRow>) {
        if self.animating || self.container.is_none() || !self.enabled {
            return;
        }
        if self.built {
            self.reorder(cx, env, rows);
            return;
        }
        if rows.is_empty() {
            return;
        }
        let n = rows.len().min(self.rows.len());
        self.shown_rows = rows;
        for k in 0..n {
            self.rows[k].1.set_visible(cx, true);
            self.fill_row(cx, env, k);
        }
        for k in n..self.rows.len() {
            self.rows[k].1.set_visible(cx, false);
        }
        let width = if (0..=4).contains(&env.event_type) {
            0x55
        } else {
            (0..n).map(|k| self.rows[k].1.text_width(cx)).max().unwrap_or(0)
        };
        if let Some(r) = self.rows.first_mut() {
            let edge = r.1.right_edge(cx);
            let (Some(big), Some(num)) = (self.large.as_mut(), self.large_number.as_mut()) else { return };
            let num_width = num.get_number(cx, "_width") as i32;
            let big_x = big.get_number(cx, "_x") as i32;
            let x = f64::from(edge + (-50 - width) - big_x - num_width);
            num.set_info(cx, &DisplayInfo { x, visible: true, ..Default::default() }, DI_X | DI_VISIBLE);
        }
        self.built = true;
        for (_, r) in &mut self.rows {
            r.frame = r.clip.current_frame(cx);
        }
    }

    fn reorder(&mut self, cx: &mut Ctx, env: &mut HudEnv, rows: Vec<LeaderboardRow>) {
        if self.moving {
            return;
        }
        let old = std::mem::replace(&mut self.shown_rows, rows);
        for k in 0..self.shown_rows.len().min(self.rows.len()) {
            let same = old.get(k).is_some_and(|o| o.name == self.shown_rows[k].name);
            if same {
                if old[k].value != self.shown_rows[k].value {
                    self.fill_row(cx, env, k);
                }
                continue;
            }
            let name = &self.shown_rows[k].name;
            let up = self.shown_rows.iter().position(|r| &r.name == name).is_some_and(|p| p < k);
            let r = &mut self.rows[k].1;
            r.clip.goto_label(cx, if up { "MovingUp" } else { "MovingDown" }, false);
            r.clip.set_visible(cx, true);
            self.moving = true;
            if k == 1 && up {
                env.sounds.push(0x74);
            }
        }
    }

    fn fill_row(&mut self, cx: &mut Ctx, env: &mut HudEnv, k: usize) {
        let Some(row) = self.shown_rows.get(k).cloned() else { return };
        let Some((_, r)) = self.rows.get_mut(k) else { return };
        r.gamerpic.invoke(cx, "LoadImage", &[Value::str(&row.icon)]);
        let place = env.loc.ordinal(i64::from(row.rank));
        r.place.set(cx, &place);
        let text = if (0..=4).contains(&env.event_type) { &row.score } else { &row.name };
        r.score.set(cx, text);
        r.glow.set(cx, text);
        if row.is_player {
            r.label(cx, "NormalLabel");
            self.large_text.set(cx, &place);
            return;
        }
        let label = match row.medal {
            -1 => "RivalLabel",
            2 => "GoldLabel",
            1 => "SilverLabel",
            0 => "BronzeLabel",
            _ => return,
        };
        r.label(cx, label);
    }

    fn show_or_hide(&mut self, cx: &mut Ctx, env: &mut HudEnv) {
        if !self.built {
            return;
        }
        let Some(c) = self.container.as_mut() else { return };
        if self.enabled {
            if !self.shown && !self.animating {
                c.goto_label(cx, "Intro", false);
                self.animating = true;
                self.frame = c.current_frame(cx);
                c.set_visible(cx, true);
                self.shown = true;
            }
            return;
        }
        if self.shown && !self.animating {
            c.goto_label(cx, "Outro", false);
            self.animating = true;
            self.frame = c.current_frame(cx);
            self.hidden = true;
            self.shown = false;
        }
        if !self.animating && self.hidden {
            env.nav_events.push("dynamicleaderboard_disable".into());
            self.hidden = false;
        }
    }
}
