use super::HudEnv;
use crate::avm1::Ctx;
use crate::avm1::value::Value;
use crate::game::clip::{Clip, DI_VISIBLE, DI_X, DI_Y, DisplayInfo};
use crate::game::fe::hint::{
    self, CAMPAIGN_COMPLETE_HINT, HINT_DONE, HINT_ID, HINT_NO_INPUT, INPUT_LOCK_COUNT, Records,
};

pub const PANEL_SOUND: u32 = 0x36;
const DISPLAY_WIDTH: u16 = 0x96;
const DISPLAY_HEIGHT: u16 = 0x95;

#[derive(Default)]
pub struct HintPanel {
    container: Option<Clip>,
    gradient: Option<Clip>,
    panel: Option<Clip>,
    message: Option<Clip>,
    title: Option<Clip>,
    title_text: Option<String>,
    bound: bool,
    intro_played: bool,
    pub outro: bool,
    wide: bool,
    no_input: bool,
    suspend_input: bool,
    lock_taken: bool,
    timer: f32,
    places: Option<Places>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Places {
    wide: [[f32; 2]; 2],
    narrow: [[f32; 2]; 2],
}

impl Places {
    fn of(rec: &hint::Record) -> Places {
        let panel = if !rec.narrow_alt {
            let x = rec.panel[0];
            [if x >= 640.0 { x - 136.0 } else { x + 136.0 }, rec.panel[1]]
        } else {
            rec.panel_alt
        };
        let gradient = if rec.narrow_alt { rec.gradient_alt } else { rec.gradient };
        Places { wide: [rec.panel, rec.gradient], narrow: [panel, gradient] }
    }
}

fn input_lock(env: &mut HudEnv, release: bool) {
    let n = env.values.int(INPUT_LOCK_COUNT) + if release { -1 } else { 1 };
    env.values.set_int(INPUT_LOCK_COUNT, n.max(0));
}

impl HintPanel {
    pub const BINDINGS: [&'static str; 5] = [
        "db_hud_hint_container",
        "db_hud_hint_gradient",
        "db_hud_hint_panel",
        "db_hud_hint_title",
        "db_hud_hint_message",
    ];

    pub fn construct(env: &mut HudEnv) -> Self {
        let (w, h) = (env.values.int(DISPLAY_WIDTH) as f32, env.values.int(DISPLAY_HEIGHT) as f32);
        env.values.set_bool(HINT_DONE, false);
        HintPanel { wide: w / h > 1.55, ..Default::default() }
    }

    pub fn set_wide(&mut self, cx: &mut Ctx, wide: bool) {
        self.wide = wide;
        let Some(p) = self.places else { return };
        let [[x, y], [gx, gy]] = if wide { p.wide } else { p.narrow };
        let at = |x: f32, y: f32| DisplayInfo { x: f64::from(x), y: f64::from(y), ..Default::default() };
        if let Some(c) = self.panel.as_mut() {
            c.set_info(cx, &at(x, y), DI_X | DI_Y);
        }
        if let Some(c) = self.gradient.as_mut() {
            c.set_info(cx, &at(gx, gy), DI_X | DI_Y);
        }
    }

    pub fn destroy(&mut self, env: &mut HudEnv) {
        if self.lock_taken {
            input_lock(env, true);
            self.lock_taken = false;
        }
        env.values.set_bool(HINT_DONE, true);
    }

    pub fn bind(&mut self, name: &str, clip: Clip) {
        let slot = match name {
            "db_hud_hint_container" => &mut self.container,
            "db_hud_hint_gradient" => &mut self.gradient,
            "db_hud_hint_panel" => &mut self.panel,
            "db_hud_hint_title" => {
                self.title_text = None;
                &mut self.title
            }
            "db_hud_hint_message" => &mut self.message,
            _ => return,
        };
        *slot = Some(clip);
        self.bound = true;
    }

    pub fn unbind(&mut self, name: &str) {
        self.bound = false;
        self.intro_played = false;
        self.places = None;
        match name {
            "db_hud_hint_container" => self.container = None,
            "db_hud_hint_gradient" => self.gradient = None,
            "db_hud_hint_panel" => self.panel = None,
            "db_hud_hint_title" => self.title = None,
            "db_hud_hint_message" => self.message = None,
            _ => {}
        }
    }
}

impl HintPanel {
    pub fn update(&mut self, cx: &mut Ctx, env: &mut HudEnv, dt: f64, records: Option<&Records>) {
        if !self.bound {
            return;
        }
        let key = env.values.int(HINT_ID) as u32;
        if let Some(rec) = records.and_then(|r| r.get(key)) {
            match rec.pages.first().map_or(0, |p| p.screen_type) as u16 {
                0 => self.single(cx, env, rec),
                1 | 2 => {
                    self.pages(cx, env, rec, false);
                    self.timer = 0.0;
                }
                3 => {
                    self.pages(cx, env, rec, true);
                    self.timer = 0.0;
                }
                _ => {}
            }
        }
        if self.timer > 0.0 {
            self.timer -= dt as f32;
            if self.timer <= 0.0 {
                if let Some(p) = self.panel.as_mut() {
                    p.goto_label(cx, "outro", false);
                }
                self.outro = true;
            }
        }
        if self.no_input && !env.values.bool(HINT_NO_INPUT) {
            if let Some(p) = self.panel.as_mut() {
                p.goto_label(cx, "outro", false);
            }
            self.no_input = false;
        }
    }

    fn take_lock(&mut self, env: &mut HudEnv, suspend: bool) {
        self.suspend_input = suspend;
        if suspend && !self.lock_taken {
            input_lock(env, false);
            self.lock_taken = true;
        }
    }

    fn single(&mut self, cx: &mut Ctx, env: &mut HudEnv, rec: &hint::Record) {
        env.values.set_int(HINT_ID, 0);
        let title = if rec.title.is_empty() { String::new() } else { env.loc.text(&rec.title) };
        if self.title_text.as_deref() != Some(title.as_str()) {
            if let Some(t) = self.title.as_mut() {
                t.set_text(cx, &title);
            }
            self.title_text = Some(title);
        }
        let body = rec.pages.first().map(|p| p.body.as_str()).unwrap_or("");
        let body = if body.is_empty() { String::new() } else { env.loc.text(body) };
        if let Some(m) = self.message.as_mut() {
            m.invoke(cx, "SetText", &[Value::str(&body)]);
        }
        self.no_input = env.values.bool(HINT_NO_INPUT);
        if let Some(p) = self.panel.as_mut() {
            p.invoke(cx, "SetSuspendInput", &[Value::Bool(rec.suspend_input), Value::Bool(self.no_input)]);
        }
        self.take_lock(env, rec.suspend_input);
        self.timer = rec.seconds;
        let places = Places::of(rec);
        self.places = Some(places);
        let [[x, y], [gx, gy]] = if self.wide { places.wide } else { places.narrow };
        let first = !self.intro_played;
        self.intro_played = true;
        if let Some(c) = self.container.as_mut() {
            c.set_visible(cx, true);
        }
        let shown =
            |x: f32, y: f32| DisplayInfo { x: f64::from(x), y: f64::from(y), visible: true, ..Default::default() };
        if let Some(p) = self.panel.as_mut() {
            p.set_info(cx, &shown(x, y), DI_X | DI_Y | DI_VISIBLE);
            if first {
                p.goto_label(cx, "intro", false);
            }
        }
        if let Some(g) = self.gradient.as_mut() {
            g.set_info(cx, &shown(gx, gy), DI_X | DI_Y | DI_VISIBLE);
        }
    }

    fn pages(&mut self, cx: &mut Ctx, env: &mut HudEnv, rec: &hint::Record, stats: bool) {
        env.values.set_int(HINT_ID, 0);
        let code = hint::language_code(env.loc.language);
        let Some(panel) = self.panel.as_mut() else { return };
        for page in &rec.pages {
            let mut t = |s: &str| if s.is_empty() { Value::str("") } else { Value::str(&env.loc.text(s)) };
            let texts = if stats {
                let [a, b, c] = hint::gear_entry_texts(env.values.item(hint::GEAR_ENTRY));
                let mut d = |k: Option<String>| k.map_or_else(|| Value::str(""), |k| t(&k));
                [d(a), d(b), d(c), Value::str("")]
            } else {
                [t(&page.texts[0]), t(&page.texts[1]), t(&page.texts[2]), t(&page.texts[3])]
            };
            let [t1, t2, t3, t4] = texts;
            let args = [
                t(&page.body),
                t1,
                t2,
                t3,
                t4,
                t(&page.button),
                Value::str(&hint::image_name(&page.images[0], code)),
                Value::str(&hint::image_name(&page.images[1], code)),
                Value::Number(f64::from(page.screen_type)),
            ];
            panel.invoke(cx, "AddPage", &args);
        }
        let head = if rec.key == CAMPAIGN_COMPLETE_HINT {
            "TXT_TUTORIAL_HINT_CAMPAIGN_COMPLETE"
        } else {
            "TXT_TUTORIAL_HINT_TITLE"
        };
        let title = if rec.title.is_empty() { String::new() } else { env.loc.text(&rec.title) };
        let args = [
            Value::str(&env.loc.text(head)),
            Value::str(&title),
            Value::str(&env.loc.text("TXT_ADVANCE")),
            Value::str(&env.loc.text("TXT_BACK")),
        ];
        panel.invoke(cx, "SetPages", &args);
        self.take_lock(env, rec.suspend_input);
        if let Some(c) = self.container.as_mut() {
            c.set_visible(cx, true);
        }
        if let Some(p) = self.panel.as_mut() {
            p.set_visible(cx, true);
            p.goto_label(cx, "layout1", false);
        }
        env.sounds.push(PANEL_SOUND);
    }
}
