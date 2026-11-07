use std::time::Duration;

use super::outbox::{Destination, Message, Peer, Recipients};
use super::{ClientId, HostConfig, ScriptHost, Side};
use crate::value::ElementId;

pub struct Loopback {
    pub server: ScriptHost,
    pub clients: Vec<(ClientId, ScriptHost)>,
    pub adopted: Option<ClientId>,
    pub now: Duration,
    pub refused: Vec<String>,
}

const REFUSED_KEPT: usize = 1024;

impl Loopback {
    pub fn new(server: HostConfig) -> Result<Self, String> {
        if server.side != Side::Server {
            return Err("a loopback session's first host is the server".into());
        }
        let server = ScriptHost::new(server)?;
        Ok(Loopback { server, clients: Vec::new(), adopted: None, now: Duration::ZERO, refused: Vec::new() })
    }

    fn note(&mut self, what: String) {
        if self.refused.len() == REFUSED_KEPT {
            self.refused.remove(0);
        }
        self.refused.push(what);
    }

    fn next_id(&self) -> Result<ClientId, String> {
        let top = self.clients.iter().map(|(c, _)| c.0).chain(self.adopted.map(|c| c.0)).max().unwrap_or(0);
        Ok(ClientId(top.checked_add(1).ok_or("too many clients")?))
    }

    pub fn join(&mut self, client: HostConfig) -> Result<(ClientId, ElementId), String> {
        if client.side != Side::Client {
            return Err("a joining host is a client's".into());
        }
        let id = self.next_id()?;
        let host = ScriptHost::new(client)?;
        let player = self.server.connect_client(id)?;
        for m in self.server.snapshot(id) {
            host.deliver(Peer::Server, m.package.as_deref(), m.body)?;
        }
        host.set_local_player(player)?;
        self.clients.push((id, host));
        Ok((id, player))
    }

    pub fn connect(&mut self, host: ScriptHost, name: &str) -> Result<(ClientId, ElementId), String> {
        let (id, player) = self.admit(&host, name)?;
        self.clients.push((id, host));
        Ok((id, player))
    }

    pub fn adopt(&mut self, host: &ScriptHost, name: &str) -> Result<(ClientId, ElementId), String> {
        let (id, player) = self.admit(host, name)?;
        self.adopted = Some(id);
        Ok((id, player))
    }

    fn admit(&mut self, host: &ScriptHost, name: &str) -> Result<(ClientId, ElementId), String> {
        if host.side() != Side::Client {
            return Err("a joining host is a client's".into());
        }
        if self.adopted.is_some() {
            return Err("a client host is adopted: no other joins".into());
        }
        self.carry();
        let id = self.next_id()?;
        let (player, first) = self.server.join_session(id, name)?;
        for m in first {
            if let Err(e) = host.deliver(Peer::Server, m.package.as_deref(), m.body) {
                self.note(format!("client {} (join): {e}", id.0));
            }
        }
        Ok((id, player))
    }

    pub fn release(&mut self, client: ClientId, reason: &str) -> Option<ScriptHost> {
        let k = self.clients.iter().position(|(c, _)| *c == client)?;
        let (_, host) = self.clients.remove(k);
        self.server.player_left(client, reason);
        Some(host)
    }

    pub fn release_adopted(&mut self, reason: &str) -> Option<ClientId> {
        let id = self.adopted.take()?;
        self.server.player_left(id, reason);
        Some(id)
    }

    pub fn leave(&mut self, client: ClientId) {
        self.clients.retain(|(c, _)| *c != client);
        self.server.disconnect_client(client);
    }

    pub fn client(&self, id: ClientId) -> Option<&ScriptHost> {
        self.clients.iter().find(|(c, _)| *c == id).map(|(_, h)| h)
    }

    pub fn carry(&mut self) {
        self.carry_with(None);
    }

    pub fn carry_with(&mut self, adopted: Option<&ScriptHost>) {
        let out = self.server.drain(usize::MAX);
        let mut hosts: Vec<(ClientId, &ScriptHost)> = self.clients.iter().map(|(c, h)| (*c, h)).collect();
        let mut notes = Vec::new();
        match (self.adopted, adopted) {
            (Some(id), Some(h)) => hosts.push((id, h)),
            (Some(id), None) => notes.push(format!("client {} (adopted): its host was not handed to the carry", id.0)),
            (None, _) => {}
        }
        if out.resync {
            for (id, h) in &hosts {
                for m in self.server.snapshot(*id) {
                    if let Err(e) = h.deliver(Peer::Server, m.package.as_deref(), m.body) {
                        notes.push(format!("client {} (resync): {e}", id.0));
                    }
                }
            }
        }
        for m in out.messages {
            let Destination::Clients(to) = &m.to else {
                notes.push("the server addressed the server".into());
                continue;
            };
            let reach_list = match to {
                Recipients::All => self.server.recipients(&m),
                Recipients::Some(list) => list.clone(),
            };
            for (id, c) in &hosts {
                if reach_list.contains(id)
                    && let Err(e) = c.deliver(Peer::Server, m.package.as_deref(), m.body.clone())
                {
                    notes.push(format!("client {}: {e}", id.0));
                }
            }
        }
        let mut up: Vec<(ClientId, Message)> = Vec::new();
        for (id, c) in &hosts {
            up.extend(c.drain(usize::MAX).messages.into_iter().map(|m| (*id, m)));
        }
        for (id, m) in up {
            if m.to != Destination::Server {
                notes.push(format!("client {} addressed other clients", id.0));
                continue;
            }
            if let Err(e) = self.server.deliver(Peer::Client(id), m.package.as_deref(), m.body) {
                notes.push(format!("server, from client {}: {e}", id.0));
            }
        }
        for n in notes {
            self.note(n);
        }
    }

    pub fn step(&mut self, dt: Duration) {
        self.carry();
        self.now = self.now.saturating_add(dt);
        self.server.tick(self.now);
        for (_, c) in &self.clients {
            c.tick(self.now);
        }
        self.carry();
    }
}
