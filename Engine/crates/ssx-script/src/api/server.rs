use mlua::Lua;
use ssx_content::{ContentId, Kind};

use super::{ApiError, Arg, Args, COLOUR, Ctx, NameKind, Param, Ret, Sides, Spec, Ty, WORLD};
use crate::host::elements::{ClientId, Creator, ROOT};
use crate::host::game::markers::{MarkerHitShape, MarkerView};
use crate::host::game::{record, text, vec3_value};
use crate::host::outbox::{Body, Destination, Recipients};
use crate::host::server::joins::{Decision, MAX_TEXT, decide};
use crate::host::server::rights::{
    Holder, MAX_COMMANDS_PER_FRAME, MAX_LINE, MAX_REASON, MAX_SECONDS, OperationOrder, Privileged, ResourceAction,
    check_reason, holds, require,
};
use crate::host::server::{MarkerKind, ServerMarker};
use crate::host::world::World;
use crate::value::{ElementId, NetValue};

const fn spec(name: &'static str, params: &'static [Param], returns: &'static [Ty], call: super::Imp) -> Spec {
    Spec { name, sides: Sides::Server, api: 1, params, rest: None, returns, any_results: false, call }
}

const fn p(name: &'static str, ty: Ty) -> Param {
    Param { name, ty }
}

const ID: Param = p("id", Ty::Name(NameKind::ContentId));
const POS: Ty = Ty::Vec3 { max: WORLD };
const B: &[Ty] = &[Ty::Bool];
const EL: Ty = Ty::Element;
const JOIN: Param = p("join", Ty::Int { min: 1, max: 1 << 53 });
const OPTIONS: Param = p("options", Ty::Opt(&Ty::Record("ServerMarkerOptions")));
const ORDERED: &[Ty] = &[Ty::Bool, Ty::Opt(&Ty::Str { max: 1024 })];
const SECONDS: Param = p("seconds", Ty::Opt(&Ty::Int { min: 1, max: MAX_SECONDS }));
const REASON: Param = p("reason", Ty::Opt(&Ty::Str { max: MAX_REASON }));
const RESOURCE: Param = p("name", Ty::Name(NameKind::Package));

pub static SPECS: &[Spec] = &[
    spec("getPlayers", &[], &[Ty::List(&Ty::Element)], players),
    spec("getPlayerName", &[p("player", EL)], &[Ty::Opt(&Ty::Str { max: 64 })], player_name),
    spec("getPlayerRider", &[p("player", EL)], &[Ty::Opt(&Ty::Element)], player_rider),
    spec("getRiderPlayer", &[p("rider", EL)], &[Ty::Opt(&Ty::Element)], rider_player),
    spec("getPlayerInstance", &[p("player", EL)], &[Ty::Opt(&Ty::Element)], player_instance),
    spec("getInstances", &[], &[Ty::List(&Ty::Element)], instances),
    spec("getInstance", &[p("instance", EL)], &[Ty::Opt(&Ty::Record("InstanceInfo"))], instance),
    spec("getStandings", &[p("instance", EL)], &[Ty::List(&Ty::Record("ServerStanding"))], standings),
    spec("getRiders", &[p("instance", Ty::Opt(&EL))], &[Ty::List(&Ty::Element)], riders),
    spec("getRiderState", &[p("rider", EL)], &[Ty::Opt(&Ty::Record("ServerRiderState"))], rider_state),
    spec("endRound", &[p("instance", EL), p("reason", Ty::Opt(&Ty::Str { max: 256 }))], B, end_round),
    spec("createMarker", &[ID, p("position", POS), OPTIONS], &[Ty::Element], create_marker),
    spec(
        "createCheckpoint",
        &[ID, p("position", POS), p("index", Ty::Int { min: 0, max: 65_535 }), OPTIONS],
        &[Ty::Element],
        create_checkpoint,
    ),
    spec(
        "createTrigger",
        &[p("position", POS), p("radius", Ty::Number { min: 0.1, max: 100.0 }), p("instance", Ty::Opt(&Ty::Element))],
        &[Ty::Element],
        create_trigger,
    ),
    spec("setMarkerPosition", &[p("marker", EL), p("position", POS)], B, set_marker_position),
    spec("getMarkerPosition", &[p("marker", EL)], &[Ty::Record("Vec3")], get_marker_position),
    spec("setMarkerColor", &[p("marker", EL), p("color", COLOUR)], B, set_marker_colour),
    spec("setMarkerVisible", &[p("marker", EL), p("visible", Ty::Bool)], B, set_marker_visible),
    spec("playSoundFor", &[p("to", Ty::Elements), ID, p("position", Ty::Opt(&POS))], B, play_sound_for),
    spec(
        "showNotice",
        &[
            p("to", Ty::Elements),
            p("title", Ty::Str { max: 256 }),
            p("text", Ty::Str { max: 1024 }),
            p("seconds", Ty::Opt(&Ty::Number { min: 0.5, max: 30.0 })),
        ],
        B,
        show_notice,
    ),
    spec("deferJoin", &[JOIN], B, defer_join),
    spec("acceptJoin", &[JOIN], B, accept_join),
    spec("refuseJoin", &[JOIN, p("reason", Ty::Str { max: MAX_TEXT })], B, refuse_join),
    spec("updateJoin", &[JOIN, p("text", Ty::Str { max: MAX_TEXT })], B, update_join),
    spec("kickPlayer", &[p("player", EL), REASON], ORDERED, kick_player),
    spec("banPlayer", &[p("player", EL), SECONDS, REASON], ORDERED, ban_player),
    spec("mutePlayer", &[p("player", EL), SECONDS, REASON], ORDERED, mute_player),
    spec("startResource", &[RESOURCE], ORDERED, start_resource),
    spec("stopResource", &[RESOURCE], ORDERED, stop_resource),
    spec("restartResource", &[RESOURCE], ORDERED, restart_resource),
    spec("executeCommand", &[p("line", Ty::Str { max: MAX_LINE })], ORDERED, execute_command),
    spec("hasObjectPermissionTo", &[p("object", EL), p("right", Ty::Str { max: 128 })], B, has_permission),
];

fn players(ctx: &Ctx, _: &Lua, _: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let w = host.lock();
    Ok(vec![Ret::Elements(w.clients.values().copied().collect())])
}

fn client_of(w: &World, a: &Args, i: usize) -> Result<ClientId, ApiError> {
    let el = a.element(i).ok_or("a player expected")?;
    w.client_of_player(el).ok_or_else(|| format!("element {el} is not a connected player").into())
}

fn player_name(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let w = host.lock();
    let c = client_of(&w, &a, 0)?;
    let name = w.player_of(c).and_then(|e| w.tree.get(e)).map(|e| e.name.clone());
    Ok(vec![name.map_or(Ret::Nil, Ret::Str)])
}

fn player_rider(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let w = host.lock();
    let c = client_of(&w, &a, 0)?;
    Ok(vec![w.server.riders.get(&c).map_or(Ret::Nil, |r| Ret::Element(r.element))])
}

fn rider_player(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let el = a.element(0).ok_or("a rider expected")?;
    let host = ctx.host()?;
    let w = host.lock();
    let player = w.server.rider_client(el).and_then(|c| w.player_of(c));
    Ok(vec![player.map_or(Ret::Nil, Ret::Element)])
}

fn player_instance(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let w = host.lock();
    let c = client_of(&w, &a, 0)?;
    let i = w.server.player(c).and_then(|p| p.instance).and_then(|i| w.server.instances.get(&i).copied());
    Ok(vec![i.map_or(Ret::Nil, Ret::Element)])
}

fn instances(ctx: &Ctx, _: &Lua, _: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let w = host.lock();
    Ok(vec![Ret::Elements(w.server.instances.values().copied().collect())])
}

fn instance_id(w: &World, a: &Args, i: usize) -> Result<u16, ApiError> {
    let el = a.element(i).ok_or("an instance expected")?;
    w.server.instance_id(el).ok_or_else(|| format!("element {el} is not an instance").into())
}

fn instance(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let w = host.lock();
    let id = instance_id(&w, &a, 0)?;
    let Some(i) = w.server.instance(id) else { return Ok(vec![Ret::Nil]) };
    let num = |v: f64| Some(NetValue::Number(v));
    let v = record(vec![
        ("id", num(f64::from(i.id))),
        ("mode", Some(text(&i.mode))),
        ("drop", Some(text(&i.drop))),
        ("state", Some(text(i.state))),
        ("round", num(f64::from(i.round))),
        ("clock", num(i.clock_ms.max(0.0))),
    ]);
    Ok(vec![Ret::Value(v)])
}

fn standings(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let w = host.lock();
    let id = instance_id(&w, &a, 0)?;
    let Some(i) = w.server.instance(id) else { return Ok(vec![Ret::Value(NetValue::Table(Vec::new()))]) };
    let rider_of = |n: u16| w.server.riders.values().find(|r| r.instance == id && r.number == n).map(|r| r.element);
    let rows = i.standings.iter().enumerate().map(|(k, s)| {
        let num = |v: f64| Some(NetValue::Number(v));
        let row = record(vec![
            ("rider", rider_of(s.rider).map(NetValue::Element)),
            ("name", Some(text(&s.name))),
            ("place", num(k as f64 + 1.0)),
            ("state", Some(text(s.state))),
            ("clock", num(f64::from(s.clock_ms))),
            ("points", num(f64::from(s.points))),
            ("split", s.split.map(|x| NetValue::Number(f64::from(x)))),
        ]);
        (NetValue::Number(k as f64 + 1.0), row)
    });
    Ok(vec![Ret::Value(NetValue::Table(rows.collect()))])
}

fn riders(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let w = host.lock();
    let only = match a.element(0) {
        Some(_) => Some(instance_id(&w, &a, 0)?),
        None => None,
    };
    let list = w.server.riders.values().filter(|r| only.is_none_or(|i| r.instance == i)).map(|r| r.element);
    Ok(vec![Ret::Elements(list.collect())])
}

fn rider_state(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let el = a.element(0).ok_or("a rider expected")?;
    let host = ctx.host()?;
    let w = host.lock();
    if w.tree.get(el).is_none() {
        return Err(format!("element {el} does not exist (it was destroyed)").into());
    }
    let Some(c) = w.server.rider_client(el) else { return Ok(vec![Ret::Nil]) };
    let (Some(p), Some(r)) = (w.server.player(c), w.server.riders.get(&c)) else { return Ok(vec![Ret::Nil]) };
    let num = |v: f64| Some(NetValue::Number(v));
    let run = p.run.as_ref();
    let speed = p.velocity.map(|v| f64::from((v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()));
    let v = record(vec![
        ("name", Some(text(&p.name))),
        ("number", num(f64::from(r.number))),
        ("progress", Some(text(p.progress))),
        ("status", Some(text(p.status))),
        ("ping", num(f64::from(p.ping_ms))),
        ("position", p.position.map(vec3_value)),
        ("velocity", p.velocity.map(vec3_value)),
        ("speed", speed.and_then(num)),
        ("split", run.and_then(|r| r.split).map(|s| NetValue::Number(f64::from(s)))),
        ("clock", run.and_then(|r| num(f64::from(r.clock_ms)))),
        ("points", run.and_then(|r| num(f64::from(r.points)))),
        ("finished", Some(NetValue::Bool(run.is_some_and(|r| r.state == "finished")))),
    ]);
    Ok(vec![Ret::Value(v)])
}

fn end_round(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let reason = a.str(1).unwrap_or("the server's scripts ended the round").to_string();
    let host = ctx.host()?;
    let mut w = host.lock();
    let id = instance_id(&w, &a, 0)?;
    let Some(round) = w.server.instance(id).filter(|i| matches!(i.state, "countdown" | "running")).map(|i| i.round)
    else {
        return Ok(vec![Ret::Bool(false)]);
    };
    if w.server.orders.end_round.iter().any(|o| o.instance == id) {
        return Ok(vec![Ret::Bool(false)]);
    }
    let reason = format!("{}: {reason}", ctx.package);
    w.server.orders.end_round.push(crate::host::server::EndRound { instance: id, round, reason });
    Ok(vec![Ret::Bool(true)])
}

fn content_id(a: &Args, i: usize, kind: Kind) -> Result<ContentId, ApiError> {
    let id = a.str(i).ok_or("no id")?.parse::<ContentId>().map_err(|e| ApiError::Refused(e.to_string()))?;
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

fn replicate_marker(w: &mut World, package: &str, element: ElementId, created: bool) {
    let Some(m) = w.server.markers.get(&element) else { return };
    let (kind, spec, parent) = (m.kind, m.spec(), w.tree.get(element).and_then(|e| e.parent).unwrap_or(ROOT));
    let replicate = |w: &mut World, body| {
        crate::host::ops::replicate(w, crate::host::Side::Server, Some(package), Recipients::All, body)
    };
    if created {
        let name = String::new();
        replicate(w, Body::Created { element, kind: kind.element_type().into(), name, parent });
    }
    if let Some(spec) = spec {
        replicate(w, Body::Marker { element, spec });
    }
}

fn place(ctx: &Ctx, a: &Args, kind: MarkerKind) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let limits = host.limits;
    let mut w = host.lock();
    let (id, position, options) = match kind {
        MarkerKind::Trigger => (None, f32s(a.vec3(0).ok_or("no position")?), None),
        MarkerKind::Marker => (Some(content_id(a, 0, Kind::Marker)?), f32s(a.vec3(1).ok_or("no position")?), Some(2)),
        MarkerKind::Checkpoint => {
            (Some(content_id(a, 0, Kind::Marker)?), f32s(a.vec3(1).ok_or("no position")?), Some(3))
        }
    };
    let p = w.running(&ctx.package).ok_or("the package is not running")?;
    if p.elements >= limits.package.elements {
        return Err(format!("the package has {} elements (at most {})", p.elements, limits.package.elements).into());
    }
    let mine = w.server.markers.values().filter(|m| m.view.package == ctx.package).count();
    if mine >= limits.game.markers_package {
        return Err(format!("the package has {mine} markers (at most {})", limits.game.markers_package).into());
    }
    let instance = match (kind, options) {
        (MarkerKind::Trigger, _) => a.element(2),
        (_, Some(o)) => match a.field(o, "instance") {
            Some(Arg::Element(e)) => Some(*e),
            _ => None,
        },
        _ => None,
    };
    if let Some(i) = instance
        && !w.server.instances.values().any(|e| *e == i)
    {
        return Err(format!("element {i} is not an instance").into());
    }
    let view = match &id {
        Some(id) => {
            let meta = w.game.catalog.markers.get(id).cloned().ok_or_else(|| format!("{id}: no source provides it"))?;
            let o = options.unwrap_or(0);
            let colour = match a.field(o, "color") {
                Some(Arg::Int(c)) => u32::try_from(*c).map_err(|_| "a colour beyond 32 bits")?,
                _ => meta.colour,
            };
            MarkerView {
                element: ElementId(0),
                package: ctx.package.clone(),
                id: id.clone(),
                position,
                heading: num_field(a, o, "heading").unwrap_or(0.0) as f32,
                size: num_field(a, o, "size").map_or(meta.size, |s| s as f32),
                colour,
                visible: !matches!(a.field(o, "visible"), Some(Arg::Bool(false))),
                hit: meta.hit,
                hit_scale: 1.0,
                sound: None,
                vertices: meta.vertices,
                triangles: meta.triangles,
                inside: false,
                instance,
                elsewhere: false,
            }
        }
        None => MarkerView {
            element: ElementId(0),
            package: ctx.package.clone(),
            id: "ava:marker/trigger".parse::<ContentId>().map_err(|e| ApiError::Refused(e.to_string()))?,
            position,
            heading: 0.0,
            size: a.num(1).ok_or("no radius")? as f32,
            colour: 0,
            visible: false,
            hit: MarkerHitShape::Cylinder,
            hit_scale: 1.0,
            sound: None,
            vertices: 0,
            triangles: 0,
            inside: false,
            instance,
            elsewhere: false,
        },
    };
    let index = (kind == MarkerKind::Checkpoint).then(|| a.int(2).and_then(|i| u16::try_from(i).ok())).flatten();
    let creator = Creator::Package(ctx.package.clone());
    let element = w.tree.create(kind.element_type(), ctx.root, creator, None, false, limits.tree_depth)?;
    if let Some(p) = w.packages.get_mut(&ctx.package) {
        p.elements += 1;
    }
    let view = MarkerView { element, ..view };
    w.server.markers.insert(element, ServerMarker { view, kind, index, inside: Default::default() });
    replicate_marker(&mut w, &ctx.package, element, true);
    Ok(vec![Ret::Element(element)])
}

fn create_marker(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    place(ctx, &a, MarkerKind::Marker)
}

fn create_checkpoint(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    place(ctx, &a, MarkerKind::Checkpoint)
}

fn create_trigger(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    place(ctx, &a, MarkerKind::Trigger)
}

fn with_marker(ctx: &Ctx, a: &Args, f: impl FnOnce(&mut ServerMarker)) -> Result<Vec<Ret>, ApiError> {
    let el = a.element(0).ok_or("a marker expected")?;
    let host = ctx.host()?;
    let mut w = host.lock();
    let m = w.server.markers.get_mut(&el).ok_or_else(|| format!("element {el} is not a marker"))?;
    if m.view.package != ctx.package {
        return Err(format!("marker {el} is not this package's to change").into());
    }
    f(m);
    replicate_marker(&mut w, &ctx.package, el, false);
    Ok(vec![Ret::Bool(true)])
}

fn set_marker_position(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let p = f32s(a.vec3(1).ok_or("no position")?);
    with_marker(ctx, &a, |m| {
        m.view.position = p;
        m.inside.clear();
    })
}

fn get_marker_position(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let el = a.element(0).ok_or("a marker expected")?;
    let host = ctx.host()?;
    let w = host.lock();
    let m = w.server.markers.get(&el).ok_or_else(|| format!("element {el} is not a marker"))?;
    Ok(vec![Ret::Value(vec3_value(m.view.position))])
}

fn set_marker_colour(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let c = u32::try_from(a.int(1).ok_or("no colour")?).map_err(|_| "a colour beyond 32 bits")?;
    with_marker(ctx, &a, |m| m.view.colour = c)
}

fn set_marker_visible(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let v = a.bool(1).ok_or("no visibility")?;
    with_marker(ctx, &a, |m| {
        if m.kind != MarkerKind::Trigger {
            m.view.visible = v;
        }
    })
}

fn targets(w: &World, a: &Args) -> Result<Recipients, ApiError> {
    let Some(Arg::Elements(to)) = a.fixed.first() else { return Err("no recipients".into()) };
    if to.contains(&ROOT) {
        return Ok(Recipients::All);
    }
    let mut out = Vec::new();
    for e in to {
        if let Some(c) = w.client_of_player(*e) {
            out.push(c);
        } else if let Some(i) = w.server.instance_id(*e) {
            out.extend(w.server.clients_in(i));
        } else {
            return Err(format!("element {e} is neither the root, an instance nor a connected player").into());
        }
    }
    out.sort_unstable();
    out.dedup();
    Ok(Recipients::Some(out))
}

fn play_sound_for(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let id = content_id(&a, 1, Kind::Sound)?;
    let position = a.vec3(2).map(f32s);
    let host = ctx.host()?;
    let limits = host.limits;
    let mut w = host.lock();
    let to = targets(&w, &a)?;
    let body = Body::Sound { id: id.to_string(), position };
    super::events::send(&mut w, ctx, &limits, Destination::Clients(to), body)?;
    Ok(vec![Ret::Bool(true)])
}

fn show_notice(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let title = a.str(1).ok_or("no title")?.to_string();
    let text = a.str(2).ok_or("no text")?.to_string();
    let seconds = a.num(3).unwrap_or(4.0) as f32;
    let host = ctx.host()?;
    let limits = host.limits;
    let mut w = host.lock();
    let to = targets(&w, &a)?;
    super::events::send(&mut w, ctx, &limits, Destination::Clients(to), Body::Notice { title, text, seconds })?;
    Ok(vec![Ret::Bool(true)])
}

fn join_id(a: &Args) -> Result<u64, ApiError> {
    a.int(0).and_then(|i| u64::try_from(i).ok()).ok_or_else(|| "a join id expected".into())
}

fn decide_join(ctx: &Ctx, a: &Args, what: Decision) -> Result<Vec<Ret>, ApiError> {
    let id = join_id(a)?;
    let host = ctx.host()?;
    let mut w = host.lock();
    Ok(vec![Ret::Bool(decide(&mut w.server.joins, &ctx.package, id, what)?)])
}

fn defer_join(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    decide_join(ctx, &a, Decision::Defer)
}

fn accept_join(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    decide_join(ctx, &a, Decision::Accept)
}

fn refuse_join(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let reason = a.str(1).ok_or("no reason")?.to_string();
    decide_join(ctx, &a, Decision::Refuse(reason))
}

fn update_join(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let t = a.str(1).ok_or("no text")?.to_string();
    decide_join(ctx, &a, Decision::Update(t))
}

fn privileged(
    w: &mut World,
    package: &str,
    name: &str,
    more: Option<String>,
    order: impl FnOnce(&World) -> Result<OperationOrder, String>,
) -> Result<Vec<Ret>, ApiError> {
    let right = format!("function.{name}");
    let made = require(w, package, &right)
        .and_then(|()| more.as_deref().map_or(Ok(()), |r| require(w, package, r)))
        .and_then(|()| order(w))
        .and_then(|o| admit(w, o));
    match made {
        Ok(order) => {
            w.server.orders.operation.push(Privileged { package: package.to_string(), right, order });
            w.server.rights_counts.ordered += 1;
            Ok(vec![Ret::Bool(true), Ret::Nil])
        }
        Err(e) => {
            if let Some(p) = w.packages.get(package) {
                p.vm.note(format!("{name}: refused: {e}"));
            }
            Ok(vec![Ret::Bool(false), Ret::Str(e)])
        }
    }
}

fn admit(w: &World, order: OperationOrder) -> Result<OperationOrder, String> {
    let given = &w.server.orders.operation;
    let same_player = |o: &OperationOrder| match (o, &order) {
        (OperationOrder::Kick { client: a, .. }, OperationOrder::Kick { client: b, .. })
        | (OperationOrder::Ban { client: a, .. }, OperationOrder::Ban { client: b, .. })
        | (OperationOrder::Mute { client: a, .. }, OperationOrder::Mute { client: b, .. }) => a == b,
        _ => false,
    };
    if given.iter().any(|p| same_player(&p.order)) {
        return Err("the player has that order already this tick".into());
    }
    let heavy = |o: &OperationOrder| matches!(o, OperationOrder::Resource { .. } | OperationOrder::Command { .. });
    if heavy(&order) && given.iter().filter(|p| heavy(&p.order)).count() >= MAX_COMMANDS_PER_FRAME {
        return Err(format!("{MAX_COMMANDS_PER_FRAME} resource orders and commands this tick already (at most)"));
    }
    Ok(order)
}

fn reason_arg(w: &World, a: &Args, i: usize) -> Result<String, String> {
    let reason = a.str(i).unwrap_or_default();
    check_reason(w, reason)?;
    Ok(reason.to_string())
}

fn seconds_arg(a: &Args, i: usize) -> Option<u64> {
    a.int(i).and_then(|n| u64::try_from(n).ok())
}

fn kick_player(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let mut w = host.lock();
    let client = client_of(&w, &a, 0)?;
    privileged(&mut w, &ctx.package, "kickPlayer", None, |w| {
        Ok(OperationOrder::Kick { client, reason: reason_arg(w, &a, 1)? })
    })
}

fn ban_player(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let mut w = host.lock();
    let client = client_of(&w, &a, 0)?;
    privileged(&mut w, &ctx.package, "banPlayer", None, |w| {
        Ok(OperationOrder::Ban { client, seconds: seconds_arg(&a, 1), reason: reason_arg(w, &a, 2)? })
    })
}

fn mute_player(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let mut w = host.lock();
    let client = client_of(&w, &a, 0)?;
    privileged(&mut w, &ctx.package, "mutePlayer", None, |w| {
        Ok(OperationOrder::Mute { client, seconds: seconds_arg(&a, 1), reason: reason_arg(w, &a, 2)? })
    })
}

fn resource_order(ctx: &Ctx, a: &Args, name: &str, action: ResourceAction) -> Result<Vec<Ret>, ApiError> {
    let resource = a.str(0).ok_or("a resource name expected")?.to_string();
    let host = ctx.host()?;
    let mut w = host.lock();
    privileged(&mut w, &ctx.package, name, None, |_| Ok(OperationOrder::Resource { action, name: resource }))
}

fn start_resource(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    resource_order(ctx, &a, "startResource", ResourceAction::Start)
}

fn stop_resource(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    resource_order(ctx, &a, "stopResource", ResourceAction::Stop)
}

fn restart_resource(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    resource_order(ctx, &a, "restartResource", ResourceAction::Restart)
}

fn execute_command(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let line = a.str(0).ok_or("a command line expected")?.trim().to_string();
    let host = ctx.host()?;
    let mut w = host.lock();
    let Some(word) = line.split_whitespace().next() else {
        return privileged(&mut w, &ctx.package, "executeCommand", None, |_| Err("an empty command line".into()));
    };
    let command_right = format!("command.{word}");
    privileged(&mut w, &ctx.package, "executeCommand", Some(command_right), |_| Ok(OperationOrder::Command { line }))
}

fn has_permission(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let el = a.element(0).ok_or("a player or a resource expected")?;
    let right = a.str(1).ok_or("a right expected")?;
    let host = ctx.host()?;
    let mut w = host.lock();
    let package = w.packages.iter().find(|(_, p)| p.root == el).map(|(n, _)| n.clone());
    let holder = match (w.client_of_player(el), package.as_deref()) {
        (Some(c), _) => Holder::Player(c),
        (None, Some(name)) => Holder::Package(name),
        (None, None) => return Err(format!("element {el} is neither a connected player nor a running resource").into()),
    };
    Ok(vec![Ret::Bool(holds(&mut w, holder, right)?)])
}
