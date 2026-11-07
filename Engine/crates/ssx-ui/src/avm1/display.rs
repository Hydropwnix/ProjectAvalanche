use std::sync::Arc;

use super::interp::Frame;
use super::object::{DisplayRef, Kind, ObjId, Object};
use super::value::{Str, Value, to_int32};
use super::{Ctx, Fault};
use crate::display::{AutoSize, GeomData, InstanceId, InstanceKind, TextAutoSize, TextFilter, TextShadow};
use crate::geom::Mat2d;

pub const PROPERTY_NAMES: [&str; 22] = [
    "_x",
    "_y",
    "_xscale",
    "_yscale",
    "_currentframe",
    "_totalframes",
    "_alpha",
    "_visible",
    "_width",
    "_height",
    "_rotation",
    "_target",
    "_framesloaded",
    "_name",
    "_droptarget",
    "_url",
    "_highquality",
    "_focusrect",
    "_soundbuftime",
    "_quality",
    "_xmouse",
    "_ymouse",
];

pub fn is_display_property(name: &str) -> bool {
    name.starts_with('_')
        && (PROPERTY_NAMES.iter().any(|p| p.eq_ignore_ascii_case(name))
            || name.eq_ignore_ascii_case("_parent")
            || name.eq_ignore_ascii_case("_root")
            || name.eq_ignore_ascii_case("_lockroot")
            || level_number(name).is_some())
}

fn level_number(name: &str) -> Option<i32> {
    let n = name.get(..6).filter(|p| p.eq_ignore_ascii_case("_level"))?;
    let _ = n;
    name[6..].parse().ok()
}

impl Ctx<'_> {
    pub fn object_for(&mut self, id: InstanceId) -> ObjId {
        if let Some(&o) = self.vm.display_objs.get(&id)
            && self.vm.heap.get(o).is_some()
        {
            return o;
        }
        let proto = match self.player.get(id).map(|i| &i.kind) {
            Some(InstanceKind::Sprite(_)) => self.vm.protos.movieclip,
            Some(InstanceKind::EditText(_)) => self.vm.protos.textfield,
            _ => self.vm.protos.object,
        };
        let path = Str::from(self.player.path(id));
        let o = self.vm.heap.alloc(Object::new(Value::Object(proto), Kind::Display(DisplayRef { id, path })));
        self.vm.display_objs.insert(id, o);
        self.player.set_scripted(id);
        o
    }

    pub fn resolve_handle(&mut self, id: ObjId) -> ObjId {
        let Some(Kind::Display(d)) = self.vm.heap.get(id).map(|o| &o.kind) else { return id };
        if self.player.get(d.id).is_some() {
            return id;
        }
        let path = d.path.clone();
        match self.instance_at_path(&path) {
            Some(i) => self.object_for(i),
            None => id,
        }
    }

    pub fn display_of(&mut self, v: &Value) -> Option<InstanceId> {
        let o = v.as_object()?;
        let d = match &self.vm.heap.get(o)?.kind {
            Kind::Display(d) => d.clone(),
            _ => return None,
        };
        if self.player.get(d.id).is_some() {
            return Some(d.id);
        }
        self.instance_at_path(&d.path)
    }

    pub fn instance_at_path(&self, path: &str) -> Option<InstanceId> {
        let mut parts = path.split('.');
        let level = level_number(parts.next()?)?;
        let mut cur = *self.player.levels.get(&level)?;
        for p in parts {
            cur = self.player.child_by_name(cur, p, true)?;
        }
        Some(cur)
    }

    pub fn child_named(&self, inst: InstanceId, name: &str) -> Option<InstanceId> {
        if name.is_empty() {
            return None;
        }
        self.player.child_by_name(inst, name, true)
    }

    pub fn root_value(&mut self, f: &Frame) -> Value {
        let base = f.target.map(Value::Object).unwrap_or_else(|| f.this.clone());
        match self.display_of(&base) {
            Some(i) => {
                let r = self.player.root_of(i);
                Value::Object(self.object_for(r))
            }
            None => match self.player.levels.get(&0).copied() {
                Some(r) => Value::Object(self.object_for(r)),
                None => Value::Undefined,
            },
        }
    }

    pub fn parent_value(&mut self, f: &Frame) -> Value {
        let base = f.target.map(Value::Object).unwrap_or_else(|| f.this.clone());
        match self.display_of(&base).and_then(|i| self.player.get(i)).and_then(|i| i.parent) {
            Some(p) => Value::Object(self.object_for(p)),
            None => Value::Undefined,
        }
    }
}

fn numeric_frame(s: &str) -> Option<i64> {
    let mut digit = false;
    for c in s.bytes() {
        match c {
            b'0'..=b'9' => digit = true,
            b'+' | b'-' if !digit => {}
            b' ' | b'\t' => {}
            _ => return None,
        }
    }
    Some(super::value::strtod_whole(s)? as i64)
}

fn slash_path(dot: &str) -> String {
    let mut parts = dot.split('.');
    let level = parts.next().unwrap_or("_level0");
    let rest: Vec<&str> = parts.collect();
    let prefix = if level == "_level0" { String::new() } else { level.to_string() };
    if rest.is_empty() {
        if prefix.is_empty() { "/".into() } else { prefix }
    } else {
        format!("{prefix}/{}", rest.join("/"))
    }
}

impl Ctx<'_> {
    pub fn display_get(&mut self, id: InstanceId, name: &str) -> Result<Option<Value>, Fault> {
        let Some(inst) = self.player.get(id) else { return Ok(None) };
        let m = inst.matrix;
        let g = inst.geom.unwrap_or_else(|| GeomData::from_matrix(&m));
        if name.starts_with('_') {
            let lower = name.to_ascii_lowercase();
            let v: Value = match lower.as_str() {
                "_x" => Value::Number(f64::from(g.x) * 0.05),
                "_y" => Value::Number(f64::from(g.y) * 0.05),
                "_xscale" => Value::Number(g.x_scale),
                "_yscale" => Value::Number(g.y_scale),
                "_rotation" => Value::Number(g.rotation),
                "_alpha" => Value::Number(f64::from(inst.cxform.mult[3]) * 100.0),
                "_visible" => Value::Bool(inst.visible),
                "_width" | "_height" => {
                    let b = self.player.local_bounds(self.lib, id).transform(&m);
                    let w = if b.is_empty() {
                        0.0
                    } else if lower == "_width" {
                        b.width()
                    } else {
                        b.height()
                    };
                    Value::Number(f64::from(w).floor() * 0.05)
                }
                "_currentframe" => Value::Number(f64::from(self.player.frames(id).map_or(1, |f| f.0.max(1)))),
                "_totalframes" | "_framesloaded" => Value::Number(f64::from(self.player.frames(id).map_or(1, |f| f.1))),
                "_name" => Value::str(&inst.name),
                "_target" => slash_path(&self.player.path(id)).into(),
                "_url" => format!("data/common/fe/{}", self.lib.movie(inst.movie).path).into(),
                "_parent" => match inst.parent {
                    Some(p) => Value::Object(self.object_for(p)),
                    None => Value::Undefined,
                },
                "_root" => {
                    let r = self.player.root_of(id);
                    Value::Object(self.object_for(r))
                }
                "_xmouse" | "_ymouse" => {
                    let p = self.vm.pointer.position;
                    match self.player.to_local(id, p) {
                        Some((x, y)) => {
                            let v = if lower == "_xmouse" { x } else { y };
                            Value::Number((f64::from(v) + 0.5).floor() * 0.05)
                        }
                        // TODO(guess): a clip whose matrix cannot be inverted reads 0; Scaleform's 0x8266EF10 with a singular matrix was not read; settle: the matrix inverse in 0x8266EF10.
                        None => Value::Number(0.0),
                    }
                }
                "_droptarget" => Value::str(""),
                "_highquality" | "_focusrect" => Value::Number(1.0),
                "_quality" => Value::str("HIGH"),
                "_soundbuftime" => Value::Number(5.0),
                "_lockroot" => Value::Bool(false),
                _ => match level_number(name).and_then(|l| self.player.levels.get(&l).copied()) {
                    Some(l) => Value::Object(self.object_for(l)),
                    None if level_number(name).is_some() => Value::Undefined,
                    None => return Ok(None),
                },
            };
            return Ok(Some(v));
        }
        if name == "filters" {
            let f = inst.filter;
            return Ok(Some(self.filters_value(f)?));
        }
        let Some(t) = inst.text() else { return Ok(None) };
        let v: Value = match name {
            "text" => self.player.text(id).unwrap_or_default().into(),
            "htmlText" => {
                if t.html {
                    Value::str(&t.source)
                } else {
                    self.player.text(id).unwrap_or_default().into()
                }
            }
            "html" => Value::Bool(t.html),
            "textColor" => {
                let c = t.default_char.color;
                let [r, g, b] = [c[0], c[1], c[2]].map(|x| (x * 255.0).round() as u32);
                Value::Number(f64::from(r << 16 | g << 8 | b))
            }
            "autoSize" => Value::str(match t.auto_size {
                AutoSize::None => "none",
                AutoSize::Left => "left",
                AutoSize::Center => "center",
                AutoSize::Right => "right",
            }),
            "wordWrap" => Value::Bool(t.word_wrap),
            "textAutoSize" => Value::str(match t.text_auto_size {
                TextAutoSize::None => "none",
                TextAutoSize::Shrink => "shrink",
                TextAutoSize::Fit => "fit",
            }),
            "textWidth" => Value::Number((f64::from(t.layout.text_width) / 20.0).round()),
            "textHeight" => Value::Number((f64::from(t.layout.text_height) / 20.0).round()),
            "length" => Value::Number(self.player.text(id).map_or(0, |s| s.chars().count()) as f64),
            "scroll" | "bottomScroll" => Value::Number(1.0),
            "maxscroll" => Value::Number(1.0),
            _ => return Ok(None),
        };
        Ok(Some(v))
    }

    fn set_geometry(&mut self, id: InstanceId, prop: &str, v: f64) {
        let ignored = match prop {
            "_xscale" | "_yscale" => v.is_infinite(),
            "_width" | "_height" => v == f64::NEG_INFINITY,
            _ => false,
        };
        if v.is_nan() || ignored {
            return;
        }
        let Some(cur) = self.player.get(id).map(|i| i.matrix) else { return };
        let Some(mut g) = self.player.ensure_geom(id) else { return };
        let (x0, y0, rot0) = g.orig_axes();
        let theta = |g: &GeomData| (g.rotation.to_radians() - rot0) as f32;
        let m = match prop {
            "_x" | "_y" => {
                let t = (if v.is_infinite() { 0.0 } else { v } * 20.0).floor() as i32;
                let mut m = cur;
                if prop == "_x" {
                    g.x = t;
                    m.tx = t as f32;
                } else {
                    g.y = t;
                    m.ty = t as f32;
                }
                m
            }
            "_xscale" | "_yscale" => {
                let (sx, sy) = if prop == "_xscale" {
                    g.x_scale = v;
                    let f = if x0 == 0.0 || v > 1e16 { 0.0 } else { v / (x0 * 100.0) };
                    (f, g.y_scale / (y0 * 100.0))
                } else {
                    g.y_scale = v;
                    let f = if y0 == 0.0 || v > 1e16 { 0.0 } else { v / (y0 * 100.0) };
                    (g.x_scale / (x0 * 100.0), f)
                };
                g.compose(&cur, sx as f32, sy as f32, theta(&g))
            }
            "_rotation" => {
                let mut r = v % 360.0;
                if r > 180.0 {
                    r -= 360.0;
                } else if r < -180.0 {
                    r += 360.0;
                }
                g.rotation = r;
                g.compose(&cur, (g.x_scale / (x0 * 100.0)) as f32, (g.y_scale / (y0 * 100.0)) as f32, theta(&g))
            }
            _ => {
                let v = if v.is_infinite() { 0.0 } else { v };
                let width = prop == "_width";
                let t = theta(&g);
                let turn = Mat2d { a: t.cos(), b: t.sin(), c: -t.sin(), d: t.cos(), tx: 0.0, ty: 0.0 };
                let at = Mat2d { tx: cur.tx, ty: cur.ty, ..g.orig };
                // TODO(guess): bounds under a matrix = the local bounds' rectangle transformed (differs from `GetBounds(matrix)` only under turn / skew); settle: the sprite class's vtable +0xD4.
                let b = self.player.local_bounds(self.lib, id).transform(&turn.mul(&at));
                let size = if width { b.width() } else { b.height() };
                let f = if size.abs() <= 1e-6 { 0.0 } else { (v * 20.0) as f32 / size };
                let (mut xs, mut ys) = (g.x_scale, g.y_scale);
                let (mut xd, mut yd) = (x0, y0);
                if width {
                    xs = f64::from(f) * x0 * 100.0;
                    g.x_scale = xs;
                    if x0 == 0.0 {
                        (xd, xs) = (1.0, 0.0);
                    }
                } else {
                    ys = f64::from(f) * y0 * 100.0;
                    g.y_scale = ys;
                    if y0 == 0.0 {
                        (yd, ys) = (1.0, 0.0);
                    }
                }
                let m = g.compose(&cur, (xs / (xd * 100.0)).abs() as f32, (ys / (yd * 100.0)).abs() as f32, t);
                g.x_scale = g.x_scale.abs();
                g.y_scale = g.y_scale.abs();
                m
            }
        };
        self.player.set_geom(id, g, m);
    }

    pub fn display_set(&mut self, id: InstanceId, name: &str, v: &Value) -> Result<bool, Fault> {
        if self.vm.watch.as_deref() == Some(name) {
            let n = self.vm.call_names.len();
            let line = format!(
                "watch: {}.{name} = {v:?} ← {}",
                self.player.path(id),
                self.vm.call_names[n.saturating_sub(6)..].join(" ← ")
            );
            self.vm.trace(&line);
        }
        let Some(inst) = self.player.get(id) else { return Ok(false) };
        let is_text = inst.text().is_some();
        if name.starts_with('_') {
            let lower = name.to_ascii_lowercase();
            if !is_display_property(&lower) {
                return Ok(false);
            }
            let n64 = self.to_number(v);
            let n = n64 as f32;
            let finite = n.is_finite();
            match lower.as_str() {
                "_x" | "_y" | "_xscale" | "_yscale" | "_rotation" | "_width" | "_height" => {
                    self.set_geometry(id, &lower, n64);
                }
                "_alpha" if finite => {
                    if let Some(mut c) = self.player.get(id).map(|i| i.cxform) {
                        c.mult[3] = n / 100.0;
                        self.player.set_cxform(id, c);
                    }
                }
                "_visible" => {
                    let b = self.to_bool(v);
                    self.player.set_visible(id, b);
                }
                "_name" => {
                    let s = self.to_string(v)?;
                    self.player.set_name(id, &s);
                }
                _ => {}
            }
            return Ok(true);
        }
        if name == "filters" {
            let f = self.filters_from(v)?;
            self.player.set_filter(id, f);
            return Ok(true);
        }
        if !is_text {
            return Ok(false);
        }
        match name {
            "text" | "htmlText" => {
                let s = self.to_string(v)?;
                let html = name == "htmlText";
                self.player.set_text(self.lib, id, &s, html);
            }
            "html" => {
                let b = self.to_bool(v);
                self.player.update_text(self.lib, id, |t| t.html = b);
            }
            "textColor" => {
                let c = to_int32(self.to_number(v)) as u32;
                let rgb = [(c >> 16) & 255, (c >> 8) & 255, c & 255].map(|x| x as f32 / 255.0);
                self.player.update_text(self.lib, id, |t| {
                    t.default_char.color[..3].copy_from_slice(&rgb);
                    for p in &mut t.paragraphs {
                        for r in &mut p.runs {
                            r.format.color[..3].copy_from_slice(&rgb);
                        }
                    }
                });
            }
            "autoSize" => {
                let s = match v {
                    Value::Bool(b) => (if *b { "left" } else { "none" }).to_string(),
                    _ => self.to_string(v)?.to_string(),
                };
                self.player.update_text(self.lib, id, |t| {
                    t.auto_size = match s.as_str() {
                        "none" => AutoSize::None,
                        "left" => AutoSize::Left,
                        "center" => AutoSize::Center,
                        "right" => AutoSize::Right,
                        _ if t.auto_size == AutoSize::None => AutoSize::Left,
                        _ => t.auto_size,
                    }
                });
            }
            "wordWrap" => {
                let b = self.to_bool(v);
                self.player.update_text(self.lib, id, |t| t.word_wrap = b);
            }
            "textAutoSize" => {
                let a = match &*self.to_string(v)? {
                    "shrink" => TextAutoSize::Shrink,
                    "fit" => TextAutoSize::Fit,
                    _ => TextAutoSize::None,
                };
                self.player.update_text(self.lib, id, |t| t.text_auto_size = a);
            }
            "textWidth" | "textHeight" | "length" | "maxscroll" | "bottomScroll" | "scroll" => {}
            _ => return Ok(false),
        }
        Ok(true)
    }
}

impl Ctx<'_> {
    fn frame_target(&mut self, f: &Frame) -> Option<InstanceId> {
        let t = f.target.map(Value::Object).unwrap_or_else(|| f.this.clone());
        self.display_of(&t)
    }

    pub(crate) fn frame_step(&mut self, f: &mut Frame, forward: bool) {
        if let Some(id) = self.frame_target(f) {
            self.player.step(self.lib, id, forward);
        }
    }

    pub(crate) fn frame_play(&mut self, f: &mut Frame, play: bool) {
        if let Some(id) = self.frame_target(f) {
            if play {
                self.player.play(id);
            } else {
                self.player.stop(id);
            }
        }
    }

    pub(crate) fn frame_goto(&mut self, f: &mut Frame, frame: Value, play: Option<bool>) -> Result<(), Fault> {
        if let Some(id) = self.frame_target(f) {
            self.goto_value(id, &frame, play)?;
        }
        Ok(())
    }

    pub fn goto_value(&mut self, id: InstanceId, frame: &Value, play: Option<bool>) -> Result<(), Fault> {
        self.goto_value_with(id, frame, play, true)
    }

    pub fn goto_value_with(
        &mut self,
        id: InstanceId,
        frame: &Value,
        play: Option<bool>,
        numbers: bool,
    ) -> Result<(), Fault> {
        let target = match frame {
            Value::String(s) => match numeric_frame(s).filter(|_| numbers) {
                Some(n) => Some(n.clamp(1, 65535) as u16),
                None => self.player.label_frame(id, s),
            },
            v => {
                let n = to_int32(self.to_number(v));
                Some(n.clamp(1, 65535) as u16)
            }
        };
        let Some(n) = target else { return Ok(()) };
        self.player.goto(self.lib, id, n);
        match play {
            Some(true) => self.player.play(id),
            Some(false) => self.player.stop(id),
            None => {}
        }
        Ok(())
    }

    pub(crate) fn set_target_value(&mut self, f: &mut Frame, t: &Value) -> Result<(), Fault> {
        let target = match t {
            Value::String(s) if s.is_empty() => f.scope_target(),
            Value::String(s) => {
                let s = s.clone();
                self.resolve_target_path(f, &s)?.and_then(|v| v.as_object())
            }
            v => v.as_object(),
        };
        f.target = target.or(f.target);
        Ok(())
    }

    fn property_target(&mut self, f: &mut Frame, t: &Value) -> Result<Option<InstanceId>, Fault> {
        let v = match t {
            Value::String(s) if s.is_empty() => f.target.map(Value::Object),
            Value::String(s) => {
                let s = s.clone();
                self.resolve_target_path(f, &s)?
            }
            v => Some(v.clone()),
        };
        Ok(v.and_then(|v| self.display_of(&v)))
    }

    pub(crate) fn get_property_action(&mut self, f: &mut Frame, t: &Value, i: &Value) -> Result<Value, Fault> {
        let idx = to_int32(self.to_number(i));
        let Some(name) = usize::try_from(idx).ok().and_then(|i| PROPERTY_NAMES.get(i)) else {
            return Ok(Value::Undefined);
        };
        match self.property_target(f, t)? {
            Some(id) => Ok(self.display_get(id, name)?.unwrap_or_default()),
            None => Ok(Value::Undefined),
        }
    }

    pub(crate) fn set_property_action(&mut self, f: &mut Frame, t: &Value, i: &Value, v: Value) -> Result<(), Fault> {
        let idx = to_int32(self.to_number(i));
        let Some(name) = usize::try_from(idx).ok().and_then(|i| PROPERTY_NAMES.get(i)) else { return Ok(()) };
        if let Some(id) = self.property_target(f, t)? {
            self.display_set(id, name, &v)?;
        }
        Ok(())
    }

    pub(crate) fn clone_sprite(
        &mut self,
        f: &mut Frame,
        src: &Value,
        name: &Value,
        depth: &Value,
    ) -> Result<(), Fault> {
        let Some(id) = self.property_target(f, src)? else { return Ok(()) };
        let name = self.to_string(name)?;
        let depth = to_int32(self.to_number(depth)) - 16384;
        if let Some(new) = self.player.duplicate_movie_clip(self.lib, id, &name, depth) {
            self.construct_clip(new, None)?;
        }
        Ok(())
    }

    pub(crate) fn remove_sprite(&mut self, f: &mut Frame, t: &Value) -> Result<(), Fault> {
        if let Some(id) = self.property_target(f, t)? {
            self.player.remove(id);
        }
        Ok(())
    }
}

impl Frame {
    pub fn scope_target(&self) -> Option<ObjId> {
        let mut s: Option<&Arc<super::object::Scope>> = Some(&self.scope);
        while let Some(sc) = s {
            if sc.kind == super::object::ScopeKind::Target {
                return Some(sc.obj);
            }
            s = sc.parent.as_ref();
        }
        None
    }
}

impl Ctx<'_> {
    fn filters_value(&mut self, f: Option<TextFilter>) -> Result<Value, Fault> {
        let mut items = Vec::new();
        if let Some(f) = f {
            if f.blur != (0.0, 0.0) {
                let o = Value::Object(self.new_object_with(self.vm.protos.filter, Kind::Plain));
                self.set_member(&o, "blurX", Value::Number(f64::from(f.blur.0) / 20.0))?;
                self.set_member(&o, "blurY", Value::Number(f64::from(f.blur.1) / 20.0))?;
                self.set_member(&o, "quality", Value::Number(if f.blur_high_quality { 2.0 } else { 1.0 }))?;
                items.push(o);
            }
            if let Some(s) = f.shadow {
                let o = Value::Object(self.new_object_with(self.vm.protos.filter, Kind::Plain));
                let [r, g, b, a] = s.color.map(|c| (c * 255.0).round() as u32);
                let distance = s.offset.0.hypot(s.offset.1) / 20.0;
                let angle = s.offset.1.atan2(s.offset.0).to_degrees();
                for (k, v) in [
                    ("color", Value::Number(f64::from(r << 16 | g << 8 | b))),
                    ("alpha", Value::Number(f64::from(a) / 255.0)),
                    ("blurX", Value::Number(f64::from(s.blur_x) / 20.0)),
                    ("blurY", Value::Number(f64::from(s.blur_y) / 20.0)),
                    ("strength", Value::Number(f64::from(s.strength))),
                    ("quality", Value::Number(if s.high_quality { 2.0 } else { 1.0 })),
                    ("distance", Value::Number(f64::from(distance))),
                    ("angle", Value::Number(f64::from(angle))),
                    ("knockout", Value::Bool(s.knockout)),
                    ("hideObject", Value::Bool(s.hide_object)),
                ] {
                    self.set_member(&o, k, v)?;
                }
                items.push(o);
            }
        }
        Ok(Value::Object(self.new_array(items)))
    }

    fn filters_from(&mut self, v: &Value) -> Result<Option<TextFilter>, Fault> {
        let items = match self.kind_of(v) {
            Some(Kind::Array(a)) => a.clone(),
            _ => return Ok(None),
        };
        let mut out = TextFilter::default();
        let mut any = false;
        for it in items.iter().take(4) {
            let num = |cx: &mut Ctx, k: &str, d: f64| -> Result<f64, Fault> {
                let x = cx.get_member(it, k)?;
                Ok(if x.is_undefined() { d } else { cx.to_number(&x) })
            };
            let bx = num(self, "blurX", 4.0)? as f32 * 20.0;
            let by = num(self, "blurY", 4.0)? as f32 * 20.0;
            let quality = num(self, "quality", 1.0)?;
            let has = |cx: &mut Ctx, k: &str| cx.get_member(it, k).map(|x| !x.is_undefined());
            if has(self, "color")? {
                let shadow = has(self, "distance")? || has(self, "angle")?;
                let c = num(self, "color", 0.0)? as u32;
                let alpha = num(self, "alpha", 1.0)? as f32;
                let offset = if shadow {
                    let (d, a) = (num(self, "distance", 4.0)? as f32, (num(self, "angle", 45.0)? as f32).to_radians());
                    crate::display::shadow_offset(d, a)
                } else {
                    (0.0, 0.0)
                };
                let knockout = self.get_member(it, "knockout")?;
                let hide = self.get_member(it, "hideObject")?;
                let [r, g, b] = [(c >> 16) & 255, (c >> 8) & 255, c & 255].map(|x| x as f32 / 255.0);
                out.shadow = Some(TextShadow {
                    color: [r, g, b, alpha],
                    blur_x: bx,
                    blur_y: by,
                    strength: num(self, "strength", 2.0)? as f32,
                    offset,
                    knockout: self.to_bool(&knockout),
                    hide_object: self.to_bool(&hide),
                    high_quality: quality > 1.0,
                });
            } else {
                out.blur = (bx, by);
                out.blur_high_quality = quality > 1.0;
            }
            any = true;
        }
        Ok(any.then_some(out))
    }
}
