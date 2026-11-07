use mlua::Lua;

use super::meta::callback;
use super::{ApiError, Args, Ctx, NameKind, Param, Ret, Sides, Spec, Ty};
use crate::host::Side;
use crate::host::call::{Dispatch, dispatch};
use crate::host::elements::{Creator, ROOT};
use crate::host::events::{CLIENT_EVENTS, HandlerRec, SERVER_EVENTS};
use crate::host::outbox::{Body, Destination, Message, Recipients};
use crate::host::world::World;
use crate::value::{ElementId, NetValue, ValueLimits, encode_list, from_lua_multi};

const fn spec(
    name: &'static str,
    sides: Sides,
    params: &'static [Param],
    rest: Option<Ty>,
    returns: &'static [Ty],
    call: super::Imp,
) -> Spec {
    Spec { name, sides, api: 1, params, rest, returns, any_results: false, call }
}

const fn p(name: &'static str, ty: Ty) -> Param {
    Param { name, ty }
}

const NAME: Param = p("name", Ty::Name(NameKind::Event));
const SOURCE: Param = p("source", Ty::Element);
const RATE: Param = p("bandwidth", Ty::Int { min: 1, max: 100_000_000 });
const B: &[Ty] = &[Ty::Bool];

pub static SPECS: &[Spec] = &[
    spec("addEvent", Sides::Both, &[NAME, p("remote", Ty::Opt(&Ty::Bool))], None, B, add_event),
    spec(
        "addEventHandler",
        Sides::Both,
        &[NAME, p("element", Ty::Element), p("handler", Ty::Function), p("propagate", Ty::Opt(&Ty::Bool))],
        None,
        B,
        add_handler,
    ),
    spec(
        "removeEventHandler",
        Sides::Both,
        &[NAME, p("element", Ty::Element), p("handler", Ty::Function)],
        None,
        B,
        remove_handler,
    ),
    spec("triggerEvent", Sides::Both, &[NAME, SOURCE], Some(Ty::Value), B, trigger_event),
    spec("cancelEvent", Sides::Both, &[], None, B, cancel_event),
    spec("wasEventCancelled", Sides::Both, &[], None, B, was_cancelled),
    spec(
        "triggerClientEvent",
        Sides::Server,
        &[p("sendTo", Ty::Elements), NAME, SOURCE],
        Some(Ty::Value),
        B,
        trigger_client_event,
    ),
    spec("triggerServerEvent", Sides::Client, &[NAME, SOURCE], Some(Ty::Value), B, trigger_server_event),
    spec(
        "triggerLatentClientEvent",
        Sides::Server,
        &[p("sendTo", Ty::Elements), NAME, RATE, SOURCE],
        Some(Ty::Value),
        &[Ty::Latent],
        trigger_latent_client_event,
    ),
    spec(
        "triggerLatentServerEvent",
        Sides::Client,
        &[NAME, RATE, SOURCE],
        Some(Ty::Value),
        &[Ty::Latent],
        trigger_latent_server_event,
    ),
    spec(
        "getLatentEventStatus",
        Sides::Both,
        &[p("handle", Ty::Latent)],
        None,
        &[Ty::Opt(&Ty::Record("LatentStatus"))],
        latent_status,
    ),
    spec("cancelLatentEvent", Sides::Both, &[p("handle", Ty::Latent)], None, B, cancel_latent),
    spec("getLatentEventHandles", Sides::Both, &[], None, &[Ty::List(&Ty::Latent)], latent_handles),
];

fn host_events(side: Side) -> &'static [&'static str] {
    match side {
        Side::Server => SERVER_EVENTS,
        Side::Client => CLIENT_EVENTS,
    }
}

fn add_event(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let name = a.str(0).ok_or("no name")?;
    if SERVER_EVENTS.contains(&name) || CLIENT_EVENTS.contains(&name) {
        return Err(format!("{name} is the host's own event").into());
    }
    let host = ctx.host()?;
    let cap = host.limits.package.events;
    let mut w = host.lock();
    if !w.events.declared_by(name, &ctx.package) && w.events.declared_of(&ctx.package) >= cap {
        return Err(format!("the package declared {cap} events (the most)").into());
    }
    w.events.declare(&ctx.package, name, a.bool(1).unwrap_or(false));
    Ok(vec![Ret::Bool(true)])
}

fn add_handler(ctx: &Ctx, lua: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let name = a.str(0).ok_or("no name")?.to_string();
    let element = a.element(1).ok_or("no element")?;
    let f = a.func(2).ok_or("no handler")?;
    let propagate = a.bool(3).unwrap_or(true);
    let host = ctx.host()?;
    let ptr = f.to_pointer().addr();
    {
        let w = host.lock();
        if !w.events.is_declared(&name) && !host_events(ctx.side).contains(&name.as_str()) {
            return Err(format!("{name} is not declared (addEvent it first)").into());
        }
        if w.tree.get(element).is_none() {
            return Err(format!("element {element} does not exist").into());
        }
        if w.events.find_handler(&ctx.package, &name, element, ptr).is_some() {
            return Ok(vec![Ret::Bool(false)]);
        }
        let cap = host.limits.package.handlers;
        if w.events.handlers_of(&ctx.package) >= cap {
            return Err(format!("the package has {cap} event handlers (the most)").into());
        }
    }
    let cb = callback(lua, f)?;
    let rec = HandlerRec { package: ctx.package.clone(), event: name, element, propagate, callback: cb };
    host.lock().events.add_handler(rec);
    Ok(vec![Ret::Bool(true)])
}

fn remove_handler(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let name = a.str(0).ok_or("no name")?;
    let element = a.element(1).ok_or("no element")?;
    let ptr = a.func(2).ok_or("no handler")?.to_pointer().addr();
    let host = ctx.host()?;
    let mut w = host.lock();
    let Some(id) = w.events.find_handler(&ctx.package, name, element, ptr) else { return Ok(vec![Ret::Bool(false)]) };
    w.events.remove_handler(id);
    Ok(vec![Ret::Bool(true)])
}

fn args_of(a: &Args, limits: &ValueLimits) -> Result<Vec<NetValue>, ApiError> {
    Ok(from_lua_multi(&a.rest, limits).map_err(|e| format!("the arguments cannot be copied: {e}"))?)
}

fn trigger_event(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let name = a.str(0).ok_or("no name")?;
    let source = a.element(1).ok_or("no source")?;
    if host_events(ctx.side).contains(&name) || SERVER_EVENTS.contains(&name) || CLIENT_EVENTS.contains(&name) {
        return Err(format!("{name} is the host's own event: scripts cannot raise it").into());
    }
    let values = args_of(&a, &ValueLimits::LOCAL)?;
    let host = ctx.host()?;
    {
        let w = host.lock();
        if !w.events.is_declared(name) {
            return Err(format!("{name} is not declared").into());
        }
        if w.tree.get(source).is_none() {
            return Err(format!("element {source} does not exist").into());
        }
    }
    let cancelled = dispatch(&host, &Dispatch { name, source, args: &values, only: None, client: None })?;
    Ok(vec![Ret::Bool(!cancelled)])
}

fn cancel_event(ctx: &Ctx, _: &Lua, _: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let mut w = host.lock();
    if !crate::host::server::lifecycle::may_cancel(&mut w, &ctx.package) {
        return Ok(vec![Ret::Bool(false)]);
    }
    match w.frames.last_mut() {
        Some(f) => {
            *f = true;
            Ok(vec![Ret::Bool(true)])
        }
        None => Ok(vec![Ret::Bool(false)]),
    }
}

fn was_cancelled(ctx: &Ctx, _: &Lua, _: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let w = host.lock();
    Ok(vec![Ret::Bool(w.last_cancelled)])
}

fn recipients(w: &World, to: &[ElementId]) -> Result<Recipients, ApiError> {
    if to.contains(&ROOT) {
        return Ok(Recipients::All);
    }
    let mut out = Vec::new();
    for e in to {
        match w.client_of_player(*e) {
            Some(c) => out.push(c),
            None => return Err(format!("element {e} is neither the root nor a connected player").into()),
        }
    }
    out.sort_unstable();
    out.dedup();
    Ok(Recipients::Some(out))
}

pub(crate) fn send(
    w: &mut World,
    ctx: &Ctx,
    limits: &crate::host::HostLimits,
    to: Destination,
    body: Body,
) -> Result<(), ApiError> {
    let size = body.size();
    let caps = limits.package;
    let p = w.packages.get_mut(&ctx.package).ok_or("the package is not running")?;
    if p.tick_messages >= caps.tick_messages || p.tick_bytes + size > caps.tick_bytes {
        p.refusals += 1;
        return Err(format!(
            "the package sent {} messages, {} bytes this tick (at most {} and {})",
            p.tick_messages, p.tick_bytes, caps.tick_messages, caps.tick_bytes
        )
        .into());
    }
    w.outbox.push(Message { package: Some(ctx.package.clone()), to, body })?;
    let p = w.packages.get_mut(&ctx.package).ok_or("the package is not running")?;
    p.tick_messages += 1;
    p.tick_bytes += size;
    Ok(())
}

fn trigger_client_event(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let Some(super::Arg::Elements(to)) = a.fixed.first() else { return Err("no recipients".into()) };
    let name = a.str(1).ok_or("no name")?.to_string();
    let source = a.element(2).ok_or("no source")?;
    let values = args_of(&a, &ValueLimits::REMOTE)?;
    let args = encode_list(&values, &ValueLimits::REMOTE).map_err(|e| e.to_string())?;
    let host = ctx.host()?;
    let mut w = host.lock();
    if w.tree.get(source).is_none() {
        return Err(format!("element {source} does not exist").into());
    }
    let to = recipients(&w, to)?;
    if w.clients.is_empty() || to == Recipients::Some(Vec::new()) {
        return Ok(vec![Ret::Bool(true)]);
    }
    send(&mut w, ctx, &host.limits, Destination::Clients(to), Body::Event { name, source, args })?;
    Ok(vec![Ret::Bool(true)])
}

fn client_source(w: &World, source: ElementId) -> Result<(), ApiError> {
    match w.tree.get(source) {
        Some(e) if e.creator == Creator::Server || source == ROOT => Ok(()),
        Some(_) => Err(format!("element {source} is this client's own; the server only takes its elements").into()),
        None => Err(format!("element {source} does not exist").into()),
    }
}

fn trigger_server_event(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let name = a.str(0).ok_or("no name")?.to_string();
    let source = a.element(1).ok_or("no source")?;
    let values = args_of(&a, &ValueLimits::REMOTE)?;
    let args = encode_list(&values, &ValueLimits::REMOTE).map_err(|e| e.to_string())?;
    let host = ctx.host()?;
    let mut w = host.lock();
    client_source(&w, source)?;
    send(&mut w, ctx, &host.limits, Destination::Server, Body::Event { name, source, args })?;
    Ok(vec![Ret::Bool(true)])
}

fn start_latent(
    ctx: &Ctx,
    a: &Args,
    to: Destination,
    name: String,
    rate: i64,
    source: ElementId,
) -> Result<Vec<Ret>, ApiError> {
    let values = args_of(a, &ValueLimits::LATENT)?;
    let payload = encode_list(&values, &ValueLimits::LATENT).map_err(|e| e.to_string())?;
    let host = ctx.host()?;
    let limits = host.limits;
    let rate = u32::try_from(rate).unwrap_or(u32::MAX).clamp(limits.latent_min_rate, limits.latent_max_rate);
    let mut w = host.lock();
    let sending = w.latent_out.count(&ctx.package);
    if sending >= limits.package.latent {
        return Err(format!("the package sends {sending} latent events (at most {})", limits.package.latent).into());
    }
    let h = w.latent_out.start(&ctx.package, to, &name, source, payload, rate);
    Ok(vec![Ret::Latent(h)])
}

fn trigger_latent_client_event(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let Some(super::Arg::Elements(to)) = a.fixed.first() else { return Err("no recipients".into()) };
    let name = a.str(1).ok_or("no name")?.to_string();
    let rate = a.int(2).unwrap_or(0);
    let source = a.element(3).ok_or("no source")?;
    let to = {
        let host = ctx.host()?;
        let w = host.lock();
        if w.tree.get(source).is_none() {
            return Err(format!("element {source} does not exist").into());
        }
        recipients(&w, to)?
    };
    start_latent(ctx, &a, Destination::Clients(to), name, rate, source)
}

fn trigger_latent_server_event(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let name = a.str(0).ok_or("no name")?.to_string();
    let rate = a.int(1).unwrap_or(0);
    let source = a.element(2).ok_or("no source")?;
    {
        let host = ctx.host()?;
        client_source(&host.lock(), source)?;
    }
    start_latent(ctx, &a, Destination::Server, name, rate, source)
}

fn handle(a: &Args) -> Result<u64, ApiError> {
    match a.fixed.first() {
        Some(super::Arg::Latent(h)) => Ok(*h),
        _ => Err("no handle".into()),
    }
}

fn latent_status(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let h = handle(&a)?;
    let host = ctx.host()?;
    let w = host.lock();
    let Some((total, sent)) = w.latent_out.status(&ctx.package, h) else { return Ok(vec![Ret::Nil]) };
    let (total, sent) = (total as f64, sent as f64);
    let percent = if total > 0.0 { (sent / total * 100.0).clamp(0.0, 100.0) } else { 100.0 };
    let field = |k: &str, v: f64| (NetValue::String(k.as_bytes().to_vec()), NetValue::Number(v));
    let t = NetValue::Table(vec![field("total", total), field("sent", sent), field("percent", percent)]);
    Ok(vec![Ret::Value(t)])
}

fn cancel_latent(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let h = handle(&a)?;
    let host = ctx.host()?;
    let mut w = host.lock();
    let World { latent_out, outbox, .. } = &mut *w;
    Ok(vec![Ret::Bool(latent_out.cancel(&ctx.package, h, outbox))])
}

fn latent_handles(ctx: &Ctx, _: &Lua, _: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let w = host.lock();
    Ok(vec![Ret::Latents(w.latent_out.handles(&ctx.package))])
}
