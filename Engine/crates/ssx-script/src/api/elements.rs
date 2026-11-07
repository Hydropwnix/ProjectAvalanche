use mlua::Lua;

use super::{ApiError, Args, Ctx, NameKind, Param, Ret, Sides, Spec, Ty};
use crate::host::Side;
use crate::host::elements::{Creator, ROOT, Sync, Tree};
use crate::host::ops::{self, DataBy, DataChange};
use crate::host::outbox::{Body, Recipients};
use crate::host::world::World;
use crate::value::{ElementId, ValueLimits};

pub const MAX_RESULT_LIST: usize = 65_536;
const SYNC: &[&str] = &["local", "subscribers", "broadcast"];

const fn spec(
    name: &'static str,
    sides: Sides,
    params: &'static [Param],
    returns: &'static [Ty],
    call: super::Imp,
) -> Spec {
    Spec { name, sides, api: 1, params, rest: None, returns, any_results: false, call }
}

const fn p(name: &'static str, ty: Ty) -> Param {
    Param { name, ty }
}

const EL: Param = p("element", Ty::Element);
const KEY: Param = p("key", Ty::Name(NameKind::DataKey));
const B: &[Ty] = &[Ty::Bool];

pub static SPECS: &[Spec] = &[
    spec(
        "createElement",
        Sides::Both,
        &[p("type", Ty::Name(NameKind::NewElementKind)), p("id", Ty::Opt(&Ty::Name(NameKind::ElementName)))],
        &[Ty::Element],
        create_element,
    ),
    spec("destroyElement", Sides::Both, &[EL], B, destroy_element),
    spec("isElement", Sides::Both, &[p("value", Ty::Any)], B, is_element),
    spec("getElementType", Sides::Both, &[EL], &[Ty::Str { max: 64 }], element_type),
    spec("getElementID", Sides::Both, &[EL], &[Ty::Str { max: 128 }], element_id),
    spec("setElementID", Sides::Both, &[EL, p("id", Ty::Name(NameKind::ElementName))], B, set_element_id),
    spec(
        "getElementByID",
        Sides::Both,
        &[p("id", Ty::Name(NameKind::ElementName)), p("index", Ty::Opt(&Ty::Int { min: 0, max: i32::MAX as i64 }))],
        &[Ty::Opt(&Ty::Element)],
        element_by_id,
    ),
    spec("getElementParent", Sides::Both, &[EL], &[Ty::Opt(&Ty::Element)], element_parent),
    spec("setElementParent", Sides::Both, &[EL, p("parent", Ty::Element)], B, set_element_parent),
    spec(
        "getElementChildren",
        Sides::Both,
        &[EL, p("type", Ty::Opt(&Ty::Name(NameKind::ElementKind)))],
        &[Ty::List(&Ty::Element)],
        element_children,
    ),
    spec(
        "getElementsByType",
        Sides::Both,
        &[p("type", Ty::Name(NameKind::ElementKind)), p("startAt", Ty::Opt(&Ty::Element))],
        &[Ty::List(&Ty::Element)],
        elements_by_type,
    ),
    spec("getRootElement", Sides::Both, &[], &[Ty::Element], root_element),
    spec("getResourceRootElement", Sides::Both, &[], &[Ty::Element], resource_root),
    spec("setElementOwner", Sides::Server, &[EL, p("player", Ty::Opt(&Ty::Element))], B, set_owner),
    spec("getElementOwner", Sides::Server, &[EL], &[Ty::Opt(&Ty::Element)], get_owner),
    spec(
        "setElementData",
        Sides::Both,
        &[EL, KEY, p("value", Ty::Value), p("sync", Ty::Opt(&Ty::Enum(SYNC)))],
        B,
        set_element_data,
    ),
    spec("getElementData", Sides::Both, &[EL, KEY], &[Ty::Value], get_element_data),
    spec("hasElementData", Sides::Both, &[EL, KEY], B, has_element_data),
    spec("removeElementData", Sides::Both, &[EL, KEY], B, remove_element_data),
    spec("setElementDataClientWritable", Sides::Server, &[EL, KEY, p("writable", Ty::Bool)], B, set_writable),
    spec("addElementDataSubscriber", Sides::Server, &[EL, KEY, p("player", Ty::Element)], B, add_subscriber),
    spec("removeElementDataSubscriber", Sides::Server, &[EL, KEY, p("player", Ty::Element)], B, remove_subscriber),
];

fn el(a: &Args, i: usize) -> Result<ElementId, ApiError> {
    a.element(i).ok_or_else(|| "an element expected".into())
}

fn exists(w: &World, id: ElementId) -> Result<&crate::host::elements::Element, ApiError> {
    w.tree.get(id).ok_or_else(|| format!("element {id} does not exist (it was destroyed)").into())
}

fn own(w: &World, ctx: &Ctx, id: ElementId) -> Result<(), ApiError> {
    let e = exists(w, id)?;
    if e.creator != Creator::Package(ctx.package.clone()) {
        return Err(format!("element {id} is not this package's to change").into());
    }
    Ok(())
}

fn create_element(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let kind = a.str(0).ok_or("no type")?.to_string();
    let host = ctx.host()?;
    let limits = host.limits;
    let mut w = host.lock();
    let p = w.running(&ctx.package).ok_or("the package is not running")?;
    if p.elements >= limits.package.elements {
        return Err(format!("the package has {} elements (at most {})", p.elements, limits.package.elements).into());
    }
    let creator = Creator::Package(ctx.package.clone());
    let id = w.tree.create(&kind, ctx.root, creator, None, ctx.side == Side::Client, limits.tree_depth)?;
    if let Some(name) = a.str(1) {
        w.tree.set_name(id, name);
    }
    if let Some(p) = w.packages.get_mut(&ctx.package) {
        p.elements += 1;
    }
    let name = a.str(1).unwrap_or_default().to_string();
    let body = Body::Created { element: id, kind, name, parent: ctx.root };
    ops::replicate(&mut w, ctx.side, Some(&ctx.package), Recipients::All, body);
    Ok(vec![Ret::Element(id)])
}

fn destroy_element(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let id = el(&a, 0)?;
    let host = ctx.host()?;
    let mut w = host.lock();
    own(&w, ctx, id)?;
    ops::destroy(&mut w, ctx.side, id);
    Ok(vec![Ret::Bool(true)])
}

fn is_element(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let id = match a.value(0) {
        mlua::Value::UserData(ud) => super::lua::element_of(&ud),
        _ => None,
    };
    let host = ctx.host()?;
    let w = host.lock();
    Ok(vec![Ret::Bool(id.is_some_and(|id| w.tree.get(id).is_some()))])
}

fn element_type(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let w = host.lock();
    Ok(vec![Ret::Str(exists(&w, el(&a, 0)?)?.kind.clone())])
}

fn element_id(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let w = host.lock();
    Ok(vec![Ret::Str(exists(&w, el(&a, 0)?)?.name.clone())])
}

fn set_element_id(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let id = el(&a, 0)?;
    let name = a.str(1).ok_or("no id")?.to_string();
    let host = ctx.host()?;
    let mut w = host.lock();
    own(&w, ctx, id)?;
    w.tree.set_name(id, &name);
    ops::replicate(&mut w, ctx.side, Some(&ctx.package), Recipients::All, Body::Renamed { element: id, name });
    Ok(vec![Ret::Bool(true)])
}

fn element_by_id(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let name = a.str(0).ok_or("no id")?;
    let index = usize::try_from(a.int(1).unwrap_or(0)).map_err(|_| "a negative index")?;
    let host = ctx.host()?;
    let w = host.lock();
    Ok(vec![w.tree.by_name(name, index).map_or(Ret::Nil, Ret::Element)])
}

fn element_parent(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let w = host.lock();
    Ok(vec![exists(&w, el(&a, 0)?)?.parent.map_or(Ret::Nil, Ret::Element)])
}

fn set_element_parent(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let (id, parent) = (el(&a, 0)?, el(&a, 1)?);
    let host = ctx.host()?;
    let mut w = host.lock();
    own(&w, ctx, id)?;
    if parent != ctx.root {
        own(&w, ctx, parent).map_err(|_| "the new parent is neither this package's root nor one of its elements")?;
    }
    w.tree.set_parent(id, parent, host.limits.tree_depth)?;
    ops::replicate(&mut w, ctx.side, Some(&ctx.package), Recipients::All, Body::Moved { element: id, parent });
    Ok(vec![Ret::Bool(true)])
}

fn list(ids: impl Iterator<Item = ElementId>) -> Result<Vec<Ret>, ApiError> {
    let ids: Vec<ElementId> = ids.take(MAX_RESULT_LIST + 1).collect();
    if ids.len() > MAX_RESULT_LIST {
        return Err(format!("more than {MAX_RESULT_LIST} elements (the most a list holds)").into());
    }
    Ok(vec![Ret::Elements(ids)])
}

fn element_children(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let w = host.lock();
    let e = exists(&w, el(&a, 0)?)?;
    let kind = a.str(1);
    list(e.children.iter().copied().filter(|c| kind.is_none_or(|k| w.tree.get(*c).is_some_and(|ce| ce.kind == k))))
}

fn elements_by_type(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let kind = a.str(0).ok_or("no type")?;
    let host = ctx.host()?;
    let w = host.lock();
    let start = a.element(1).unwrap_or(ROOT);
    exists(&w, start)?;
    if start == ROOT {
        return list(w.tree.by_type(kind));
    }
    list(w.tree.subtree(start).into_iter().filter(|id| w.tree.get(*id).is_some_and(|e| e.kind == kind)))
}

fn root_element(_: &Ctx, _: &Lua, _: Args) -> Result<Vec<Ret>, ApiError> {
    Ok(vec![Ret::Element(ROOT)])
}

fn resource_root(ctx: &Ctx, _: &Lua, _: Args) -> Result<Vec<Ret>, ApiError> {
    Ok(vec![Ret::Element(ctx.root)])
}

fn player_client(w: &World, id: ElementId) -> Result<crate::host::elements::ClientId, ApiError> {
    w.client_of_player(id).ok_or_else(|| format!("element {id} is not a connected player").into())
}

fn set_owner(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let id = el(&a, 0)?;
    let host = ctx.host()?;
    let mut w = host.lock();
    own(&w, ctx, id)?;
    let owner = match a.element(1) {
        Some(player) => Some(player_client(&w, player)?),
        None => None,
    };
    if let Some(e) = w.tree.get_mut(id) {
        e.owner = owner;
    }
    Ok(vec![Ret::Bool(true)])
}

fn get_owner(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let w = host.lock();
    let owner = exists(&w, el(&a, 0)?)?.owner;
    Ok(vec![owner.and_then(|c| w.player_of(c)).map_or(Ret::Nil, Ret::Element)])
}

fn change(
    ctx: &Ctx,
    id: ElementId,
    key: &str,
    value: Option<crate::value::NetValue>,
    sync: Option<Sync>,
) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let applied =
        ops::set_data(&host, &DataBy::Package(ctx.package.clone()), DataChange { element: id, key, value, sync })?;
    Ok(vec![Ret::Bool(applied)])
}

fn set_element_data(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let (id, key) = (el(&a, 0)?, a.str(1).ok_or("no key")?);
    let value = match a.value(2) {
        mlua::Value::Nil => None,
        v => Some(super::copy(&v, &ValueLimits::REMOTE)?),
    };
    let sync = match a.str(3) {
        Some("local") => Some(Sync::Local),
        Some("subscribers") => Some(Sync::Subscribers),
        Some(_) => Some(Sync::Broadcast),
        None => None,
    };
    if ctx.side == Side::Client && sync.is_some_and(|s| s != Sync::Local) && Tree::is_local(id) {
        return Err("a client's own elements keep their data local".into());
    }
    change(ctx, id, key, value, sync)
}

fn get_element_data(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let w = host.lock();
    let e = exists(&w, el(&a, 0)?)?;
    Ok(vec![e.data.get(a.str(1).ok_or("no key")?).map_or(Ret::Nil, |d| Ret::Value(d.value.clone()))])
}

fn has_element_data(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let w = host.lock();
    let e = exists(&w, el(&a, 0)?)?;
    Ok(vec![Ret::Bool(e.data.contains_key(a.str(1).ok_or("no key")?))])
}

fn remove_element_data(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    change(ctx, el(&a, 0)?, a.str(1).ok_or("no key")?, None, None)
}

fn may_open(w: &World, ctx: &Ctx, id: ElementId) -> Result<(), ApiError> {
    if exists(w, id)?.creator == Creator::Session {
        return Ok(());
    }
    own(w, ctx, id)
}

fn set_writable(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let (id, key) = (el(&a, 0)?, a.str(1).ok_or("no key")?.to_string());
    let writable = a.bool(2).unwrap_or(false);
    let host = ctx.host()?;
    let cap = host.limits.writable_keys;
    let mut w = host.lock();
    may_open(&w, ctx, id)?;
    let e = w.tree.get_mut(id).ok_or("the element does not exist")?;
    if writable {
        if !e.writable.contains_key(&key) && e.writable.len() >= cap {
            return Err(format!("{cap} keys are open on this element (the most)").into());
        }
        e.writable.insert(key, ctx.package.clone());
    } else {
        e.writable.remove(&key);
    }
    Ok(vec![Ret::Bool(true)])
}

fn add_subscriber(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let (id, key) = (el(&a, 0)?, a.str(1).ok_or("no key")?.to_string());
    let host = ctx.host()?;
    let mut w = host.lock();
    exists(&w, id)?;
    let client = player_client(&w, el(&a, 2)?)?;
    let e = w.tree.get_mut(id).ok_or("the element does not exist")?;
    e.subscribers.entry(key.clone()).or_default().insert(client, ctx.package.clone());
    let current = e.data.get(&key).filter(|d| d.sync == Sync::Subscribers).map(|d| d.value.clone());
    if let Some(v) = current {
        let value = crate::value::encode(&v, &ValueLimits::REMOTE).ok();
        let body = Body::Data { element: id, key, value };
        ops::replicate(&mut w, Side::Server, Some(&ctx.package), Recipients::Some(vec![client]), body);
    }
    Ok(vec![Ret::Bool(true)])
}

fn remove_subscriber(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let (id, key) = (el(&a, 0)?, a.str(1).ok_or("no key")?);
    let host = ctx.host()?;
    let mut w = host.lock();
    exists(&w, id)?;
    let client = player_client(&w, el(&a, 2)?)?;
    let e = w.tree.get_mut(id).ok_or("the element does not exist")?;
    let gone = e.subscribers.get_mut(key).is_some_and(|s| s.remove(&client).is_some());
    Ok(vec![Ret::Bool(gone)])
}
