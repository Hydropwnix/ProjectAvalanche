use mlua::Lua;
use ssx_content::{ContentId, Kind};

use super::meta::callback;
use super::{ApiError, Arg, Args, COLOUR, Ctx, NameKind, Param, Ret, Sides, Spec, Ty, WORLD};
use crate::host::elements::Creator;
use crate::host::game::keys::{BINDABLE, STATES, bindable};
use crate::host::game::markers::MarkerView;
use crate::host::game::{Bind, CameraOrder, FadeOrder, HudOrder, SoundOrder, vec3_value};

const fn spec(name: &'static str, params: &'static [Param], returns: &'static [Ty], call: super::Imp) -> Spec {
    Spec { name, sides: Sides::Client, api: 1, params, rest: None, returns, any_results: false, call }
}

const fn p(name: &'static str, ty: Ty) -> Param {
    Param { name, ty }
}

const ID: Param = p("id", Ty::Name(NameKind::ContentId));
const POS: Ty = Ty::Vec3 { max: WORLD };
const B: &[Ty] = &[Ty::Bool];

pub static SPECS: &[Spec] = &[
    spec(
        "createMarker",
        &[ID, p("position", POS), p("options", Ty::Opt(&Ty::Record("MarkerOptions")))],
        &[Ty::Element],
        create_marker,
    ),
    spec("setMarkerPosition", &[p("marker", Ty::Element), p("position", POS)], B, set_marker_position),
    spec("getMarkerPosition", &[p("marker", Ty::Element)], &[Ty::Record("Vec3")], get_marker_position),
    spec("setMarkerColor", &[p("marker", Ty::Element), p("color", COLOUR)], B, set_marker_colour),
    spec("setMarkerVisible", &[p("marker", Ty::Element), p("visible", Ty::Bool)], B, set_marker_visible),
    spec(
        "bindKey",
        &[p("key", Ty::Enum(BINDABLE)), p("state", Ty::Enum(STATES)), p("handler", Ty::Function)],
        B,
        bind_key,
    ),
    spec(
        "unbindKey",
        &[p("key", Ty::Enum(BINDABLE)), p("state", Ty::Opt(&Ty::Enum(STATES))), p("handler", Ty::Opt(&Ty::Function))],
        B,
        unbind_key,
    ),
    spec("setCameraTarget", &[p("rider", Ty::Opt(&Ty::Element))], B, set_camera_target),
    spec(
        "fadeCamera",
        &[
            p("fadeIn", Ty::Bool),
            p("seconds", Ty::Opt(&Ty::Number { min: 0.0, max: 60.0 })),
            p("color", Ty::Opt(&COLOUR)),
        ],
        B,
        fade_camera,
    ),
    spec("playSound", &[ID, p("position", Ty::Opt(&POS))], B, play_sound),
    spec("playMusic", &[ID], B, play_music),
    spec(
        "showHudMessage",
        &[
            p("title", Ty::Str { max: 256 }),
            p("text", Ty::Str { max: 1024 }),
            p("seconds", Ty::Opt(&Ty::Number { min: 0.5, max: 30.0 })),
        ],
        B,
        show_hud_message,
    ),
];

fn content_id(a: &Args, kind: Kind) -> Result<ContentId, ApiError> {
    let id = a.str(0).ok_or("no id")?.parse::<ContentId>().map_err(|e| ApiError::Refused(e.to_string()))?;
    if id.kind() != kind {
        return Err(format!("{id} is not a {kind} id").into());
    }
    Ok(id)
}

fn f32s(v: [f64; 3]) -> [f32; 3] {
    [v[0] as f32, v[1] as f32, v[2] as f32]
}

fn num_field(a: &Args, i: usize, name: &str) -> Option<f64> {
    match a.field(i, name) {
        Some(Arg::Num(n)) => Some(*n),
        Some(Arg::Int(n)) => Some(*n as f64),
        _ => None,
    }
}

fn create_marker(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let id = content_id(&a, Kind::Marker)?;
    let position = f32s(a.vec3(1).ok_or("no position")?);
    let host = ctx.host()?;
    let limits = host.limits;
    let mut w = host.lock();
    if w.running(&ctx.package).is_none() {
        return Err("the package is not running".into());
    }
    let Some(level) = w.game.frame.level.clone() else { return Err("no level is up (markers belong to one)".into()) };
    let meta = w.game.catalog.markers.get(&id).cloned().ok_or_else(|| format!("{id}: no source provides it"))?;
    let m = &w.game.markers;
    let mine = m.count_of(&ctx.package);
    if mine >= limits.game.markers_package {
        return Err(format!("the package has {mine} markers (at most {})", limits.game.markers_package).into());
    }
    let b = level.budget;
    let held = m.scripts_count();
    if held >= b.markers {
        return Err(format!("the level holds {held} markers (its budget: {})", b.markers).into());
    }
    if m.vertices + meta.vertices > b.vertices || m.triangles + meta.triangles > b.triangles {
        return Err(format!(
            "{id} ({} vertices, {} triangles) would pass the level's marker budget ({} / {} vertices, {} / {} \
             triangles held)",
            meta.vertices, meta.triangles, m.vertices, b.vertices, m.triangles, b.triangles
        )
        .into());
    }
    let p = w.running(&ctx.package).ok_or("the package is not running")?;
    if p.elements >= limits.package.elements {
        return Err(format!("the package has {} elements (at most {})", p.elements, limits.package.elements).into());
    }
    let creator = Creator::Package(ctx.package.clone());
    let element = w.tree.create("marker", ctx.root, creator, None, true, limits.tree_depth)?;
    if let Some(p) = w.packages.get_mut(&ctx.package) {
        p.elements += 1;
    }
    let size = num_field(&a, 2, "size").map_or(meta.size, |s| s as f32);
    let colour = match a.field(2, "color") {
        Some(Arg::Int(c)) => u32::try_from(*c).map_err(|_| "a colour beyond 32 bits")?,
        _ => meta.colour,
    };
    let visible = !matches!(a.field(2, "visible"), Some(Arg::Bool(false)));
    let heading = num_field(&a, 2, "heading").unwrap_or(0.0) as f32;
    w.game.markers.insert(MarkerView {
        element,
        package: ctx.package.clone(),
        id,
        position,
        heading,
        size,
        colour,
        visible,
        hit: meta.hit,
        hit_scale: 1.0,
        sound: meta.sound,
        vertices: meta.vertices,
        triangles: meta.triangles,
        inside: false,
        instance: None,
        elsewhere: false,
    });
    Ok(vec![Ret::Element(element)])
}

fn with_marker<R>(ctx: &Ctx, a: &Args, f: impl FnOnce(&mut MarkerView) -> R) -> Result<R, ApiError> {
    let el = a.element(0).ok_or("a marker expected")?;
    let host = ctx.host()?;
    let mut w = host.lock();
    if w.tree.get(el).is_none() {
        return Err(format!("element {el} does not exist (it was destroyed)").into());
    }
    let m = w.game.markers.by_element.get_mut(&el).ok_or_else(|| format!("element {el} is not a marker"))?;
    if m.package != ctx.package {
        return Err(format!("marker {el} is not this package's to change").into());
    }
    let r = f(m);
    w.game.markers.touch();
    Ok(r)
}

fn set_marker_position(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let p = f32s(a.vec3(1).ok_or("no position")?);
    with_marker(ctx, &a, |m| {
        m.position = p;
        m.inside = false;
    })?;
    Ok(vec![Ret::Bool(true)])
}

fn get_marker_position(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let el = a.element(0).ok_or("a marker expected")?;
    let host = ctx.host()?;
    let w = host.lock();
    let m = w.game.markers.by_element.get(&el).ok_or_else(|| format!("element {el} is not a marker"))?;
    Ok(vec![Ret::Value(vec3_value(m.position))])
}

fn set_marker_colour(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let c = u32::try_from(a.int(1).ok_or("no colour")?).map_err(|_| "a colour beyond 32 bits")?;
    with_marker(ctx, &a, |m| m.colour = c)?;
    Ok(vec![Ret::Bool(true)])
}

fn set_marker_visible(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let v = a.bool(1).ok_or("no visibility")?;
    with_marker(ctx, &a, |m| m.visible = v)?;
    Ok(vec![Ret::Bool(true)])
}

fn bind_key(ctx: &Ctx, lua: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let key = a.str(0).and_then(bindable).ok_or("no such key")?;
    let state = a.str(1).and_then(|s| STATES.iter().copied().find(|x| *x == s)).ok_or("no such state")?;
    let f = a.func(2).ok_or("no handler")?;
    let host = ctx.host()?;
    let cap = host.limits.game.binds_package;
    {
        let w = host.lock();
        let mine = w.game.binds.iter().filter(|b| b.package == ctx.package).count();
        if mine >= cap {
            return Err(format!("the package has {mine} key binds (at most {cap})").into());
        }
        let ptr = f.to_pointer().addr();
        if w.game
            .binds
            .iter()
            .any(|b| b.package == ctx.package && b.key == key && b.state == state && b.callback.ptr == ptr)
        {
            return Ok(vec![Ret::Bool(false)]);
        }
    }
    let callback = callback(lua, f)?;
    let mut w = host.lock();
    if w.running(&ctx.package).is_none() {
        return Err("the package is not running".into());
    }
    w.game.binds.push(Bind { package: ctx.package.clone(), key, state, callback });
    Ok(vec![Ret::Bool(true)])
}

fn unbind_key(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let key = a.str(0).and_then(bindable).ok_or("no such key")?;
    let state = a.str(1);
    let ptr = a.func(2).map(|f| f.to_pointer().addr());
    let host = ctx.host()?;
    let mut w = host.lock();
    let before = w.game.binds.len();
    w.game.binds.retain(|b| {
        !(b.package == ctx.package
            && b.key == key
            && state.is_none_or(|s| s == b.state)
            && ptr.is_none_or(|p| p == b.callback.ptr))
    });
    Ok(vec![Ret::Bool(w.game.binds.len() != before)])
}

fn set_camera_target(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let mut w = host.lock();
    let order = match a.element(0) {
        None => CameraOrder::Own,
        Some(el) => {
            let key = w.game.rider_key(el).ok_or_else(|| format!("element {el} is not a rider of the level"))?;
            if w.game.rider_view(key).is_some_and(|r| r.local.is_some()) {
                CameraOrder::Own
            } else {
                CameraOrder::Rider(key)
            }
        }
    };
    w.game.orders.camera = Some(order);
    Ok(vec![Ret::Bool(true)])
}

fn fade_camera(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let fade_in = a.bool(0).ok_or("no direction")?;
    let seconds = a.num(1).unwrap_or(1.0) as f32;
    let colour = match a.int(2) {
        Some(c) => u32::try_from(c).map_err(|_| "a colour beyond 32 bits")?,
        None => 0xFF00_0000,
    };
    let host = ctx.host()?;
    host.lock().game.orders.fade = Some(FadeOrder { fade_in, seconds, colour });
    Ok(vec![Ret::Bool(true)])
}

fn play_sound(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let id = content_id(&a, Kind::Sound)?;
    let position = a.vec3(1).map(f32s);
    let host = ctx.host()?;
    let caps = host.limits.game;
    let mut w = host.lock();
    if !w.game.catalog.ids.contains_key(&id) {
        return Err(format!("{id}: no source provides it").into());
    }
    let sounds = &w.game.orders.sounds;
    let mine = sounds.iter().filter(|s| s.package == ctx.package).count();
    if mine >= caps.sounds_frame_package || sounds.len() >= caps.sounds_frame {
        return Err(format!(
            "{} sounds this frame, {mine} of this package (at most {} and {})",
            sounds.len(),
            caps.sounds_frame,
            caps.sounds_frame_package
        )
        .into());
    }
    w.game.orders.sounds.push(SoundOrder { package: ctx.package.clone(), id, position });
    Ok(vec![Ret::Bool(true)])
}

fn play_music(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let id = content_id(&a, Kind::Music)?;
    let host = ctx.host()?;
    let every = host.limits.game.music_every;
    let mut w = host.lock();
    if !w.game.catalog.ids.contains_key(&id) {
        return Err(format!("{id}: no source provides it").into());
    }
    let now = w.now;
    if let Some(at) = w.game.music_at.get(&ctx.package)
        && now.saturating_sub(*at) < every
    {
        return Err(format!("one playMusic every {} ms per package", every.as_millis()).into());
    }
    w.game.music_at.insert(ctx.package.clone(), now);
    w.game.orders.music = Some((ctx.package.clone(), id));
    Ok(vec![Ret::Bool(true)])
}

fn show_hud_message(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let title = a.str(0).ok_or("no title")?.to_string();
    let text = a.str(1).ok_or("no text")?.to_string();
    let seconds = a.num(2).unwrap_or(4.0) as f32;
    let host = ctx.host()?;
    let every = host.limits.game.hud_every;
    let mut w = host.lock();
    if w.game.frame.hud_busy {
        return Ok(vec![Ret::Bool(false)]);
    }
    let now = w.now;
    if let Some(at) = w.game.hud_at.get(&ctx.package)
        && now.saturating_sub(*at) < every
    {
        return Err(format!("one HUD message every {} ms per package", every.as_millis()).into());
    }
    w.game.hud_at.insert(ctx.package.clone(), now);
    w.game.orders.hud.push(HudOrder { package: ctx.package.clone(), title, text, seconds });
    Ok(vec![Ret::Bool(true)])
}
