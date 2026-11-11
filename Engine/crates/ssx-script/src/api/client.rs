use mlua::Lua;
use ssx_content::{ContentId, Kind};

use super::{ApiError, Args, Ctx, NameKind, Param, Ret, Sides, Spec, Ty, WORLD};
use crate::host::game::{record, text, vec3_value};
use crate::value::NetValue;

const fn spec(name: &'static str, params: &'static [Param], returns: &'static [Ty], call: super::Imp) -> Spec {
    Spec { name, sides: Sides::Client, api: 1, params, rest: None, returns, any_results: false, call }
}

const fn p(name: &'static str, ty: Ty) -> Param {
    Param { name, ty }
}

pub const KIND_NAMES: &[&str] = &[
    "texture",
    "model",
    "object",
    "material",
    "character",
    "gear",
    "board",
    "anim",
    "track",
    "drop",
    "event",
    "lighting",
    "sky",
    "postfx",
    "particles",
    "sound",
    "music",
    "movie",
    "ui",
    "font",
    "string",
    "tuning",
    "scoring",
    "script",
    "marker",
    "map",
    "gamemode",
    "trick",
];

pub const MAX_LIST_CONTENT: usize = 65_536;
const PRELOADED: [Kind; 3] = [Kind::Texture, Kind::Sound, Kind::Marker];
const SKY: f32 = 1.0e5;

const ID: Param = p("id", Ty::Name(NameKind::ContentId));

pub static SPECS: &[Spec] = &[
    spec("getLocalRider", &[], &[Ty::Opt(&Ty::Element)], local_rider),
    spec("getRiders", &[], &[Ty::List(&Ty::Element)], riders),
    spec("getRiderState", &[p("rider", Ty::Element)], &[Ty::Opt(&Ty::Record("RiderState"))], rider_state),
    spec("getInstance", &[], &[Ty::Opt(&Ty::Record("Instance"))], instance),
    spec("getStandings", &[], &[Ty::List(&Ty::Record("Standing"))], standings),
    spec("getContent", &[ID], &[Ty::Opt(&Ty::Record("ContentInfo"))], content),
    spec("listContent", &[p("kind", Ty::Enum(KIND_NAMES))], &[Ty::List(&Ty::Str { max: 256 })], list_content),
    spec("preloadContent", &[ID], &[Ty::Bool], preload),
    spec(
        "getGroundHeight",
        &[p("x", Ty::Number { min: -WORLD, max: WORLD }), p("z", Ty::Number { min: -WORLD, max: WORLD })],
        &[Ty::Opt(&Ty::Number { min: -WORLD, max: WORLD })],
        ground_height,
    ),
    spec("captureMenuKeys", &[p("on", Ty::Bool)], &[Ty::Bool], capture_menu_keys),
    spec("getScreen", &[], &[Ty::Record("Screen")], screen),
    spec("getCursor", &[], &[Ty::Opt(&Ty::Record("Cursor"))], cursor),
    spec("isMultiplayer", &[], &[Ty::Bool], is_multiplayer),
];

fn local_rider(ctx: &Ctx, _: &Lua, _: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let w = host.lock();
    let g = &w.game;
    let local = g.frame.riders.iter().find(|r| r.local.is_some()).and_then(|r| g.rider_element(r.key));
    Ok(vec![local.map_or(Ret::Nil, Ret::Element)])
}

fn riders(ctx: &Ctx, _: &Lua, _: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let w = host.lock();
    let g = &w.game;
    Ok(vec![Ret::Elements(g.frame.riders.iter().filter_map(|r| g.rider_element(r.key)).collect())])
}

fn rider_state(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let el = a.element(0).ok_or("a rider expected")?;
    let host = ctx.host()?;
    let w = host.lock();
    if w.tree.get(el).is_none() {
        return Err(format!("element {el} does not exist (it was destroyed)").into());
    }
    let g = &w.game;
    let Some(r) = g.rider_key(el).and_then(|k| g.rider_view(k)) else { return Ok(vec![Ret::Nil]) };
    let l = r.local.as_ref();
    let num = |v: f64| Some(NetValue::Number(v));
    let v = record(vec![
        ("name", Some(text(&r.name))),
        ("position", Some(vec3_value(r.position))),
        ("velocity", l.map(|l| vec3_value(l.velocity))),
        ("speed", l.and_then(|l| num(f64::from(l.speed)))),
        ("air", l.map(|l| NetValue::Bool(l.air))),
        ("grind", l.map(|l| NetValue::Bool(l.grind))),
        ("trick", l.and_then(|l| l.trick.as_deref().map(text))),
        ("score", l.and_then(|l| num(l.score))),
        ("health", l.and_then(|l| num(f64::from(l.health.clamp(0.0, 1.0))))),
        ("finished", l.map(|l| NetValue::Bool(l.finished))),
        ("isLocal", Some(NetValue::Bool(l.is_some()))),
    ]);
    Ok(vec![Ret::Value(v)])
}

fn instance(ctx: &Ctx, _: &Lua, _: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let w = host.lock();
    let Some(i) = &w.game.frame.instance else { return Ok(vec![Ret::Nil]) };
    let v = record(vec![
        ("mode", Some(text(&i.mode))),
        ("drop", Some(text(&i.drop))),
        ("state", Some(text(i.state))),
        ("clock", Some(NetValue::Number(i.clock_ms.max(0.0)))),
    ]);
    Ok(vec![Ret::Value(v)])
}

fn standings(ctx: &Ctx, _: &Lua, _: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let w = host.lock();
    let g = &w.game;
    let rows = g.frame.standings.iter().enumerate().map(|(i, s)| {
        let row = record(vec![
            ("rider", s.rider.and_then(|k| g.rider_element(k)).map(NetValue::Element)),
            ("name", Some(text(&s.name))),
            ("place", Some(NetValue::Number(f64::from(s.place)))),
            ("value", Some(NetValue::Number(s.value))),
            ("finished", Some(NetValue::Bool(s.finished))),
        ]);
        (NetValue::Number((i + 1) as f64), row)
    });
    Ok(vec![Ret::Value(NetValue::Table(rows.collect()))])
}

fn content_id(a: &Args) -> Result<ContentId, ApiError> {
    a.str(0).ok_or("no id")?.parse::<ContentId>().map_err(|e| ApiError::Refused(e.to_string()))
}

fn content(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let id = content_id(&a)?;
    let host = ctx.host()?;
    let catalog = host.lock().game.catalog.clone();
    let Some(m) = catalog.ids.get(&id) else { return Ok(vec![Ret::Nil]) };
    let v = record(vec![
        ("id", Some(text(id.as_str()))),
        ("kind", Some(text(&id.kind().to_string()))),
        ("source", Some(text(&m.source))),
        ("layer", Some(text(m.layer))),
    ]);
    Ok(vec![Ret::Value(v)])
}

fn list_content(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let kind = a.str(0).and_then(Kind::parse).ok_or("no such kind")?;
    let host = ctx.host()?;
    let catalog = host.lock().game.catalog.clone();
    let n = catalog.kind(kind).count();
    if n > MAX_LIST_CONTENT {
        return Err(format!("{n} ids of kind {kind} (a list holds at most {MAX_LIST_CONTENT})").into());
    }
    Ok(vec![Ret::Strings(catalog.kind(kind).map(|id| id.as_str().to_string()).collect())])
}

fn preload(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let id = content_id(&a)?;
    if !PRELOADED.contains(&id.kind()) {
        return Ok(vec![Ret::Bool(false)]);
    }
    let host = ctx.host()?;
    let cap = host.limits.game.preload_frame;
    let mut w = host.lock();
    if !w.game.catalog.ids.contains_key(&id) {
        return Err(format!("{id}: no source provides it").into());
    }
    if w.game.orders.preload.len() >= cap {
        return Err(format!("{cap} preloads asked this frame (at most {cap})").into());
    }
    if !w.game.orders.preload.contains(&id) {
        w.game.orders.preload.push(id);
    }
    Ok(vec![Ret::Bool(true)])
}

fn ground_height(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let (x, z) = (a.num(0).unwrap_or(0.0) as f32, a.num(1).unwrap_or(0.0) as f32);
    let host = ctx.host()?;
    let ground = host.lock().game.frame.ground.clone();
    let Some(ground) = ground else { return Ok(vec![Ret::Nil]) };
    let h = ground.height(x, SKY, z).filter(|h| h.is_finite() && f64::from(h.abs()) <= WORLD);
    Ok(vec![h.map_or(Ret::Nil, |h| Ret::Num(f64::from(h)))])
}

fn screen(ctx: &Ctx, _: &Lua, _: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let w = host.lock();
    let s = w.game.frame.screen.unwrap_or_default();
    let n = |v: f32| NetValue::Number(f64::from(v));
    let v = record(vec![
        ("width", Some(NetValue::Number(f64::from(s.width)))),
        ("height", Some(NetValue::Number(f64::from(s.height)))),
        ("left", Some(n(s.left))),
        ("top", Some(n(s.top))),
        ("right", Some(n(s.right))),
        ("bottom", Some(n(s.bottom))),
    ]);
    Ok(vec![Ret::Value(v)])
}

fn cursor(ctx: &Ctx, _: &Lua, _: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let w = host.lock();
    let Some((x, y)) = w.game.frame.cursor else { return Ok(vec![Ret::Nil]) };
    let n = |v: f32| Some(NetValue::Number(f64::from(v)));
    Ok(vec![Ret::Value(record(vec![("x", n(x)), ("y", n(y))]))])
}

fn is_multiplayer(ctx: &Ctx, _: &Lua, _: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let w = host.lock();
    Ok(vec![Ret::Bool(w.game.frame.multiplayer)])
}

fn capture_menu_keys(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let on = a.bool(0).ok_or("no on")?;
    let host = ctx.host()?;
    let mut w = host.lock();
    if on {
        w.game.capture.insert(ctx.package.clone());
    } else {
        w.game.capture.remove(&ctx.package);
    }
    Ok(vec![Ret::Bool(true)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kind_names_are_the_contents() {
        let ours: Vec<String> = Kind::ALL.iter().map(ToString::to_string).collect();
        assert_eq!(ours, KIND_NAMES);
    }
}
