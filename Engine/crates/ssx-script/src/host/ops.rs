use std::sync::Arc;

use super::Side;
use super::call::{Dispatch, dispatch};
use super::elements::{ClientId, Creator, Datum, ROOT, Sync, Tree, Writer};
use super::outbox::{Body, Destination, Message, Recipients};
use super::world::{HostShared, World};
use crate::value::{ElementId, NetValue, ValueLimits, encode};

pub fn data_event(side: Side) -> &'static str {
    match side {
        Side::Server => "onElementDataChange",
        Side::Client => "onClientElementDataChange",
    }
}

pub fn replicate(w: &mut World, side: Side, package: Option<&str>, to: Recipients, body: Body) {
    if side != Side::Server || w.clients.is_empty() {
        return;
    }
    if let Recipients::Some(list) = &to
        && list.is_empty()
    {
        return;
    }
    let m = Message { package: package.map(str::to_string), to: Destination::Clients(to), body };
    w.outbox.push_replication(m);
}

fn creator_package(c: &Creator) -> Option<&str> {
    match c {
        Creator::Package(p) => Some(p),
        _ => None,
    }
}

fn count(w: &mut World, d: &Datum, arriving: bool) {
    let apply = |n: &mut usize, by: usize| {
        if arriving { *n += by } else { *n = n.saturating_sub(by) }
    };
    match &d.writer {
        Writer::Package(p) => {
            if let Some(p) = w.packages.get_mut(p) {
                apply(&mut p.data_keys, 1);
                apply(&mut p.data_bytes, d.bytes);
            }
        }
        Writer::Client(c) => {
            let e = w.client_data.entry(*c).or_default();
            apply(&mut e.0, 1);
            apply(&mut e.1, d.bytes);
        }
        Writer::Server => {
            apply(&mut w.mirror.1, 1);
            apply(&mut w.mirror.2, d.bytes);
        }
    }
}

pub fn destroy(w: &mut World, side: Side, id: ElementId) -> usize {
    let package = w.tree.get(id).and_then(|e| creator_package(&e.creator).map(str::to_string));
    let replicated = w.tree.get(id).is_some_and(|e| !Tree::is_local(id) && e.creator != Creator::Server);
    let removed = w.tree.destroy(id);
    let ids: Vec<ElementId> = removed.iter().map(|(id, _)| *id).collect();
    for (_, e) in &removed {
        if e.creator == Creator::Server {
            w.mirror.0 = w.mirror.0.saturating_sub(1);
        }
        if let Some(p) = creator_package(&e.creator)
            && let Some(p) = w.packages.get_mut(p)
        {
            p.elements = p.elements.saturating_sub(1);
        }
        for d in e.data.values() {
            count(w, d, false);
        }
    }
    w.events.remove_on_elements(&ids);
    w.game.markers.forget(&ids);
    w.server.forget(&ids);
    w.game.forget_session(&ids);
    if w.local_player.is_some_and(|p| ids.contains(&p)) {
        w.local_player = None;
    }
    if replicated && !removed.is_empty() {
        replicate(w, side, package.as_deref(), Recipients::All, Body::Destroyed { element: id });
    }
    removed.len()
}

pub enum DataBy {
    Package(String),
    Client(ClientId),
    Server,
}

pub struct DataChange<'a> {
    pub element: ElementId,
    pub key: &'a str,
    pub value: Option<NetValue>,
    pub sync: Option<Sync>,
}

fn recipients(e: &super::elements::Element, key: &str, sync: Sync) -> Option<Recipients> {
    match sync {
        Sync::Local => None,
        Sync::Broadcast => Some(Recipients::All),
        Sync::Subscribers => {
            Some(Recipients::Some(e.subscribers.get(key).into_iter().flatten().map(|(c, _)| *c).collect()))
        }
    }
}

pub fn set_data(shared: &Arc<HostShared>, by: &DataBy, c: DataChange) -> Result<bool, String> {
    let side = shared.side;
    let limits = &shared.limits;
    let encoded = match &c.value {
        Some(v) => Some(encode(v, &ValueLimits::REMOTE).map_err(|e| format!("the value: {e}"))?),
        None => None,
    };
    let bytes = c.key.len() + encoded.as_ref().map_or(0, Vec::len);
    let (old, mirror, player_of_writer) = {
        let mut w = shared.lock();
        let e = w.tree.get(c.element).ok_or("the element does not exist")?;
        let mirror = e.creator == Creator::Server;
        if let DataBy::Client(cl) = by {
            let own = w.player_of(*cl) == Some(c.element) || e.owner == Some(*cl);
            if !own || !e.writable.contains_key(c.key) {
                return Err(format!("key {:?} is not open to this client on this element", c.key));
            }
        }
        let old = e.data.get(c.key).cloned();
        let sync = match (side, by) {
            (Side::Client, _) => Sync::Local,
            (Side::Server, DataBy::Package(_)) => c.sync.unwrap_or(Sync::Broadcast),
            (Side::Server, _) => old.as_ref().map_or(Sync::Broadcast, |d| d.sync),
        };
        let writer = match by {
            DataBy::Package(p) => Writer::Package(p.clone()),
            DataBy::Client(cl) => Writer::Client(*cl),
            DataBy::Server => Writer::Server,
        };
        let old_bytes = old.as_ref().filter(|d| d.writer == writer).map_or(0, |d| d.bytes);
        let new_key = old.as_ref().is_none_or(|d| d.writer != writer);
        if c.value.is_some() {
            match &writer {
                Writer::Package(p) => {
                    let pk = w.packages.get(p).ok_or("the package is not running")?;
                    let caps = &limits.package;
                    if new_key && pk.data_keys >= caps.data_keys {
                        return Err(format!(
                            "the package holds {} data keys (at most {})",
                            pk.data_keys, caps.data_keys
                        ));
                    }
                    let after = pk.data_bytes.saturating_sub(old_bytes) + bytes;
                    if after > caps.data_bytes {
                        return Err(format!("the package's data would be {after} bytes (at most {})", caps.data_bytes));
                    }
                }
                Writer::Client(cl) => {
                    let (keys, total) = w.client_data.get(cl).copied().unwrap_or_default();
                    if (new_key && keys >= limits.client_data_keys)
                        || total.saturating_sub(old_bytes) + bytes > limits.client_data_bytes
                    {
                        return Err("the client's data is at its cap".into());
                    }
                }
                Writer::Server => {
                    let (_, keys, total) = w.mirror;
                    if (new_key && keys >= limits.mirror_data_keys)
                        || total.saturating_sub(old_bytes) + bytes > limits.mirror_data_bytes
                    {
                        return Err("the server's data on this client is at its cap".into());
                    }
                }
            }
        }
        if let Some(o) = &old {
            count(&mut w, o, false);
        }
        let e = w.tree.get_mut(c.element).ok_or("the element does not exist")?;
        match &c.value {
            Some(v) => {
                let d = Datum { value: v.clone(), bytes, sync, writer };
                e.data.insert(c.key.to_string(), d.clone());
                count(&mut w, &d, true);
            }
            None => {
                e.data.remove(c.key);
            }
        }
        let player = match by {
            DataBy::Client(cl) => w.player_of(*cl),
            _ => None,
        };
        (old, mirror, player)
    };
    let args = [
        NetValue::String(c.key.as_bytes().to_vec()),
        old.as_ref().map_or(NetValue::Nil, |d| d.value.clone()),
        c.value.clone().unwrap_or(NetValue::Nil),
    ];
    let d = Dispatch { name: data_event(side), source: c.element, args: &args, only: None, client: player_of_writer };
    let cancelled = dispatch(shared, &d)? && !matches!(by, DataBy::Server);
    let mut w = shared.lock();
    if cancelled {
        if let Some(e) = w.tree.get_mut(c.element) {
            let now = match &old {
                Some(o) => e.data.insert(c.key.to_string(), o.clone()),
                None => e.data.remove(c.key),
            };
            if let Some(n) = now {
                count(&mut w, &n, false);
            }
            if let Some(o) = &old {
                count(&mut w, o, true);
            }
        }
        return Ok(false);
    }
    let Some(e) = w.tree.get(c.element) else { return Ok(true) };
    let sync_now = e.data.get(c.key).map(|d| d.sync);
    let package = match by {
        DataBy::Package(p) => Some(p.clone()),
        _ => None,
    };
    if side == Side::Server {
        let was = old.as_ref().and_then(|o| recipients(e, c.key, o.sync));
        let to = sync_now.and_then(|s| recipients(e, c.key, s));
        let body = |value| Body::Data { element: c.element, key: c.key.to_string(), value };
        match (to, &encoded) {
            (Some(to), Some(v)) => replicate(&mut w, side, package.as_deref(), to, body(Some(v.clone()))),
            (None, _) | (_, None) => {
                if let Some(was) = was {
                    replicate(&mut w, side, package.as_deref(), was, body(None));
                }
            }
        }
    } else if mirror && let DataBy::Package(p) = by {
        let body = Body::DataWrite { element: c.element, key: c.key.to_string(), value: encoded };
        w.outbox.push(Message { package: Some(p.clone()), to: Destination::Server, body })?;
    }
    Ok(true)
}

pub fn send_back(w: &mut World, client: ClientId, element: ElementId, key: &str) {
    let value = w
        .tree
        .get(element)
        .and_then(|e| e.data.get(key))
        .filter(|d| d.sync != Sync::Local)
        .and_then(|d| encode(&d.value, &ValueLimits::REMOTE).ok());
    let body = Body::Data { element, key: key.to_string(), value };
    replicate(w, Side::Server, None, Recipients::Some(vec![client]), body);
}

pub fn snapshot(w: &World, client: ClientId) -> Vec<Message> {
    let to = || Destination::Clients(Recipients::Some(vec![client]));
    let mut out = vec![Message { package: None, to: to(), body: Body::Reset }];
    for id in w.tree.subtree(ROOT) {
        let Some(e) = w.tree.get(id) else { continue };
        let package = creator_package(&e.creator).map(str::to_string);
        if id != ROOT {
            let parent = e.parent.unwrap_or(ROOT);
            let body = Body::Created { element: id, kind: e.kind.clone(), name: e.name.clone(), parent };
            out.push(Message { package: package.clone(), to: to(), body });
            if let Some((c, r)) = w.server.riders.iter().find(|(_, r)| r.element == id)
                && let Some(player) = w.player_of(*c)
            {
                let body = Body::Rider { element: id, player, number: r.number };
                out.push(Message { package: None, to: to(), body });
            }
            if let Some(spec) = w.server.markers.get(&id).and_then(|m| m.spec()) {
                out.push(Message { package: package.clone(), to: to(), body: Body::Marker { element: id, spec } });
            }
        }
        for (key, d) in &e.data {
            let send = match d.sync {
                Sync::Local => false,
                Sync::Broadcast => true,
                Sync::Subscribers => e.subscribers.get(key).is_some_and(|s| s.contains_key(&client)),
            };
            if send && let Ok(v) = encode(&d.value, &ValueLimits::REMOTE) {
                let body = Body::Data { element: id, key: key.clone(), value: Some(v) };
                out.push(Message { package: package.clone(), to: to(), body });
            }
        }
    }
    out
}
