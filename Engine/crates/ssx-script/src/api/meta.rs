use std::sync::Arc;
use std::time::Duration;

use mlua::{Function, Lua, MultiValue, Table, Value};

use super::{ApiError, Args, Ctx, NameKind, Param, Ret, Sides, Spec, Ty};
use crate::host::call::{Errors, Outcome, enter};
use crate::host::events::{Callback, CommandRec};
use crate::host::world::TimerRec;
use crate::value::{ValueLimits, from_lua_multi, json, to_lua};

const fn spec(
    name: &'static str,
    params: &'static [Param],
    rest: Option<Ty>,
    returns: &'static [Ty],
    call: super::Imp,
) -> Spec {
    Spec { name, sides: Sides::Both, api: 1, params, rest, returns, any_results: false, call }
}

const fn p(name: &'static str, ty: Ty) -> Param {
    Param { name, ty }
}

const MS_DAY: f64 = 86_400_000.0;

pub static SPECS: &[Spec] = &[
    spec("getResourceName", &[], None, &[Ty::Str { max: 64 }], get_resource_name),
    spec(
        "setTimer",
        &[
            p("callback", Ty::Function),
            p("interval", Ty::Number { min: 0.0, max: MS_DAY }),
            p("times", Ty::Int { min: 0, max: i32::MAX as i64 }),
        ],
        Some(Ty::Any),
        &[Ty::Timer],
        set_timer,
    ),
    spec("killTimer", &[p("timer", Ty::Timer)], None, &[Ty::Bool], kill_timer),
    spec("isTimer", &[p("value", Ty::Any)], None, &[Ty::Bool], is_timer),
    spec(
        "addCommandHandler",
        &[p("name", Ty::Name(NameKind::Command)), p("handler", Ty::Function)],
        None,
        &[Ty::Bool],
        add_command,
    ),
    spec(
        "removeCommandHandler",
        &[p("name", Ty::Name(NameKind::Command)), p("handler", Ty::Opt(&Ty::Function))],
        None,
        &[Ty::Bool],
        remove_command,
    ),
    spec("get", &[p("name", Ty::Name(NameKind::Setting))], None, &[Ty::Value], get_setting),
    spec("json.encode", &[p("value", Ty::Value)], None, &[Ty::Str { max: json::MAX_JSON }], json_encode),
    spec("json.decode", &[p("text", Ty::Str { max: json::MAX_JSON })], None, &[Ty::Value], json_decode),
    spec("storage.get", &[p("key", Ty::Name(NameKind::StorageKey))], None, &[Ty::Value], storage_get),
    spec("storage.set", &[p("key", Ty::Name(NameKind::StorageKey)), p("value", Ty::Value)], None, &[], storage_set),
];

pub(crate) fn callback(lua: &Lua, f: &Function) -> mlua::Result<Callback> {
    let info = f.info();
    let at = format!(
        "{}:{}",
        crate::env::cap(info.short_src.as_deref().unwrap_or("?"), 96),
        info.line_defined.map_or_else(|| "?".to_string(), |l| l.to_string())
    );
    let key = lua.create_registry_value(f.clone())?;
    Ok(Callback { key: Arc::new(key), ptr: f.to_pointer().addr(), at })
}

fn get_resource_name(ctx: &Ctx, _: &Lua, _: Args) -> Result<Vec<Ret>, ApiError> {
    Ok(vec![Ret::Str(ctx.package.clone())])
}

fn set_timer(ctx: &Ctx, lua: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let limits = host.limits;
    let ms = a.num(1).unwrap_or(0.0);
    let interval = Duration::from_secs_f64(ms / 1000.0);
    if interval < limits.min_timer || interval > limits.max_timer {
        return Err(format!(
            "an interval of {ms} ms (from {} to {} ms)",
            limits.min_timer.as_millis(),
            limits.max_timer.as_millis()
        )
        .into());
    }
    if a.rest.len() > limits.timer_args {
        return Err(format!("{} arguments for the timer (at most {})", a.rest.len(), limits.timer_args).into());
    }
    let times = u32::try_from(a.int(2).unwrap_or(0)).map_err(|_| "a count of times beyond 2^31")?;
    {
        let w = host.lock();
        let p = w.running(&ctx.package).ok_or("the package is not running")?;
        if p.timers >= limits.package.timers {
            return Err(format!("the package has {} timers (at most {})", p.timers, limits.package.timers).into());
        }
    }
    let f = a.func(0).ok_or("no callback")?;
    let cb = callback(lua, f)?;
    let args = lua.create_sequence_from(a.rest)?;
    let args = Arc::new(lua.create_registry_value(args)?);
    let mut w = host.lock();
    w.timers.next += 1;
    let id = w.timers.next;
    let due = w.now.saturating_add(interval);
    let rec = TimerRec { id, package: ctx.package.clone(), callback: cb, args, interval, left: times };
    w.timers.due.insert((due, id), rec);
    w.timers.at.insert(id, (due, ctx.package.clone()));
    let p = w.packages.get_mut(&ctx.package).ok_or("the package is not running")?;
    p.timers += 1;
    Ok(vec![Ret::Timer(id)])
}

fn kill_timer(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let Some(super::Arg::Timer(id)) = a.fixed.first() else { return Err("no timer".into()) };
    let host = ctx.host()?;
    let mut w = host.lock();
    let Some((due, owner)) = w.timers.at.get(id).cloned() else { return Ok(vec![Ret::Bool(false)]) };
    if owner != ctx.package {
        return Ok(vec![Ret::Bool(false)]);
    }
    w.timers.at.remove(id);
    w.timers.due.remove(&(due, *id));
    if let Some(p) = w.packages.get_mut(&ctx.package) {
        p.timers = p.timers.saturating_sub(1);
    }
    Ok(vec![Ret::Bool(true)])
}

fn is_timer(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let id = match a.value(0) {
        Value::UserData(ud) => ud.borrow::<super::lua::TimerRef>().ok().map(|t| t.0),
        _ => None,
    };
    let host = ctx.host()?;
    let w = host.lock();
    let live = id.is_some_and(|id| w.timers.at.get(&id).is_some_and(|(_, p)| *p == ctx.package));
    Ok(vec![Ret::Bool(live)])
}

fn add_command(ctx: &Ctx, lua: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let name = a.str(0).ok_or("no name")?.to_string();
    if ctx.side == crate::host::Side::Server {
        super::check_server_command_name(&name)?;
    }
    let f = a.func(1).ok_or("no handler")?;
    let host = ctx.host()?;
    let ptr = f.to_pointer().addr();
    {
        let w = host.lock();
        if w.events.command(&name).iter().any(|c| c.package == ctx.package && c.callback.ptr == ptr) {
            return Ok(vec![Ret::Bool(false)]);
        }
        let cap = host.limits.package.commands;
        if w.events.commands_of(&ctx.package) >= cap {
            return Err(format!("the package has {cap} command handlers (the most)").into());
        }
    }
    let cb = callback(lua, f)?;
    host.lock().events.add_command(&name, CommandRec { package: ctx.package.clone(), callback: cb });
    Ok(vec![Ret::Bool(true)])
}

fn remove_command(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let name = a.str(0).ok_or("no name")?;
    let ptr = a.func(1).map(|f| f.to_pointer().addr());
    let host = ctx.host()?;
    let gone = host.lock().events.remove_command(name, &ctx.package, ptr);
    Ok(vec![Ret::Bool(gone > 0)])
}

fn get_setting(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let name = a.str(0).ok_or("no name")?;
    let host = ctx.host()?;
    let w = host.lock();
    let p = w.running(&ctx.package).ok_or("the package is not running")?;
    Ok(vec![p.settings.field(name).cloned().map_or(Ret::Nil, Ret::Value)])
}

fn json_encode(_: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let v = super::copy(&a.value(0), &ValueLimits::JSON)?;
    Ok(vec![Ret::Str(json::encode(&v).map_err(|e| e.0)?)])
}

fn json_decode(_: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let text = a.str(0).ok_or("no text")?;
    Ok(vec![Ret::Value(json::decode(text.as_bytes()).map_err(|e| e.0)?)])
}

fn storage_get(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let key = a.str(0).ok_or("no key")?;
    let host = ctx.host()?;
    let w = host.lock();
    let p = w.running(&ctx.package).ok_or("the package is not running")?;
    let store = p.store.as_ref().ok_or("this side has no storage")?;
    if let Some(why) = &store.unavailable {
        return Err(format!("this package has no storage: {why}").into());
    }
    Ok(vec![store.get(key).map_or(Ret::Nil, Ret::Value)])
}

fn storage_set(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let key = a.str(0).ok_or("no key")?;
    let value = match a.value(1) {
        Value::Nil => None,
        v => Some(super::copy(&v, &ValueLimits::STORAGE)?),
    };
    let host = ctx.host()?;
    let caps = host.limits.storage;
    let mut w = host.lock();
    let p = w.packages.get_mut(&ctx.package).ok_or("the package is not running")?;
    let store = p.store.as_mut().ok_or("this side has no storage")?;
    store.set(key, value.as_ref(), &caps)?;
    Ok(vec![])
}

pub fn exports_table(lua: &Lua, ctx: &Ctx) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    let meta = lua.create_table()?;
    let reg = ctx.clone();
    meta.raw_set(
        "__call",
        lua.create_function(move |lua, (_, name, f): (Value, Value, Value)| match register(&reg, lua, name, f) {
            Ok(()) => Ok(()),
            Err(ApiError::Refused(e)) => Err(mlua::Error::runtime(format!("exports: {e}"))),
            Err(ApiError::Lua(e)) => Err(e),
        })?,
    )?;
    let idx = ctx.clone();
    meta.raw_set(
        "__index",
        lua.create_function(move |lua, (_, pkg): (Value, Value)| {
            let pkg = match super::check_one(&Ty::Name(NameKind::Package), pkg) {
                Ok(super::Arg::Str(s)) => s,
                Ok(_) => return Err(mlua::Error::runtime("exports: a package name expected")),
                Err(e) => return Err(mlua::Error::runtime(format!("exports: {e}"))),
            };
            proxy(lua, &idx, pkg)
        })?,
    )?;
    meta.raw_set("__metatable", "locked")?;
    meta.set_readonly(true);
    t.set_metatable(Some(meta))?;
    t.set_readonly(true);
    Ok(t)
}

fn register(ctx: &Ctx, lua: &Lua, name: Value, f: Value) -> Result<(), ApiError> {
    let super::Arg::Str(name) = super::check_one(&Ty::Name(NameKind::Export), name)? else {
        return Err("an export name expected".into());
    };
    let super::Arg::Func(f) = super::check_one(&Ty::Function, f)? else { return Err("a function expected".into()) };
    let host = ctx.host()?;
    let cap = host.limits.package.exports;
    {
        let w = host.lock();
        let p = w.packages.get(&ctx.package).ok_or("the package is not running")?;
        if !p.exports.contains_key(&name) && p.exports.len() >= cap {
            return Err(format!("the package has {cap} exports (the most)").into());
        }
    }
    let cb = callback(lua, &f)?;
    let mut w = host.lock();
    let p = w.packages.get_mut(&ctx.package).ok_or("the package is not running")?;
    p.exports.insert(name, cb);
    Ok(())
}

fn proxy(lua: &Lua, ctx: &Ctx, target: String) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    let me = t.to_pointer().addr();
    let meta = lua.create_table()?;
    let ctx = ctx.clone();
    meta.raw_set(
        "__index",
        lua.create_function(move |lua, (_, fname): (Value, Value)| {
            let fname = match super::check_one(&Ty::Name(NameKind::Export), fname) {
                Ok(super::Arg::Str(s)) => s,
                _ => return Err(mlua::Error::runtime("exports: an export name expected")),
            };
            let (ctx, target) = (ctx.clone(), target.clone());
            lua.create_function(move |lua, args: MultiValue| {
                let mut args: Vec<Value> = args.into_iter().collect();
                let first = if args.is_empty() { Value::Nil } else { args.remove(0) };
                if !matches!(&first, Value::Table(t) if t.to_pointer().addr() == me) {
                    return Err(mlua::Error::runtime(format!("exports.{target}:{fname}: call it with ':'")));
                }
                match call_export(&ctx, lua, &target, &fname, &args) {
                    Ok(v) => Ok(v),
                    Err(ApiError::Refused(e)) => Err(mlua::Error::runtime(e)),
                    Err(ApiError::Lua(e)) => Err(e),
                }
            })
        })?,
    )?;
    meta.raw_set("__metatable", "locked")?;
    meta.set_readonly(true);
    t.set_metatable(Some(meta))?;
    t.set_readonly(true);
    Ok(t)
}

fn call_export(ctx: &Ctx, lua: &Lua, target: &str, fname: &str, args: &[Value]) -> Result<MultiValue, ApiError> {
    let host = ctx.host()?;
    let values = from_lua_multi(args, &ValueLimits::LOCAL).map_err(|e| format!("the arguments: {e}"))?;
    let cb = {
        let mut w = host.lock();
        let max = host.limits.call_depth;
        if w.export_depth >= max {
            return Err(format!("export calls nested deeper than {max}").into());
        }
        let p = w.running(target).ok_or_else(|| format!("{target} is not running"))?;
        let cb = p.exports.get(fname).cloned().ok_or_else(|| format!("{target} exports no {fname}"))?;
        w.export_depth += 1;
        cb
    };
    let during = format!("exports.{target}:{fname} ({})", cb.at);
    let out = enter(&host, target, &during, Errors::Return, |vm| {
        let (f, argv) = vm.guarded(|| {
            let l = vm.lua();
            let f: Function = l.registry_value(&cb.key)?;
            let argv = values
                .iter()
                .map(|v| to_lua(l, v, &super::lua::element_value))
                .collect::<mlua::Result<MultiValue>>()?;
            Ok((f, argv))
        })?;
        let results: MultiValue = vm.run(&f, argv)?;
        let results: Vec<Value> = results.into_iter().collect();
        Ok(from_lua_multi(&results, &ValueLimits::LOCAL))
    });
    host.lock().export_depth -= 1;
    let values = match out {
        Outcome::Done(Ok(v)) => v,
        Outcome::Done(Err(e)) => return Err(format!("exports.{target}:{fname}: its results: {e}").into()),
        Outcome::Failed(e) => return Err(format!("exports.{target}:{fname}: {e}").into()),
        Outcome::Stopped => return Err(format!("exports.{target}:{fname}: {target} is stopped").into()),
    };
    Ok(values.iter().map(|v| to_lua(lua, v, &super::lua::element_value)).collect::<mlua::Result<MultiValue>>()?)
}
