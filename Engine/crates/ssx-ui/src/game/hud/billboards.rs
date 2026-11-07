use super::{Component, HudEnv};
use crate::avm1::Ctx;
use crate::avm1::value::Value;
use crate::game::clip::{Clip, DI_ALPHA, DI_VISIBLE, DI_X, DI_XSCALE, DI_Y, DI_YSCALE, DisplayInfo};
use crate::game::values::{Marker, id};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub view_proj: [[f32; 4]; 4],
    pub screen: [f32; 2],
}

impl Camera {
    fn clip(&self, p: [f32; 3]) -> [f32; 4] {
        let m = &self.view_proj;
        let mut c = [0.0; 4];
        for (k, c) in c.iter_mut().enumerate() {
            *c = p[0] * m[0][k] + p[1] * m[1][k] + p[2] * m[2][k] + m[3][k];
        }
        c
    }

    fn screen(&self, c: [f32; 4]) -> [f32; 2] {
        let inv = 1.0 / c[3];
        [(c[0] * inv + 1.0) * self.screen[0] * 0.5, (-(c[1] * inv) + 1.0) * self.screen[1] * 0.5]
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Ghost {
    pub id: u64,
    pub name: String,
    pub colour: u32,
    pub position: [f32; 3],
}

pub fn ghost_list(camera: &Camera, ghosts: &[Ghost]) -> Vec<Marker> {
    let mut out = Vec::new();
    for g in ghosts {
        let c = camera.clip([g.position[0], g.position[1] + 0.8, g.position[2]]);
        let z = c[2];
        if z <= 0.0 || z >= 200.0 || c[3] == 0.0 {
            continue;
        }
        let [x, y] = camera.screen(c);
        if x <= 0.0 || x >= camera.screen[0] || y <= 0.0 || y >= camera.screen[1] {
            continue;
        }
        let k = (1.0 - z * (1.0 / 120.0)).clamp(0.0, 1.0);
        let a = (g.colour >> 24) as f32 * (1.0 / 255.0);
        out.push(Marker {
            id: g.id,
            name: g.name.clone(),
            colour: g.colour,
            x,
            y,
            scale: k * 20.0 + 40.0,
            alpha: k * 100.0 * a,
            distance: 0,
        });
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
pub struct TombstonePoint {
    pub id: u64,
    pub name: String,
    pub distance_m: u32,
    pub position: [f32; 3],
}

pub fn tombstone_points(
    t: Option<&ssx_physics::session::tombstones::Tombstones>,
    loc: &mut crate::game::loc::Localization,
) -> Vec<TombstonePoint> {
    let Some(t) = t.filter(|t| t.hud_running()) else { return Vec::new() };
    t.hud_entities()
        .iter()
        .map(|e| TombstonePoint {
            id: e.id,
            name: if e.name.is_empty() || !e.name_is_text_id { e.name.clone() } else { loc.text(&e.name) },
            distance_m: e.distance_m,
            position: e.position,
        })
        .collect()
}

pub fn tombstone_list(camera: &Camera, tombstones: &[TombstonePoint]) -> Vec<Marker> {
    let inv_120 = f32::from_bits(0x3C08_8889);
    let twenty = f32::from_bits(0x41A0_0001);
    let mut out = Vec::new();
    for t in tombstones {
        let c = camera.clip([t.position[0], t.position[1] + 0.8, t.position[2]]);
        let z = c[2];
        if z <= 0.0 || z >= 200.0 || c[3] == 0.0 {
            continue;
        }
        let [x, y] = camera.screen(c);
        if x <= 0.0 || x >= camera.screen[0] || y <= 0.0 || y >= camera.screen[1] {
            continue;
        }
        let k = (1.0 - z * inv_120).clamp(0.0, 1.0);
        out.push(Marker {
            id: t.id,
            name: t.name.clone(),
            colour: 0,
            x,
            y,
            scale: k * twenty + 40.0,
            alpha: k * 100.0,
            distance: t.distance_m as i32,
        });
    }
    out
}

pub fn wingsuit_timer_pos(camera: &Camera, rider: [f32; 3]) -> [f32; 2] {
    let [x, y] = camera.screen(camera.clip(rider));
    [x + 75.0, y]
}

pub fn html_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Ghost,
    Tombstone,
}

struct Billboard {
    id: u64,
    board: Clip,
    names: Vec<(Clip, Option<String>)>,
    extra: Vec<(Clip, Option<String>)>,
}

impl Billboard {
    fn new(cx: &mut Ctx, kind: Kind, clip: &Clip) -> Billboard {
        let p = format!("{}.mcBillboard", clip.path);
        let mut board = Clip::bind(cx, &p);
        let text =
            |cx: &mut Ctx, layer: &str, name: &str| (Clip::bind(cx, &format!("{p}.mcRootClip.{layer}.{name}")), None);
        let (names, extra) = match kind {
            Kind::Ghost => (
                vec![
                    text(cx, "mcAdditive", "txtTopName"),
                    text(cx, "mcAdditive", "txtBottomName"),
                    text(cx, "mcNormal", "txtTopName"),
                    text(cx, "mcNormal", "txtBottomName"),
                ],
                Vec::new(),
            ),
            Kind::Tombstone => (
                vec![
                    text(cx, "mcAdditive", "txtNameNormal"),
                    text(cx, "mcAdditive", "txtNameBlur"),
                    text(cx, "mcNormal", "txtNameNormal"),
                    text(cx, "mcNormal", "txtNameBlur"),
                ],
                vec![
                    text(cx, "mcAdditive", "txtDistanceNormal"),
                    text(cx, "mcAdditive", "txtDistanceBlur"),
                    text(cx, "mcNormal", "txtDistanceNormal"),
                    text(cx, "mcNormal", "txtDistanceBlur"),
                ],
            ),
        };
        board.set_visible(cx, false);
        Billboard { id: 0, board, names, extra }
    }

    fn init_ghost(&mut self, cx: &mut Ctx, name: &str, colour: u32) {
        let [_, r, g, b] = colour.to_be_bytes();
        let name = html_text(name);
        let bottom = format!("<font color=\"#{r:02X}{g:02X}{b:02X}\">{name}</font>");
        let top = format!("<font color=\"#FFFFFF\">{name}</font>");
        for (k, (c, _)) in self.names.iter_mut().enumerate() {
            let html = if k % 2 == 1 { &bottom } else { &top };
            c.set(cx, "htmlText", Value::str(html));
        }
    }

    fn set_texts(cx: &mut Ctx, texts: &mut [(Clip, Option<String>)], s: &str) {
        for (c, last) in texts {
            if last.as_deref() != Some(s) {
                c.set_text(cx, s);
                *last = Some(s.to_string());
            }
        }
    }

    fn set(&mut self, cx: &mut Ctx, x: f32, y: f32, alpha: f32, scale: f32) {
        if alpha <= 0.0 {
            self.board.set_visible(cx, false);
            return;
        }
        let i = DisplayInfo {
            x: f64::from(x),
            y: f64::from(y),
            xscale: f64::from(scale),
            yscale: f64::from(scale),
            alpha: f64::from(alpha),
            visible: true,
            ..Default::default()
        };
        self.board.set_info(cx, &i, DI_X | DI_Y | DI_XSCALE | DI_YSCALE | DI_ALPHA | DI_VISIBLE);
    }
}

struct Pool {
    kind: Kind,
    holder: Option<Clip>,
    free: Vec<Billboard>,
    used: Vec<Billboard>,
    unseen: Vec<u64>,
    created: u32,
}

impl Pool {
    fn new(kind: Kind) -> Pool {
        Pool { kind, holder: None, free: Vec::new(), used: Vec::new(), unseen: Vec::new(), created: 0 }
    }

    fn begin(&mut self) {
        self.unseen = self.used.iter().map(|b| b.id).collect();
    }

    fn get(&mut self, cx: &mut Ctx, id: u64) -> Option<(usize, bool)> {
        if let Some(k) = self.used.iter().position(|b| b.id == id) {
            self.unseen.retain(|&u| u != id);
            return Some((k, false));
        }
        let mut b = match self.free.pop() {
            Some(b) => b,
            None => {
                let holder = self.holder.as_mut()?;
                self.created += 1;
                let (prefix, symbol) = match self.kind {
                    Kind::Ghost => ("Ghost_", "GhostBillboardWrapper"),
                    Kind::Tombstone => ("Tombstone_", "TombstoneBillboardWrapper"),
                };
                let name = format!("{prefix}{}", self.created);
                let depth = holder.invoke(cx, "getNextHighestDepth", &[]);
                holder.invoke(cx, "attachMovie", &[Value::str(symbol), Value::str(&name), depth]);
                let clip = Clip::bind(cx, &format!("{}.{name}", holder.path));
                Billboard::new(cx, self.kind, &clip)
            }
        };
        b.id = id;
        self.used.push(b);
        Some((self.used.len() - 1, true))
    }

    fn end(&mut self, cx: &mut Ctx) {
        for id in std::mem::take(&mut self.unseen) {
            if let Some(k) = self.used.iter().position(|b| b.id == id) {
                let mut b = self.used.remove(k);
                b.board.set_visible(cx, false);
                b.id = 0;
                self.free.push(b);
            }
        }
    }
}

struct Billboards {
    clip: Option<Clip>,
    pool: Pool,
    offset: [f32; 2],
    holders: u32,
    drawn: bool,
    bound: bool,
}

impl Billboards {
    fn new(kind: Kind) -> Billboards {
        Billboards { clip: None, pool: Pool::new(kind), offset: [0.0; 2], holders: 0, drawn: false, bound: false }
    }

    fn bind(&mut self, cx: &mut Ctx, env: &mut HudEnv, clip: Clip) {
        self.offset = [env.values.float(id::FE_OFFSET_X), env.values.float(id::FE_OFFSET_Y)];
        self.holders += 1;
        let name = format!("holder_{}", self.holders);
        let mut c = clip.clone();
        let init = Value::Object(cx.new_object());
        let _ = cx.set_member(&init, "_x", Value::Number(0.0));
        let _ = cx.set_member(&init, "_y", Value::Number(0.0));
        let depth = Value::Number(f64::from(self.holders));
        c.invoke(cx, "attachMovie", &[Value::str("BillboardGroupHolder"), Value::str(&name), depth, init]);
        self.pool.holder = Some(Clip::bind(cx, &format!("{}.{name}", clip.path)));
        self.clip = Some(clip);
        self.bound = true;
    }
}

pub struct GhostData(Billboards);

impl Default for GhostData {
    fn default() -> Self {
        GhostData(Billboards::new(Kind::Ghost))
    }
}

impl GhostData {
    pub fn paths(&self) -> (Option<String>, Option<String>) {
        (self.0.clip.as_ref().map(|c| c.path.clone()), self.0.pool.holder.as_ref().map(|c| c.path.clone()))
    }
}

impl Component for GhostData {
    fn bindings(&self) -> &'static [&'static str] {
        &["db_hud_ghostdata"]
    }

    fn bind(&mut self, cx: &mut Ctx, env: &mut HudEnv, _name: &str, clip: Clip) {
        self.0.bind(cx, env, clip);
    }

    fn unbind(&mut self, _name: &str) {
        (self.0.bound, self.0.clip) = (false, None);
    }

    fn update(&mut self, cx: &mut Ctx, env: &mut HudEnv, _dt: f64) {
        let b = &mut self.0;
        if !b.bound || (b.drawn && !env.values.is_dirty(id::GHOST_LIST)) {
            return;
        }
        let started = env.values.bool(id::RACE_STARTED);
        let list = env.values.markers(id::GHOST_LIST).to_vec();
        b.pool.begin();
        for m in &list {
            let Some((k, new)) = b.pool.get(cx, m.id) else { continue };
            let bb = &mut b.pool.used[k];
            if new {
                bb.init_ghost(cx, &m.name, m.colour);
            }
            let alpha = if started { m.alpha } else { 0.0 };
            bb.set(cx, m.x + b.offset[0], m.y + b.offset[1], alpha, m.scale);
        }
        b.pool.end(cx);
        b.drawn = true;
    }
}

pub struct Tombstones(Billboards);

impl Default for Tombstones {
    fn default() -> Self {
        Tombstones(Billboards::new(Kind::Tombstone))
    }
}

impl Component for Tombstones {
    fn bindings(&self) -> &'static [&'static str] {
        &["db_hud_tombstonedata"]
    }

    fn bind(&mut self, cx: &mut Ctx, env: &mut HudEnv, _name: &str, clip: Clip) {
        self.0.bind(cx, env, clip);
    }

    fn unbind(&mut self, _name: &str) {
        (self.0.bound, self.0.clip) = (false, None);
    }

    fn update(&mut self, cx: &mut Ctx, env: &mut HudEnv, _dt: f64) {
        let b = &mut self.0;
        if !b.bound || (b.drawn && !env.values.is_dirty(id::TOMBSTONE_LIST)) {
            return;
        }
        let list = env.values.markers(id::TOMBSTONE_LIST).to_vec();
        b.pool.begin();
        for m in &list {
            let Some((k, _)) = b.pool.get(cx, m.id) else { continue };
            let distance = env.loc.distance_with_unit(f64::from(m.distance));
            let bb = &mut b.pool.used[k];
            Billboard::set_texts(cx, &mut bb.names, &m.name);
            Billboard::set_texts(cx, &mut bb.extra, &distance);
            bb.set(cx, m.x + b.offset[0], m.y + b.offset[1], m.alpha, m.scale);
        }
        b.pool.end(cx);
        b.drawn = true;
    }
}
