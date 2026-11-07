pub mod joins;
pub mod lifecycle;
pub mod rights;

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use super::elements::{ClientId, Creator, ROOT};
use super::game::markers::{MarkerView, hit};
use super::outbox::{Body, Destination, MarkerSpec, Message, Recipients};
use super::{ScriptHost, Side, ops};
use crate::value::{ElementId, NetValue};

#[derive(Clone, Debug, PartialEq)]
pub struct PlayerView {
    pub client: ClientId,
    pub name: String,
    pub instance: Option<u16>,
    pub rider: Option<u16>,
    pub progress: &'static str,
    pub status: &'static str,
    pub ping_ms: u32,
    pub position: Option<[f32; 3]>,
    pub velocity: Option<[f32; 3]>,
    pub run: Option<RunView>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunView {
    pub split: Option<u8>,
    pub clock_ms: u32,
    pub points: u32,
    pub state: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StandingFrame {
    pub rider: u16,
    pub name: String,
    pub state: &'static str,
    pub clock_ms: u32,
    pub points: u32,
    pub split: Option<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InstanceFrame {
    pub id: u16,
    pub mode: String,
    pub drop: String,
    pub state: &'static str,
    pub round: u16,
    pub clock_ms: f64,
    pub standings: Vec<StandingFrame>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerEvent {
    Checkpoint { client: ClientId, split: u8, clock_ms: u32 },
    Finish { client: ClientId, clock_ms: u32, points: u32, failed: bool },
    RoundStart { instance: u16, round: u16, drop: String },
    RoundEnd { instance: u16, round: u16 },
    Shutdown { round_end_secs: u32 },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServerFrame {
    pub now: Duration,
    pub instances: Vec<InstanceFrame>,
    pub players: Vec<PlayerView>,
    pub events: Vec<ServerEvent>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServerOrders {
    pub end_round: Vec<EndRound>,
    pub operation: Vec<rights::Privileged>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EndRound {
    pub instance: u16,
    pub round: u16,
    pub reason: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkerKind {
    Marker,
    Checkpoint,
    Trigger,
}

impl MarkerKind {
    pub fn element_type(self) -> &'static str {
        match self {
            MarkerKind::Marker => "marker",
            MarkerKind::Checkpoint => "checkpoint",
            MarkerKind::Trigger => "trigger",
        }
    }
}

pub(crate) struct ServerMarker {
    pub view: MarkerView,
    pub kind: MarkerKind,
    pub index: Option<u16>,
    pub inside: BTreeSet<ClientId>,
}

impl ServerMarker {
    pub fn spec(&self) -> Option<MarkerSpec> {
        (self.kind != MarkerKind::Trigger).then(|| MarkerSpec {
            id: self.view.id.to_string(),
            position: self.view.position,
            heading: self.view.heading,
            size: self.view.size,
            colour: self.view.colour,
            visible: self.view.visible,
            checkpoint: self.index.filter(|_| self.kind == MarkerKind::Checkpoint),
            instance: self.view.instance,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RiderRec {
    pub element: ElementId,
    pub number: u16,
    pub instance: u16,
}

#[derive(Default)]
pub struct ServerState {
    pub(crate) frame: ServerFrame,
    pub(crate) instances: BTreeMap<u16, ElementId>,
    pub(crate) riders: BTreeMap<ClientId, RiderRec>,
    pub(crate) markers: BTreeMap<ElementId, ServerMarker>,
    pub(crate) last: BTreeMap<ClientId, [f32; 3]>,
    pub(crate) orders: ServerOrders,
    pub(crate) joins: joins::Joins,
    pub(crate) rights: Option<std::sync::Arc<dyn rights::Rights>>,
    pub(crate) rights_counts: rights::RightsCounts,
    pub(crate) pre_start: Option<lifecycle::PreStart>,
    pub(crate) lifecycle: lifecycle::LifecycleCounts,
    pub(crate) spent: BTreeMap<String, Duration>,
}

impl ServerState {
    pub(crate) fn forget(&mut self, gone: &[ElementId]) {
        self.markers.retain(|e, _| !gone.contains(e));
        self.riders.retain(|_, r| !gone.contains(&r.element));
        self.instances.retain(|_, e| !gone.contains(e));
    }

    pub(crate) fn player(&self, client: ClientId) -> Option<&PlayerView> {
        self.frame.players.iter().find(|p| p.client == client)
    }

    pub(crate) fn instance(&self, id: u16) -> Option<&InstanceFrame> {
        self.frame.instances.iter().find(|i| i.id == id)
    }

    pub(crate) fn rider_client(&self, element: ElementId) -> Option<ClientId> {
        self.riders.iter().find(|(_, r)| r.element == element).map(|(c, _)| *c)
    }

    pub(crate) fn instance_id(&self, element: ElementId) -> Option<u16> {
        self.instances.iter().find(|(_, e)| **e == element).map(|(i, _)| *i)
    }

    pub(crate) fn clients_in(&self, id: u16) -> Vec<ClientId> {
        self.frame.players.iter().filter(|p| p.instance == Some(id)).map(|p| p.client).collect()
    }
}

type Raise = (&'static str, ElementId, Vec<NetValue>);

fn num(v: impl Into<f64>) -> NetValue {
    NetValue::Number(v.into())
}

impl ScriptHost {
    pub fn join_session(&self, client: ClientId, name: &str) -> Result<(ElementId, Vec<Message>), String> {
        crate::api::check_element_name(name)?;
        let player = self.connect_client(client)?;
        let first = {
            let mut w = self.shared.lock();
            w.tree.set_name(player, name);
            let others: Vec<ClientId> = w.clients.keys().copied().filter(|c| *c != client).collect();
            let body = Body::Renamed { element: player, name: name.to_string() };
            ops::replicate(&mut w, Side::Server, None, Recipients::Some(others), body);
            let mut first = ops::snapshot(&w, client);
            let to = Destination::Clients(Recipients::Some(vec![client]));
            first.push(Message { package: None, to, body: Body::Hello { player } });
            first
        };
        self.raise_engine("onPlayerJoin", player, &[]);
        Ok((player, first))
    }

    pub fn element_count(&self) -> usize {
        self.shared.lock().tree.len()
    }

    pub fn clients(&self) -> Vec<ClientId> {
        self.shared.lock().clients.keys().copied().collect()
    }

    pub fn player_left(&self, client: ClientId, reason: &str) {
        let player = self.shared.lock().player_of(client);
        let Some(player) = player else { return };
        let why = NetValue::String(ssx_core_text(reason).into_bytes());
        self.raise_engine("onPlayerQuit", player, &[why]);
        let mut w = self.shared.lock();
        if let Some(r) = w.server.riders.remove(&client) {
            ops::destroy(&mut w, Side::Server, r.element);
        }
        w.server.last.remove(&client);
        for m in w.server.markers.values_mut() {
            m.inside.remove(&client);
        }
        drop(w);
        self.disconnect_client(client);
    }

    pub fn player_flooded(&self, client: ClientId, dropped: u64) {
        let player = self.shared.lock().player_of(client);
        let Some(player) = player else { return };
        if dropped == 0 {
            return;
        }
        let n = dropped.min(1 << 53) as f64;
        self.raise_engine("onPlayerEventFlood", player, &[NetValue::Number(n)]);
    }

    pub fn server_frame(&self, f: ServerFrame) -> Result<ServerOrders, String> {
        if self.shared.side != Side::Server {
            return Err("the server's frame is the server's".into());
        }
        let now = f.now;
        let raised = self.take_frame(f)?;
        for (name, source, args) in raised {
            self.raise_engine(name, source, &args);
        }
        self.tick(now);
        Ok(std::mem::take(&mut self.shared.lock().server.orders))
    }

    fn take_frame(&self, mut f: ServerFrame) -> Result<Vec<Raise>, String> {
        let depth = self.shared.limits.tree_depth;
        let mut w = self.shared.lock();
        let events = std::mem::take(&mut f.events);
        let ids: BTreeSet<u16> = f.instances.iter().map(|i| i.id).collect();
        let gone: Vec<ElementId> =
            w.server.instances.iter().filter(|(i, _)| !ids.contains(i)).map(|(_, e)| *e).collect();
        for e in gone {
            ops::destroy(&mut w, Side::Server, e);
        }
        for i in &f.instances {
            if w.server.instances.contains_key(&i.id) {
                continue;
            }
            let e = w.tree.create("instance", ROOT, Creator::Session, None, false, depth)?;
            let name = i.id.to_string();
            w.tree.set_name(e, &name);
            w.server.instances.insert(i.id, e);
            let body = Body::Created { element: e, kind: "instance".into(), name, parent: ROOT };
            ops::replicate(&mut w, Side::Server, None, Recipients::All, body);
        }
        let placed: BTreeMap<ClientId, (u16, u16, &str)> = f
            .players
            .iter()
            .filter_map(|p| Some((p.client, (p.instance?, p.rider?, p.name.as_str()))))
            .filter(|(c, (i, _, _))| w.server.instances.contains_key(i) && w.clients.contains_key(c))
            .collect();
        let stale: Vec<ClientId> = w
            .server
            .riders
            .iter()
            .filter(|(c, r)| placed.get(c).is_none_or(|(i, n, _)| *i != r.instance || *n != r.number))
            .map(|(c, _)| *c)
            .collect();
        for c in stale {
            if let Some(r) = w.server.riders.remove(&c) {
                ops::destroy(&mut w, Side::Server, r.element);
            }
            w.server.last.remove(&c);
        }
        for (&client, &(instance, number, name)) in &placed {
            if w.server.riders.contains_key(&client) {
                continue;
            }
            let (Some(&parent), Some(player)) = (w.server.instances.get(&instance), w.player_of(client)) else {
                continue;
            };
            let e = w.tree.create("rider", parent, Creator::Session, Some(client), false, depth)?;
            w.tree.set_name(e, name);
            w.server.riders.insert(client, RiderRec { element: e, number, instance });
            let created = Body::Created { element: e, kind: "rider".into(), name: name.to_string(), parent };
            ops::replicate(&mut w, Side::Server, None, Recipients::All, created);
            ops::replicate(&mut w, Side::Server, None, Recipients::All, Body::Rider { element: e, player, number });
        }
        let mut raised: Vec<Raise> = Vec::new();
        let mut last = BTreeMap::new();
        for p in &f.players {
            let (Some(p1), Some(r)) = (p.position, w.server.riders.get(&p.client).copied()) else { continue };
            last.insert(p.client, p1);
            let Some(p0) = w.server.last.get(&p.client).copied() else { continue };
            let clock = p.run.as_ref().map_or(0, |r| r.clock_ms);
            let own = w.server.instances.get(&r.instance).copied();
            for (&el, m) in w.server.markers.iter_mut() {
                if !(m.view.visible || m.kind == MarkerKind::Trigger) {
                    continue;
                }
                if m.view.instance.is_some_and(|i| Some(i) != own) {
                    continue;
                }
                m.view.inside = m.inside.contains(&p.client);
                let h = hit(&mut m.view, p0, p1);
                if m.view.inside {
                    m.inside.insert(p.client);
                } else {
                    m.inside.remove(&p.client);
                }
                let Some(forward) = h else { continue };
                match m.kind {
                    MarkerKind::Checkpoint => raised.push((
                        "onRiderCheckpoint",
                        r.element,
                        vec![NetValue::Element(el), num(m.index.unwrap_or(0)), num(clock)],
                    )),
                    MarkerKind::Marker | MarkerKind::Trigger => {
                        raised.push(("onMarkerHit", el, vec![NetValue::Element(r.element), NetValue::Bool(forward)]))
                    }
                }
            }
        }
        w.server.last = last;
        let mut own: Vec<Raise> = Vec::new();
        let rider = |w: &super::world::World, c: &ClientId| w.server.riders.get(c).map(|r| r.element);
        for e in events {
            match e {
                ServerEvent::Checkpoint { client, split, clock_ms } => {
                    if let Some(r) = rider(&w, &client) {
                        own.push(("onRiderCheckpoint", r, vec![NetValue::Nil, num(split), num(clock_ms)]));
                    }
                }
                ServerEvent::Finish { client, clock_ms, points, failed } => {
                    if let Some(r) = rider(&w, &client) {
                        own.push(("onRiderFinish", r, vec![num(clock_ms), num(points), NetValue::Bool(failed)]));
                    }
                }
                ServerEvent::RoundStart { instance, round, drop } => {
                    if let Some(&i) = w.server.instances.get(&instance) {
                        own.push(("onRoundStart", i, vec![num(round), NetValue::String(drop.into_bytes())]));
                    }
                }
                ServerEvent::RoundEnd { instance, round } => {
                    if let Some(&i) = w.server.instances.get(&instance) {
                        own.push(("onRoundEnd", i, vec![num(round)]));
                    }
                }
                ServerEvent::Shutdown { round_end_secs } => own.push(("onShutdown", ROOT, vec![num(round_end_secs)])),
            }
        }
        w.server.frame = f;
        own.extend(raised);
        Ok(own)
    }

    pub fn recipients(&self, m: &Message) -> Vec<ClientId> {
        let w = self.shared.lock();
        let list = match &m.to {
            Destination::Server => return Vec::new(),
            Destination::Clients(Recipients::Some(list)) => return list.clone(),
            Destination::Clients(Recipients::All) => w.clients.keys().copied().collect::<Vec<_>>(),
        };
        let source = match &m.body {
            Body::Event { source, .. } | Body::LatentStart { source, .. } => *source,
            _ => return list,
        };
        match instance_of(&w, source, self.shared.limits.tree_depth) {
            Some(i) => {
                let inside = w.server.clients_in(i);
                list.into_iter().filter(|c| inside.contains(c)).collect()
            }
            None => list,
        }
    }
}

fn instance_of(w: &super::world::World, element: ElementId, depth: usize) -> Option<u16> {
    if let Some(c) = w.client_of_player(element) {
        return w.server.player(c).and_then(|p| p.instance);
    }
    let mut at = Some(element);
    for _ in 0..=depth {
        let e = at?;
        if let Some(i) = w.server.instance_id(e) {
            return Some(i);
        }
        at = w.tree.get(e).and_then(|x| x.parent);
    }
    None
}

fn ssx_core_text(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(256).collect()
}
