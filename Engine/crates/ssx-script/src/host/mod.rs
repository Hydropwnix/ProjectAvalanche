pub(crate) mod call;
pub(crate) mod elements;
pub(crate) mod events;
pub mod game;
mod inbound;
pub(crate) mod latent;
pub mod limits;
pub mod loopback;
pub mod monitor;
pub(crate) mod ops;
pub mod outbox;
pub mod package;
pub mod server;
pub(crate) mod storage;
mod tick;
#[cfg(feature = "wire")]
pub mod wire;
pub(crate) mod world;

use std::cell::Cell;
use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::sync::{Arc, Weak};

use self::call::{Dispatch, Errors, Outcome, dispatch, enter};
use self::elements::{Creator, Tree};
use self::monitor::{Counts, Monitor, PackageMonitor, StopReason};
use self::outbox::{Body, Drained, Message, Recipients};
use self::package::{PackageScripts, setting_value};
pub use self::server::lifecycle::STATES;
use self::storage::{Store, Writer};
use self::world::{HostShared, Pkg, PkgState, World};
use crate::value::{ElementId, NetValue};
use crate::vm::Cutter;
use crate::{Limits, Output, ScriptError, ScriptVm};

pub use self::elements::{ClientId, LOCAL_BASE, ROOT};
pub use self::limits::HostLimits;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Server,
    Client,
}

impl Side {
    pub fn name(self) -> &'static str {
        match self {
            Side::Server => "server",
            Side::Client => "client",
        }
    }
}

#[derive(Clone, Debug)]
pub struct HostConfig {
    pub side: Side,
    pub limits: HostLimits,
    pub storage_root: Option<PathBuf>,
    pub settings: BTreeMap<String, toml::Table>,
}

impl HostConfig {
    pub fn new(side: Side) -> Self {
        let limits = match side {
            Side::Server => HostLimits::SERVER,
            Side::Client => HostLimits::CLIENT,
        };
        HostConfig { side, limits, storage_root: None, settings: BTreeMap::new() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandOrigin {
    Console,
    Client(ClientId),
}

pub struct ScriptHost {
    shared: Arc<HostShared>,
    settings: BTreeMap<String, toml::Table>,
    storage_root: Option<PathBuf>,
    one_caller: PhantomData<Cell<()>>,
}

impl ScriptHost {
    pub fn new(config: HostConfig) -> Result<Self, String> {
        let b = config.limits.budgets;
        Limits { call: b.call, call_backstop: b.call_backstop, memory: b.memory }.check().map_err(|e| e.to_string())?;
        crate::cpu::check()?;
        let writer = match &config.storage_root {
            Some(root) => Some(Writer::new(root.clone()).map_err(|e| format!("the storage writer: {e}"))?),
            None => None,
        };
        if b.chain <= b.call || b.chain_backstop <= b.call_backstop {
            return Err(format!(
                "the chain bound ({} checkpoints, backstop {:?}) must exceed the call budget ({} checkpoints, backstop \
                 {:?})",
                b.chain, b.chain_backstop, b.call, b.call_backstop
            ));
        }
        if b.tick_checkpoints == 0 || b.tick_backstop.is_zero() || b.tick.is_zero() || b.over_tick == 0 {
            return Err("the tick budget, its backstop, the tick time and the ticks in a row must not be zero".into());
        }
        let shared = Arc::new_cyclic(|me: &Weak<HostShared>| {
            let me = me.clone();
            let cutter: Cutter = Box::new(move |now, unit| {
                if let Some(h) = me.upgrade() {
                    call::cut_chain(&h, now, unit);
                }
            });
            HostShared::new(config.side, config.limits, writer, cutter)
        });
        Ok(ScriptHost { shared, settings: config.settings, storage_root: config.storage_root, one_caller: PhantomData })
    }

    pub fn side(&self) -> Side {
        self.shared.side
    }

    pub fn limits(&self) -> &HostLimits {
        &self.shared.limits
    }

    pub fn start(&self, pkg: PackageScripts) -> Result<(), String> {
        let r = self.start_inner(pkg, self.storage_root.clone());
        self.settle();
        r
    }

    pub fn start_with_storage(&self, pkg: PackageScripts, storage_root: &std::path::Path) -> Result<(), String> {
        let root = self.storage_root.as_ref().map(|_| storage_root.to_path_buf());
        let r = self.start_inner(pkg, root);
        self.settle();
        r
    }

    fn start_inner(&self, pkg: PackageScripts, storage_root: Option<PathBuf>) -> Result<(), String> {
        let side = self.shared.side;
        let limits = self.shared.limits;
        package::check_name(&pkg.name)?;
        if pkg.api > crate::api::API_LEVEL {
            return Err(format!("it needs API level {} (this build has {})", pkg.api, crate::api::API_LEVEL));
        }
        let name = pkg.name.clone();
        let compiled = compile_modules(&pkg.modules)?;
        let mut settings = pkg.settings.clone();
        if let Some(over) = self.settings.get(&name) {
            for (k, v) in over {
                settings.insert(k.clone(), v.clone());
            }
        }
        let settings = setting_value(&toml::Value::Table(settings), 0).map_err(|e| format!("[settings]: {e}"))?;
        if self.shared.lock().packages.contains_key(&name) {
            return Err(format!("{name} is already running"));
        }
        self.pre_start(&name)?;
        self.state_change(&name, "loaded", "starting");
        let root = {
            let mut w = self.shared.lock();
            let made = if w.packages.contains_key(&name) {
                Err(format!("{name} is already running"))
            } else if w.packages.len() >= limits.packages {
                Err(format!("{} packages are running (at most {})", w.packages.len(), limits.packages))
            } else {
                w.tree.create("resource", ROOT, Creator::Host, None, side == Side::Client, limits.tree_depth)
            };
            match made {
                Ok(root) => {
                    w.tree.set_name(root, &name);
                    if side == Side::Server {
                        let kind = "resource".into();
                        let body = Body::Created { element: root, kind, name: name.clone(), parent: ROOT };
                        ops::replicate(&mut w, side, None, Recipients::All, body);
                    }
                    root
                }
                Err(e) => {
                    drop(w);
                    self.state_change(&name, "starting", "stopped");
                    return Err(e);
                }
            }
        };
        let ctx = crate::api::Ctx { shared: Arc::downgrade(&self.shared), package: name.clone(), side, root };
        let b = limits.budgets;
        let vm_limits = Limits { call: b.call, call_backstop: b.call_backstop, memory: b.memory };
        let chain = self.shared.chain.clone();
        let vm = match ScriptVm::build_on(pkg.modules.clone(), compiled.clone(), vm_limits, chain, |lua, g| {
            crate::api::install(lua, g, &ctx)
        }) {
            Ok(vm) => Arc::new(vm),
            Err(e) => {
                let mut w = self.shared.lock();
                ops::destroy(&mut w, side, root);
                drop(w);
                self.state_change(&name, "starting", "stopped");
                return Err(format!("its VM: {e}"));
            }
        };
        let store = storage_root.as_ref().map(|r| {
            let settled = self.shared.lock().writer.as_ref().is_none_or(|w| w.settled(&name));
            if settled {
                Store::open(r, &name, &limits.storage)
            } else {
                Store::unavailable("the last instance's final write has not finished")
            }
        });
        {
            let mut w = self.shared.lock();
            let mut p = new_pkg(vm.clone(), root, settings, store);
            p.client_scripts = pkg.client_scripts;
            w.packages.insert(name.clone(), p);
            server::rights::resolve(&mut w, &name);
        }
        for (path, _) in &pkg.scripts {
            let during = format!("the top level of {path}");
            let Some(code) = compiled.get(path) else {
                return Err(self.roll_back(&name, format!("{path}: not among the package's modules")));
            };
            let f = match vm.load_script(path, code) {
                Ok((f, _)) => f,
                Err(ScriptError::Memory(limit)) => {
                    self.shared.lock().stop(&name, StopReason::Memory { limit, during: during.clone() });
                    return Err(self.roll_back(&name, String::new()));
                }
                Err(e) => return Err(self.roll_back(&name, format!("{path}: {e}"))),
            };
            match enter(&self.shared, &name, &during, Errors::Return, |vm| vm.run::<()>(&f, ())) {
                Outcome::Done(()) => {}
                Outcome::Failed(e) => return Err(self.roll_back(&name, format!("{path}: {e}"))),
                Outcome::Stopped => return Err(self.roll_back(&name, String::new())),
            }
        }
        let event = match side {
            Side::Server => "onResourceStart",
            Side::Client => "onClientResourceStart",
        };
        match dispatch(&self.shared, &Dispatch { name: event, source: root, args: &[], only: None, client: None }) {
            Ok(false) => {}
            Ok(true) => return Err(self.roll_back(&name, format!("a handler of {event} cancelled it"))),
            Err(e) => return Err(self.roll_back(&name, e)),
        }
        let mut w = self.shared.lock();
        match w.packages.get_mut(&name) {
            Some(p) if p.state == PkgState::Starting => {
                p.state = PkgState::Running;
                p.ran = true;
                w.order.push(name.clone());
                drop(w);
                self.state_change(&name, "starting", "running");
                Ok(())
            }
            _ => Err(format!("{name} was stopped while it started")),
        }
    }

    fn roll_back(&self, name: &str, why: String) -> String {
        let mut w = self.shared.lock();
        if !why.is_empty() {
            w.stop(name, StopReason::Start(why.clone()));
            return why;
        }
        w.pending_stops.iter().find(|(p, _)| p == name).map(|(_, r)| r.to_string()).unwrap_or(why)
    }

    pub fn stop(&self, name: &str, why: &str) -> bool {
        let (root, ran) = match self.shared.lock().running(name) {
            Some(p) => (p.root, p.ran),
            None => return false,
        };
        let event = match self.shared.side {
            Side::Server => "onResourceStop",
            Side::Client => "onClientResourceStop",
        };
        self.state_change(name, if ran { "running" } else { "starting" }, "stopping");
        let _ = dispatch(&self.shared, &Dispatch { name: event, source: root, args: &[], only: None, client: None });
        self.shared.lock().stop(name, StopReason::Requested(why.to_string()));
        self.settle();
        true
    }

    pub fn restart(&self, pkg: PackageScripts) -> Result<(), String> {
        self.stop(&pkg.name, "restarted");
        self.start(pkg)
    }

    pub fn running(&self) -> Vec<String> {
        self.shared.lock().order.clone()
    }

    fn settle(&self) {
        loop {
            let next = {
                let mut w = self.shared.lock();
                if !w.calls.is_empty() || w.pending_stops.is_empty() {
                    return;
                }
                w.pending_stops.remove(0)
            };
            let (name, reason) = next;
            let found = self.shared.lock().packages.get(&name).map(|p| (p.root, p.ran));
            if let Some((root, ran)) = found
                && !matches!(reason, StopReason::Requested(_))
            {
                let event = match self.shared.side {
                    Side::Server => "onResourceStop",
                    Side::Client => "onClientResourceStop",
                };
                self.state_change(&name, if ran { "running" } else { "starting" }, "stopping");
                let _ = dispatch(
                    &self.shared,
                    &Dispatch { name: event, source: root, args: &[], only: None, client: None },
                );
            }
            {
                let mut w = self.shared.lock();
                clean_up(&mut w, self.shared.side, &name);
                w.record_stop(&name, reason);
            }
            if found.is_some() {
                self.state_change(&name, "stopping", "stopped");
            }
        }
    }

    pub fn drain(&self, max: usize) -> Drained {
        self.shared.lock().outbox.drain(max)
    }

    pub fn snapshot(&self, client: ClientId) -> Vec<Message> {
        ops::snapshot(&self.shared.lock(), client)
    }

    pub fn connect_client(&self, client: ClientId) -> Result<ElementId, String> {
        if self.shared.side != Side::Server {
            return Err("clients connect to the server's host".into());
        }
        let mut w = self.shared.lock();
        if w.clients.contains_key(&client) {
            return Err(format!("client {} is already connected", client.0));
        }
        let id = w.tree.create("player", ROOT, Creator::Session, Some(client), false, self.shared.limits.tree_depth)?;
        let others: Vec<ClientId> = w.clients.keys().copied().collect();
        w.clients.insert(client, id);
        let body = Body::Created { element: id, kind: "player".into(), name: String::new(), parent: ROOT };
        ops::replicate(&mut w, Side::Server, None, Recipients::Some(others), body);
        Ok(id)
    }

    pub fn disconnect_client(&self, client: ClientId) {
        let mut w = self.shared.lock();
        let Some(player) = w.clients.get(&client).copied() else { return };
        ops::destroy(&mut w, Side::Server, player);
        w.clients.remove(&client);
        w.client_data.remove(&client);
        let ids: Vec<ElementId> = w.tree.iter().map(|(id, _)| id).collect();
        for id in ids {
            if let Some(e) = w.tree.get_mut(id) {
                if e.owner == Some(client) {
                    e.owner = None;
                }
                for subs in e.subscribers.values_mut() {
                    subs.remove(&client);
                }
            }
        }
        w.latent_in.drop_peer(outbox::Peer::Client(client));
        for p in w.packages.values_mut() {
            p.acknowledged.remove(&client);
        }
    }

    pub fn set_local_player(&self, element: ElementId) -> Result<(), String> {
        let mut w = self.shared.lock();
        if self.shared.side != Side::Client || w.tree.get(element).is_none_or(|e| e.creator != Creator::Server) {
            return Err("the local player is a known element of the server's, on a client".into());
        }
        w.local_player = Some(element);
        Ok(())
    }

    pub fn take_output(&self, package: &str) -> Option<Output> {
        let vm = self.shared.lock().packages.get(package).map(|p| p.vm.clone())?;
        Some(vm.take_output())
    }

    pub fn take_last_output(&self) -> Vec<(String, Output)> {
        self.shared.lock().last_output.drain(..).collect()
    }

    pub fn monitor(&self) -> Monitor {
        let w = self.shared.lock();
        let packages = w.order.iter().filter_map(|n| w.packages.get(n).map(|p| (n, p)));
        let packages = packages
            .map(|(n, p)| PackageMonitor {
                name: n.clone(),
                checkpoints: p.count_window.stats(),
                time: p.window.stats(),
                call_peak: p.call_peak,
                charged: p.charged,
                charged_cpu: p.charged_cpu,
                heap: p.vm.used_memory(),
                heap_peak: p.heap_peak,
                counts: counts(&w, n, false),
                errors: p.errors,
                refusals: p.refusals,
                over_budget: p.over,
            })
            .collect();
        Monitor {
            server: self.shared.side == Side::Server,
            tick: w.tick,
            budgets: self.shared.limits.budgets,
            side_time: w.side_window.stats(),
            packages,
            stops: w.stops.iter().cloned().collect(),
            outbox_messages: w.outbox.len(),
            outbox_bytes: w.outbox.bytes(),
            latent_receiving: w.latent_in.open(),
            refused_inbound: w.refused_inbound,
            chain_peak_checkpoints: w.chain_peak_checkpoints,
            chain_peak: w.chain_peak,
            chain_peak_wall: w.chain_peak_wall,
            chain_cuts: w.chain_cuts,
            lifecycle: w.server.lifecycle,
            rights: w.server.rights_counts,
        }
    }

    pub fn holdings(&self, package: &str) -> (Counts, bool) {
        let w = self.shared.lock();
        (counts(&w, package, true), w.packages.contains_key(package))
    }

    pub fn elements(&self) -> usize {
        self.shared.lock().tree.len()
    }

    pub fn set_catalog(&self, catalog: Arc<game::Catalog>) {
        self.shared.lock().game.catalog = catalog;
    }

    pub fn markers(&self) -> Vec<game::MarkerView> {
        self.shared.lock().game.markers.list()
    }
}

impl Drop for ScriptHost {
    fn drop(&mut self) {
        let names = self.running();
        for n in names.iter().rev() {
            self.stop(n, "the host is going away");
        }
    }
}

fn compile_modules(modules: &crate::FileMap) -> Result<crate::require::Compiled, String> {
    let mut out = BTreeMap::new();
    for (path, source) in modules {
        let code = crate::compile::bytecode(path, source.as_bytes()).map_err(|e| e.0)?;
        out.insert(path.clone(), Arc::<[u8]>::from(code));
    }
    Ok(Arc::new(out))
}

fn new_pkg(vm: Arc<ScriptVm>, root: ElementId, settings: NetValue, store: Option<Store>) -> Pkg {
    Pkg {
        vm,
        state: PkgState::Starting,
        ran: false,
        client_scripts: false,
        acknowledged: Default::default(),
        root,
        settings,
        elements: 0,
        data_keys: 0,
        data_bytes: 0,
        timers: 0,
        exports: BTreeMap::new(),
        queue: Default::default(),
        queue_bytes: 0,
        store,
        window: Default::default(),
        tick_time: Default::default(),
        count_window: Default::default(),
        tick_count: 0,
        tick_charge: 0,
        tick_charge_cpu: Default::default(),
        charged: 0,
        charged_cpu: Default::default(),
        over: 0,
        call_peak: 0,
        heap_peak: 0,
        errors: 0,
        refusals: 0,
        tick_messages: 0,
        tick_bytes: 0,
        last_during: String::new(),
        calls: None,
    }
}

fn clean_up(w: &mut World, side: Side, name: &str) {
    w.events.remove_package(name);
    server::joins::package_stopped(&mut w.server.joins, name);
    w.game.binds.retain(|b| b.package != name);
    w.game.hud_at.remove(name);
    w.game.music_at.remove(name);
    let timers: Vec<(std::time::Duration, u64)> =
        w.timers.due.iter().filter(|(_, t)| t.package == name).map(|(k, _)| *k).collect();
    for k in timers {
        w.timers.due.remove(&k);
    }
    w.timers.at.retain(|_, (_, p)| p != name);
    let own = Creator::Package(name.to_string());
    let roots: Vec<ElementId> = w
        .tree
        .iter()
        .filter(|(_, e)| {
            e.creator == own && !e.parent.is_some_and(|p| w.tree.get(p).is_some_and(|pe| pe.creator == own))
        })
        .map(|(id, _)| id)
        .collect();
    let mut roots = roots;
    if let Some(p) = w.packages.get(name) {
        roots.push(p.root);
    }
    for r in roots {
        ops::destroy(w, side, r);
    }
    let ids: Vec<ElementId> = w.tree.iter().map(|(id, _)| id).collect();
    for id in ids {
        let Some(e) = w.tree.get_mut(id) else { continue };
        e.writable.retain(|_, by| by != name);
        for subs in e.subscribers.values_mut() {
            subs.retain(|_, by| by != name);
        }
        let mine: Vec<String> = e
            .data
            .iter()
            .filter(|(_, d)| d.writer == elements::Writer::Package(name.to_string()))
            .map(|(k, _)| k.clone())
            .collect();
        for key in mine {
            let Some(e) = w.tree.get_mut(id) else { break };
            let Some(d) = e.data.remove(&key) else { continue };
            let to = match d.sync {
                elements::Sync::Local => None,
                elements::Sync::Broadcast => Some(Recipients::All),
                elements::Sync::Subscribers => {
                    Some(Recipients::Some(e.subscribers.get(&key).into_iter().flatten().map(|(c, _)| *c).collect()))
                }
            };
            if !Tree::is_local(id)
                && let Some(to) = to
            {
                ops::replicate(w, side, Some(name), to, Body::Data { element: id, key, value: None });
            }
        }
    }
    w.outbox.purge_package(name);
    w.latent_out.remove_package(name);
    w.latent_in.remove_package(name);
    w.order.retain(|n| n != name);
    if let Some(mut p) = w.packages.remove(name) {
        if side == Side::Server && !p.tick_time.is_zero() {
            let spent = w.server.spent.entry(name.to_string()).or_default();
            *spent = spent.saturating_add(p.tick_time);
        }
        if let Some(store) = p.store.as_mut()
            && store.dirty
            && let Some(writer) = &w.writer
            && !writer.queue_final(name, store)
        {
            p.vm.note("storage: the last write could not be queued; the newest values are lost".into());
        }
        w.keep_last_output(name, p.vm.take_output());
    }
}

fn counts(w: &World, name: &str, scan: bool) -> Counts {
    let p = w.packages.get(name);
    let mut c = Counts {
        handlers: w.events.handlers_of(name),
        events: w.events.declared_of(name),
        commands: w.events.commands_of(name),
        latent: w.latent_out.count(name),
        outbox: w.outbox.count_package(name),
        markers: w.game.markers.count_of(name),
        binds: w.game.binds.iter().filter(|b| b.package == name).count(),
        ..Counts::default()
    };
    if scan {
        let own = Creator::Package(name.to_string());
        let writer = elements::Writer::Package(name.to_string());
        for (_, e) in w.tree.iter() {
            if e.creator == own {
                c.elements += 1;
            }
            for d in e.data.values().filter(|d| d.writer == writer) {
                c.data_keys += 1;
                c.data_bytes += d.bytes;
            }
        }
        c.timers = w.timers.at.values().filter(|(_, p)| p == name).count();
    } else if let Some(p) = p {
        c.elements = p.elements;
        c.data_keys = p.data_keys;
        c.data_bytes = p.data_bytes;
        c.timers = p.timers;
    }
    if let Some(p) = p {
        c.exports = p.exports.len();
        c.queued = p.queue.len();
        if let Some(s) = &p.store {
            c.storage_keys = s.keys();
            c.storage_bytes = s.bytes();
        }
    }
    c
}
