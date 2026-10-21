use super::{Builder, R, arg};
use crate::avm1::object::{Kind, ObjId};
use crate::avm1::runtime::{LoadRequest, LoadTarget};
use crate::avm1::value::{Value, to_int32};
use crate::avm1::{ClipEvent, Ctx, Fault, Job};
use crate::display::InstanceId;
use crate::geom::{Mat2d, RectF};
use crate::text::Align;

pub(crate) fn install(b: &mut Builder, global: ObjId) -> (ObjId, ObjId, ObjId, ObjId) {
    let mc = b.obj();
    b.class(global, "MovieClip", |_, this, _| Ok(this), mc);
    b.methods(
        mc,
        &[
            ("play", |cx, t, _| clip_op(cx, &t, |cx, id| cx.player.play(id))),
            ("stop", |cx, t, _| clip_op(cx, &t, |cx, id| cx.player.stop(id))),
            ("nextFrame", |cx, t, _| clip_op(cx, &t, |cx, id| cx.player.step(cx.lib, id, true))),
            ("prevFrame", |cx, t, _| clip_op(cx, &t, |cx, id| cx.player.step(cx.lib, id, false))),
            ("gotoAndPlay", |cx, t, a| goto(cx, &t, a, true)),
            ("gotoAndStop", |cx, t, a| goto(cx, &t, a, false)),
            ("getDepth", get_depth),
            ("getNextHighestDepth", get_next_highest_depth),
            ("swapDepths", swap_depths),
            ("getInstanceAtDepth", get_instance_at_depth),
            ("attachMovie", attach_movie),
            ("createEmptyMovieClip", create_empty_movie_clip),
            ("duplicateMovieClip", duplicate_movie_clip),
            ("removeMovieClip", remove_movie_clip),
            ("loadMovie", load_movie),
            ("unloadMovie", unload_movie),
            ("getBounds", get_bounds),
            ("getRect", get_bounds),
            ("localToGlobal", |cx, t, a| convert_point(cx, &t, a, true)),
            ("globalToLocal", |cx, t, a| convert_point(cx, &t, a, false)),
            ("hitTest", hit_test),
            ("getBytesLoaded", |_, _, _| Ok(Value::Number(1.0))),
            ("getBytesTotal", |_, _, _| Ok(Value::Number(1.0))),
            ("getSWFVersion", |cx, t, _| {
                Ok(cx.display_of(&t).map_or(Value::Number(-1.0), |id| {
                    Value::from(i32::from(cx.lib.movie(cx.player.get(id).map_or(0, |i| i.movie)).version))
                }))
            }),
            ("startDrag", super::no_op),
            ("stopDrag", super::no_op),
            ("setMask", |cx, _, _| unsupported(cx, "MovieClip.setMask")),
            ("createTextField", |cx, _, _| unsupported(cx, "MovieClip.createTextField")),
            ("attachBitmap", |cx, _, _| unsupported(cx, "MovieClip.attachBitmap")),
            ("clear", |cx, _, _| unsupported(cx, "MovieClip drawing API")),
            ("beginFill", |cx, _, _| unsupported(cx, "MovieClip drawing API")),
            ("lineStyle", |cx, _, _| unsupported(cx, "MovieClip drawing API")),
            ("moveTo", |cx, _, _| unsupported(cx, "MovieClip drawing API")),
            ("lineTo", |cx, _, _| unsupported(cx, "MovieClip drawing API")),
            ("endFill", |cx, _, _| unsupported(cx, "MovieClip drawing API")),
        ],
    );
    b.value(mc, "enabled", Value::Bool(true));
    b.value(mc, "useHandCursor", Value::Bool(true));

    let tf = b.obj();
    b.class(global, "TextField", |_, this, _| Ok(this), tf);
    b.methods(
        tf,
        &[
            ("setNewTextFormat", set_new_text_format),
            ("setTextFormat", set_text_format),
            ("getTextFormat", get_text_format),
            ("getNewTextFormat", get_text_format),
            ("getDepth", get_depth),
            ("swapDepths", swap_depths),
            ("removeTextField", remove_movie_clip),
        ],
    );
    let fmt = b.obj();
    b.class(global, "TextFormat", text_format_ctor, fmt);

    let loader = b.obj();
    b.class(global, "MovieClipLoader", loader_ctor, loader);
    b.methods(
        loader,
        &[
            ("loadClip", load_clip),
            ("unloadClip", unload_clip),
            ("addListener", super::misc::add_listener),
            ("removeListener", super::misc::remove_listener),
            ("getProgress", get_progress),
        ],
    );
    (mc, tf, fmt, loader)
}

fn unsupported(cx: &mut Ctx, what: &str) -> R {
    cx.vm.diag.note("unsupported", what);
    Ok(Value::Undefined)
}

fn clip_op(cx: &mut Ctx, this: &Value, f: impl FnOnce(&mut Ctx, InstanceId)) -> R {
    if let Some(id) = cx.display_of(this) {
        f(cx, id);
    }
    Ok(Value::Undefined)
}

fn goto(cx: &mut Ctx, this: &Value, a: &[Value], play: bool) -> R {
    if let (Some(id), Some(f)) = (cx.display_of(this), a.first()) {
        cx.goto_value(id, f, Some(play))?;
    }
    Ok(Value::Undefined)
}

fn get_depth(cx: &mut Ctx, this: Value, _: &[Value]) -> R {
    Ok(cx.display_of(&this).and_then(|id| cx.player.get(id)).map_or(Value::Undefined, |i| Value::from(i.depth)))
}

fn get_next_highest_depth(cx: &mut Ctx, this: Value, _: &[Value]) -> R {
    Ok(cx.display_of(&this).map_or(Value::Undefined, |id| Value::from(cx.player.next_highest_depth(id))))
}

fn swap_depths(cx: &mut Ctx, this: Value, a: &[Value]) -> R {
    let Some(id) = cx.display_of(&this) else { return Ok(Value::Undefined) };
    let t = arg(a, 0);
    let depth = match cx.display_of(&t) {
        Some(other) => {
            let (p1, p2) = (cx.player.get(id).and_then(|i| i.parent), cx.player.get(other).and_then(|i| i.parent));
            if p1 != p2 {
                return Ok(Value::Undefined);
            }
            cx.player.get(other).map(|i| i.depth)
        }
        None => {
            let n = cx.to_number(&t);
            n.is_finite().then_some(n as i32)
        }
    };
    if let Some(d) = depth {
        cx.player.swap_depths(id, d);
    }
    Ok(Value::Undefined)
}

fn get_instance_at_depth(cx: &mut Ctx, this: Value, a: &[Value]) -> R {
    let Some(id) = cx.display_of(&this) else { return Ok(Value::Undefined) };
    let d = to_int32(cx.to_number(&arg(a, 0)));
    Ok(cx.player.child_at_depth(id, d).map_or(Value::Undefined, |c| Value::Object(cx.object_for(c))))
}

fn adopt(cx: &mut Ctx, new: InstanceId, init: Option<&Value>) -> R {
    cx.construct_clip(new, init)?;
    let obj = cx.object_for(new);
    cx.vm.queue[2].push_back(Job::Event { clip: new, event: ClipEvent::Load });
    Ok(Value::Object(obj))
}

fn attach_movie(cx: &mut Ctx, this: Value, a: &[Value]) -> R {
    let Some(id) = cx.display_of(&this) else { return Ok(Value::Undefined) };
    if a.len() < 3 {
        return Ok(Value::Undefined);
    }
    let export = cx.to_string(&a[0])?;
    let name = cx.to_string(&a[1])?;
    let depth = cx.to_number(&a[2]) as i32;
    if depth + 16384 < 0 {
        cx.vm.diag.note("attachMovie failed", "depth below -16384");
        return Ok(Value::Undefined);
    }
    match cx.player.attach_movie(cx.lib, id, &export, &name, depth) {
        Some(new) => adopt(cx, new, a.get(3)),
        None => {
            let movie = cx.player.get(id).map_or(String::new(), |i| cx.lib.movie(i.movie).path.clone());
            cx.vm.diag.note("attachMovie failed", &format!("export name {export} not found (in {movie})"));
            Ok(Value::Undefined)
        }
    }
}

fn create_empty_movie_clip(cx: &mut Ctx, this: Value, a: &[Value]) -> R {
    let Some(id) = cx.display_of(&this) else { return Ok(Value::Undefined) };
    let name = cx.to_string(&arg(a, 0))?;
    let depth = cx.to_number(&arg(a, 1)) as i32;
    match cx.player.create_empty_movie_clip(id, &name, depth) {
        Some(new) => Ok(Value::Object(cx.object_for(new))),
        None => Ok(Value::Undefined),
    }
}

fn duplicate_movie_clip(cx: &mut Ctx, this: Value, a: &[Value]) -> R {
    let Some(id) = cx.display_of(&this) else { return Ok(Value::Undefined) };
    let name = cx.to_string(&arg(a, 0))?;
    let depth = cx.to_number(&arg(a, 1)) as i32;
    match cx.player.duplicate_movie_clip(cx.lib, id, &name, depth) {
        Some(new) => adopt(cx, new, a.get(2)),
        None => Ok(Value::Undefined),
    }
}

fn remove_movie_clip(cx: &mut Ctx, this: Value, _: &[Value]) -> R {
    let Some(id) = cx.display_of(&this) else { return Ok(Value::Undefined) };
    if cx.player.get(id).is_some_and(|i| i.depth >= 0 && i.parent.is_some()) {
        cx.player.remove(id);
    }
    Ok(Value::Undefined)
}

fn load_movie(cx: &mut Ctx, this: Value, a: &[Value]) -> R {
    let Some(o) = this.as_object() else { return Ok(Value::Undefined) };
    let url = cx.to_string(&arg(a, 0))?.to_string();
    cx.vm.loads.push(LoadRequest { url, target: LoadTarget::Clip(o), loader: None });
    Ok(Value::Undefined)
}

fn unload_movie(cx: &mut Ctx, this: Value, _: &[Value]) -> R {
    if let Some(o) = this.as_object().filter(|_| cx.display_of(&this).is_some()) {
        cx.vm.loads.push(LoadRequest { url: String::new(), target: LoadTarget::Clip(o), loader: None });
    }
    Ok(Value::Undefined)
}

pub(crate) fn world_matrix(cx: &Ctx, id: InstanceId) -> Mat2d {
    let mut m = Mat2d::IDENTITY;
    let mut cur = Some(id);
    while let Some(c) = cur {
        let Some(i) = cx.player.get(c) else { break };
        m = i.matrix.mul(&m);
        cur = i.parent;
    }
    m
}

fn rect_object(cx: &mut Ctx, r: RectF) -> Result<Value, Fault> {
    let o = Value::Object(cx.new_object());
    let r = if r.is_empty() { RectF::new(0.0, 0.0, 0.0, 0.0) } else { r };
    for (k, v) in [("xMin", r.x0), ("xMax", r.x1), ("yMin", r.y0), ("yMax", r.y1)] {
        cx.set_member(&o, k, Value::Number(f64::from(v) / 20.0))?;
    }
    Ok(o)
}

fn get_bounds(cx: &mut Ctx, this: Value, a: &[Value]) -> R {
    let Some(id) = cx.display_of(&this) else { return Ok(Value::Undefined) };
    let local = cx.player.local_bounds(cx.lib, id);
    let to = match a.first().and_then(|t| cx.display_of(t)) {
        Some(space) => world_matrix(cx, space).inverse().unwrap_or(Mat2d::IDENTITY).mul(&world_matrix(cx, id)),
        None if a.is_empty() => Mat2d::IDENTITY,
        None => world_matrix(cx, id),
    };
    let r = if local.is_empty() { local } else { local.transform(&to) };
    rect_object(cx, r)
}

fn convert_point(cx: &mut Ctx, this: &Value, a: &[Value], to_global: bool) -> R {
    let (Some(id), Some(pt)) = (cx.display_of(this), a.first().cloned()) else { return Ok(Value::Undefined) };
    let x = cx.get_member(&pt, "x")?;
    let y = cx.get_member(&pt, "y")?;
    let (x, y) = (cx.to_number(&x) as f32 * 20.0, cx.to_number(&y) as f32 * 20.0);
    let w = world_matrix(cx, id);
    let m = if to_global { w } else { w.inverse().unwrap_or(Mat2d::IDENTITY) };
    let (nx, ny) = m.apply(x, y);
    cx.set_member(&pt, "x", Value::Number(f64::from(nx) / 20.0))?;
    cx.set_member(&pt, "y", Value::Number(f64::from(ny) / 20.0))?;
    Ok(Value::Undefined)
}

fn hit_test(cx: &mut Ctx, this: Value, a: &[Value]) -> R {
    let Some(id) = cx.display_of(&this) else { return Ok(Value::Bool(false)) };
    let bounds = |cx: &Ctx, i: InstanceId| {
        let l = cx.player.local_bounds(cx.lib, i);
        if l.is_empty() { l } else { l.transform(&world_matrix(cx, i)) }
    };
    let mine = bounds(cx, id);
    if mine.is_empty() {
        return Ok(Value::Bool(false));
    }
    if let Some(other) = a.first().and_then(|t| cx.display_of(t)) {
        let o = bounds(cx, other);
        let hit = !o.is_empty() && mine.x0 <= o.x1 && o.x0 <= mine.x1 && mine.y0 <= o.y1 && o.y0 <= mine.y1;
        return Ok(Value::Bool(hit));
    }
    let x = cx.to_number(&arg(a, 0)) as f32 * 20.0;
    let y = cx.to_number(&arg(a, 1)) as f32 * 20.0;
    Ok(Value::Bool(mine.contains(x, y)))
}

fn text_format_ctor(cx: &mut Ctx, this: Value, a: &[Value]) -> R {
    const FIELDS: [&str; 13] = [
        "font",
        "size",
        "color",
        "bold",
        "italic",
        "underline",
        "url",
        "target",
        "align",
        "leftMargin",
        "rightMargin",
        "indent",
        "leading",
    ];
    for (i, f) in FIELDS.iter().enumerate() {
        cx.set_member(&this, f, a.get(i).cloned().unwrap_or(Value::Null))?;
    }
    Ok(this)
}

fn read_format(cx: &mut Ctx, fmt: &Value) -> Result<FormatPatch, Fault> {
    let mut p = FormatPatch::default();
    let num = |cx: &mut Ctx, k: &str| -> Result<Option<f32>, Fault> {
        let v = cx.get_member(fmt, k)?;
        Ok(if v.is_nullish() { None } else { Some(cx.to_number(&v) as f32).filter(|n| n.is_finite()) })
    };
    p.size = num(cx, "size")?.map(|v| v * 20.0);
    p.color = num(cx, "color")?.map(|v| v as u32);
    p.left_margin = num(cx, "leftMargin")?.map(|v| v * 20.0);
    p.right_margin = num(cx, "rightMargin")?.map(|v| v * 20.0);
    p.indent = num(cx, "indent")?.map(|v| v * 20.0);
    p.leading = num(cx, "leading")?.map(|v| v * 20.0);
    p.letter_spacing = num(cx, "letterSpacing")?.map(|v| v * 20.0);
    let font = cx.get_member(fmt, "font")?;
    if !font.is_nullish() {
        p.font = Some(cx.to_string(&font)?.to_string());
    }
    let align = cx.get_member(fmt, "align")?;
    if !align.is_nullish() {
        p.align = match &*cx.to_string(&align)? {
            "left" => Some(Align::Left),
            "right" => Some(Align::Right),
            "center" => Some(Align::Center),
            "justify" => Some(Align::Justify),
            _ => None,
        };
    }
    for (k, slot) in
        [("bold", &mut p.bold), ("italic", &mut p.italic), ("underline", &mut p.underline), ("kerning", &mut p.kerning)]
    {
        let v = cx.get_member(fmt, k)?;
        if !v.is_nullish() {
            *slot = Some(cx.to_bool(&v));
        }
    }
    Ok(p)
}

#[derive(Default)]
struct FormatPatch {
    font: Option<String>,
    size: Option<f32>,
    color: Option<u32>,
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<bool>,
    kerning: Option<bool>,
    letter_spacing: Option<f32>,
    align: Option<Align>,
    left_margin: Option<f32>,
    right_margin: Option<f32>,
    indent: Option<f32>,
    leading: Option<f32>,
}

impl FormatPatch {
    fn char(&self, c: &mut crate::text::CharFormat) {
        if let Some(f) = &self.font {
            c.face = Some(f.clone());
        }
        if let Some(s) = self.size {
            c.size = s;
        }
        if let Some(v) = self.color {
            c.color[..3].copy_from_slice(&[(v >> 16) & 255, (v >> 8) & 255, v & 255].map(|x| x as f32 / 255.0));
        }
        c.bold = self.bold.unwrap_or(c.bold);
        c.italic = self.italic.unwrap_or(c.italic);
        c.underline = self.underline.unwrap_or(c.underline);
        c.kerning = self.kerning.unwrap_or(c.kerning);
        c.letter_spacing = self.letter_spacing.unwrap_or(c.letter_spacing);
    }

    fn para(&self, p: &mut crate::text::ParaFormat) {
        p.align = self.align.unwrap_or(p.align);
        p.left_margin = self.left_margin.unwrap_or(p.left_margin);
        p.right_margin = self.right_margin.unwrap_or(p.right_margin);
        p.indent = self.indent.unwrap_or(p.indent);
        p.leading = self.leading.unwrap_or(p.leading);
    }
}

fn set_new_text_format(cx: &mut Ctx, this: Value, a: &[Value]) -> R {
    let Some(id) = cx.display_of(&this) else { return Ok(Value::Undefined) };
    let p = read_format(cx, &arg(a, 0))?;
    cx.player.update_text(cx.lib, id, |t| {
        p.char(&mut t.default_char);
        p.para(&mut t.default_para);
    });
    Ok(Value::Undefined)
}

// TODO(guess): character ranges are not split (the format applies to every run); settle: a ranged call on the disc (none in any constant pool).
fn set_text_format(cx: &mut Ctx, this: Value, a: &[Value]) -> R {
    let Some(id) = cx.display_of(&this) else { return Ok(Value::Undefined) };
    let fmt = a.last().cloned().unwrap_or_default();
    let p = read_format(cx, &fmt)?;
    cx.player.update_text(cx.lib, id, |t| {
        for para in &mut t.paragraphs {
            p.para(&mut para.format);
            for r in &mut para.runs {
                p.char(&mut r.format);
            }
        }
        p.char(&mut t.default_char);
        p.para(&mut t.default_para);
    });
    Ok(Value::Undefined)
}

fn get_text_format(cx: &mut Ctx, this: Value, _: &[Value]) -> R {
    let Some(t) = cx.display_of(&this).and_then(|id| cx.player.text_state(id)) else { return Ok(Value::Undefined) };
    let (c, p) = (t.default_char.clone(), t.default_para);
    let fmt = cx.new_object_with(cx.vm.protos.textformat, Kind::Plain);
    let f = Value::Object(fmt);
    let rgb = c.color.map(|x| (x * 255.0).round() as u32);
    let align = match p.align {
        Align::Left => "left",
        Align::Right => "right",
        Align::Center => "center",
        Align::Justify => "justify",
    };
    let fields: [(&str, Value); 11] = [
        ("font", c.face.as_deref().map_or(Value::Null, Value::str)),
        ("size", Value::Number(f64::from(c.size) / 20.0)),
        ("color", Value::Number(f64::from(rgb[0] << 16 | rgb[1] << 8 | rgb[2]))),
        ("bold", Value::Bool(c.bold)),
        ("italic", Value::Bool(c.italic)),
        ("underline", Value::Bool(c.underline)),
        ("align", Value::str(align)),
        ("leftMargin", Value::Number(f64::from(p.left_margin) / 20.0)),
        ("rightMargin", Value::Number(f64::from(p.right_margin) / 20.0)),
        ("indent", Value::Number(f64::from(p.indent) / 20.0)),
        ("leading", Value::Number(f64::from(p.leading) / 20.0)),
    ];
    for (k, v) in fields {
        cx.set_member(&f, k, v)?;
    }
    Ok(f)
}

fn loader_ctor(cx: &mut Ctx, this: Value, _: &[Value]) -> R {
    let l = cx.new_array(vec![this.clone()]);
    if let Some(o) = this.as_object() {
        cx.define(o, "_listeners", Value::Object(l), crate::avm1::object::flags::DONT_ENUM);
    }
    Ok(this)
}

fn load_clip(cx: &mut Ctx, this: Value, a: &[Value]) -> R {
    let url = cx.to_string(&arg(a, 0))?.to_string();
    let t = arg(a, 1);
    let target = match &t {
        Value::Object(o) => LoadTarget::Clip(*o),
        v => {
            let n = cx.to_number(v);
            if !n.is_finite() {
                return Ok(Value::Bool(false));
            }
            LoadTarget::Level(n as i32)
        }
    };
    cx.vm.loads.push(LoadRequest { url, target, loader: this.as_object() });
    Ok(Value::Bool(true))
}

fn unload_clip(cx: &mut Ctx, _: Value, a: &[Value]) -> R {
    let t = arg(a, 0);
    unload_movie(cx, t, &[])?;
    Ok(Value::Bool(true))
}

fn get_progress(cx: &mut Ctx, _: Value, _: &[Value]) -> R {
    let o = Value::Object(cx.new_object());
    cx.set_member(&o, "bytesLoaded", Value::Number(1.0))?;
    cx.set_member(&o, "bytesTotal", Value::Number(1.0))?;
    Ok(o)
}
