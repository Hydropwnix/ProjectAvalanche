use super::elements::{ClientId, Creator, Tree};
use super::latent::{Partial, ReceiveCaps};
use super::ops::{self, DataBy, DataChange};
use super::outbox::{Body, Peer};
use super::world::{Queued, World};
use super::{ScriptHost, Side};
use crate::value::{ElementId, NetValue, ValueLimits, decode, decode_list};

struct Invalid {
    name: String,
    package: String,
    added: bool,
    remote: bool,
}

struct Incoming<'a> {
    package: &'a str,
    name: &'a str,
    source: ElementId,
    args: Vec<NetValue>,
    bytes: usize,
}

impl ScriptHost {
    pub fn deliver(&self, from: Peer, package: Option<&str>, body: Body) -> Result<(), String> {
        let mut invalid = None;
        let r = self.deliver_inner(from, package, body, &mut invalid);
        if r.is_err() {
            self.shared.lock().refused_inbound += 1;
        }
        if let (Some(i), Peer::Client(client), Side::Server) = (invalid, from, self.shared.side) {
            let player = self.shared.lock().player_of(client);
            if let Some(player) = player {
                let args = [
                    NetValue::String(i.name.into_bytes()),
                    NetValue::Bool(i.added),
                    NetValue::Bool(i.remote),
                    NetValue::String(i.package.into_bytes()),
                ];
                self.raise_engine("onPlayerTriggerInvalidEvent", player, &args);
            }
        }
        self.settle();
        r
    }

    fn deliver_inner(
        &self,
        from: Peer,
        package: Option<&str>,
        body: Body,
        invalid: &mut Option<Invalid>,
    ) -> Result<(), String> {
        let side = self.shared.side;
        match (side, from) {
            (Side::Server, Peer::Client(_)) | (Side::Client, Peer::Server) => {}
            _ => return Err("a message from the wrong side".into()),
        }
        let replication = body.is_replication();
        let to_client = matches!(body, Body::Notice { .. } | Body::Sound { .. });
        if (side == Side::Server && (replication || to_client))
            || (side == Side::Client && matches!(body, Body::DataWrite { .. } | Body::Started))
        {
            return Err("a message this side does not take".into());
        }
        let limits = self.shared.limits;
        match body {
            Body::Event { name, source, args } => {
                let package = package.ok_or("an event without its package")?;
                let values = decode_list(&args, &ValueLimits::REMOTE).map_err(|e| format!("its arguments: {e}"))?;
                let ev = Incoming { package, name: &name, source, args: values, bytes: args.len() };
                self.queue_event(from, ev, invalid)
            }
            Body::LatentStart { transfer, name, source, total } => {
                let package = package.ok_or("a latent event without its package")?;
                let total = usize::try_from(total).map_err(|_| "a size beyond this system's")?;
                let p = Partial { package: package.to_string(), name, source, total, bytes: Vec::new() };
                let caps = ReceiveCaps {
                    total: limits.latent_bytes,
                    transfers: limits.latent_peer_transfers,
                    buffered: limits.latent_peer_bytes,
                };
                self.shared.lock().latent_in.start(from, caps, transfer, p)
            }
            Body::LatentChunk { transfer, offset, bytes } => {
                let done = self.shared.lock().latent_in.chunk(from, transfer, offset, &bytes)?;
                let Some(p) = done else { return Ok(()) };
                let values = decode_list(&p.bytes, &ValueLimits::LATENT).map_err(|e| format!("its arguments: {e}"))?;
                let ev =
                    Incoming { package: &p.package, name: &p.name, source: p.source, args: values, bytes: p.total };
                self.queue_event(from, ev, invalid)
            }
            Body::LatentCancel { transfer } => {
                self.shared.lock().latent_in.cancel(from, transfer);
                Ok(())
            }
            Body::DataWrite { element, key, value } => {
                let Peer::Client(client) = from else { return Err("data writes come from clients".into()) };
                self.client_write(client, element, &key, value)
            }
            Body::Reset => {
                let mut w = self.shared.lock();
                let keep = w.local_player;
                let tops: Vec<ElementId> = w
                    .tree
                    .iter()
                    .filter(|(_, e)| {
                        e.creator == Creator::Server
                            && !e.parent.is_some_and(|p| w.tree.get(p).is_some_and(|pe| pe.creator == Creator::Server))
                    })
                    .map(|(id, _)| id)
                    .collect();
                for id in tops {
                    ops::destroy(&mut w, Side::Client, id);
                }
                w.local_player = keep;
                Ok(())
            }
            Body::Created { element, kind, name, parent } => {
                crate::api::check_element_kind(&kind)?;
                crate::api::check_element_name(&name)?;
                let mut w = self.shared.lock();
                if w.mirror.0 >= limits.mirror_elements {
                    return Err(format!("{} elements mirrored (at most {})", w.mirror.0, limits.mirror_elements));
                }
                w.tree.insert_mirror(element, &kind, &name, parent, limits.tree_depth)?;
                w.mirror.0 += 1;
                Ok(())
            }
            Body::Destroyed { element } => self.mirror_only(element, |w| {
                ops::destroy(w, Side::Client, element);
            }),
            Body::Moved { element, parent } => {
                let mut w = self.shared.lock();
                mirror(&w, element)?;
                if Tree::is_local(parent) {
                    return Err("a mirror cannot move below a local element".into());
                }
                w.tree.set_parent(element, parent, limits.tree_depth)
            }
            Body::Renamed { element, name } => {
                crate::api::check_element_name(&name)?;
                self.mirror_only(element, |w| w.tree.set_name(element, &name))
            }
            Body::Data { element, key, value } => {
                crate::api::check_data_key(&key)?;
                {
                    let w = self.shared.lock();
                    if Tree::is_local(element) || w.tree.get(element).is_none() {
                        return Err(format!("data for {element}, which this client does not mirror"));
                    }
                }
                let value = match value {
                    Some(v) => Some(decode(&v, &ValueLimits::REMOTE).map_err(|e| format!("the value: {e}"))?),
                    None => None,
                };
                let change = DataChange { element, key: &key, value, sync: None };
                ops::set_data(&self.shared, &DataBy::Server, change).map(|_| ())
            }
            Body::Hello { player } => self.set_local_player(player),
            Body::Rider { element, player, number } => {
                let mut w = self.shared.lock();
                mirror_of(&w, element, "rider")?;
                mirror_of(&w, player, "player")?;
                w.game.session.retain(|_, (r, _)| *r != element);
                w.game.session.insert(number, (element, player));
                Ok(())
            }
            Body::Marker { element, spec } => self.mirror_marker(package, element, spec),
            Body::Notice { title, text, seconds } => self.server_notice(package, title, text, seconds),
            Body::Sound { id, position } => self.server_sound(package, &id, position),
            Body::Started => {
                let Peer::Client(client) = from else { return Err("acknowledgements come from clients".into()) };
                self.client_started(client, package)
            }
        }
    }

    fn mirror_only(&self, element: ElementId, f: impl FnOnce(&mut World)) -> Result<(), String> {
        let mut w = self.shared.lock();
        mirror(&w, element)?;
        f(&mut w);
        Ok(())
    }

    fn queue_event(&self, from: Peer, ev: Incoming<'_>, invalid: &mut Option<Invalid>) -> Result<(), String> {
        let Incoming { package, name, source, args, bytes } = ev;
        crate::api::check_event_name(name)?;
        let caps = self.shared.limits.package;
        let mut w = self.shared.lock();
        let mut refuse = |added: bool, why: String| {
            *invalid = Some(Invalid { name: name.to_string(), package: package.to_string(), added, remote: false });
            Err(why)
        };
        if w.running(package).is_none() {
            return refuse(false, format!("{package} does not run here"));
        }
        if !w.events.is_remote_for(name, package) {
            let added = w.events.declared_by(name, package);
            return refuse(added, format!("{package} did not declare {name} remote"));
        }
        if !w.may_source(from, source) {
            return Err(format!("{source} is not the sender's to name as a source"));
        }
        let client = match from {
            Peer::Client(c) => w.player_of(c),
            Peer::Server => None,
        };
        let p = w.packages.get_mut(package).ok_or("the package is gone")?;
        if p.queue.len() >= caps.queue_entries || p.queue_bytes + bytes > caps.queue_bytes {
            p.refusals += 1;
            return Err(format!("{package}'s queue is full ({} events, {} bytes)", p.queue.len(), p.queue_bytes));
        }
        p.queue_bytes += bytes;
        p.queue.push_back(Queued { name: name.to_string(), source, args, client, bytes });
        Ok(())
    }

    fn client_write(
        &self,
        client: ClientId,
        element: ElementId,
        key: &str,
        value: Option<Vec<u8>>,
    ) -> Result<(), String> {
        crate::api::check_data_key(key)?;
        let decoded = value.map(|v| decode(&v, &ValueLimits::REMOTE).map_err(|e| format!("the value: {e}")));
        let r = match decoded.transpose() {
            Ok(value) => {
                let change = DataChange { element, key, value, sync: None };
                ops::set_data(&self.shared, &DataBy::Client(client), change)
            }
            Err(e) => Err(e),
        };
        match r {
            Ok(true) => Ok(()),
            Ok(false) => {
                ops::send_back(&mut self.shared.lock(), client, element, key);
                Err(format!("a handler cancelled the write of {key:?}"))
            }
            Err(e) => {
                ops::send_back(&mut self.shared.lock(), client, element, key);
                Err(e)
            }
        }
    }
}

fn mirror_of(w: &World, element: ElementId, kind: &str) -> Result<(), String> {
    match w.tree.get(element) {
        Some(e) if e.creator == Creator::Server && e.kind == kind => Ok(()),
        _ => Err(format!("{element} is not a {kind} of the server's")),
    }
}

fn mirror(w: &World, element: ElementId) -> Result<(), String> {
    match w.tree.get(element) {
        Some(e) if e.creator == Creator::Server => Ok(()),
        _ => Err(format!("{element} is not a mirror of the server's")),
    }
}
