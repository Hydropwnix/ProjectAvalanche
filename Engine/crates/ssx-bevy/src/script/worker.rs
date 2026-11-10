use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

use ssx_assets::content::GameContent;
use ssx_assets::content::markers::MarkerLook;
use ssx_content::marker::Hit;
use ssx_content::package::Origin;
use ssx_content::{ContentHash, ContentId, Kind, Layer, PackageSource};
use ssx_script::host::game::{Catalog, ContentMeta, FrameReport, GameFrame, MarkerHitShape, MarkerMeta};
use ssx_script::host::outbox::Message;
#[cfg(feature = "net")]
use ssx_script::host::outbox::{Body, Peer};
use ssx_script::host::package::PackageScripts;
use ssx_script::host::server::ServerFrame;
use ssx_script::host::{HostConfig, ScriptHost, Side};

mod clients;
pub(super) use clients::Word;
mod local;
#[cfg(test)]
pub(super) use local::local_client_is_the_first_admitted;
pub(super) use local::{LOCAL_CLIENT, LocalOut};
#[cfg(test)]
mod tests;

const STACK: usize = 8 << 20;
pub(super) const QUEUE: usize = 8;
const MAX_LINES: usize = 64;
const MAX_LINE: usize = 300;

pub(super) enum Job {
    Frame(Box<GameFrame>, Option<Box<ServerFrame>>),
    Restart(PathBuf),
    Clients(Word),
    Session {
        packages: Vec<(String, PathBuf, ContentHash)>,
        storage: PathBuf,
    },
    SessionScriptsOff,
    #[cfg(feature = "net")]
    Net(Vec<(Option<String>, Body)>),
    Local {
        package: String,
        player: String,
    },
    EndLocal(String),
    Shutdown,
}

pub(super) struct Out {
    pub frame: bool,
    pub report: FrameReport,
    pub monitor: String,
    pub lines: Vec<(bool, String)>,
    pub looks: Option<Arc<BTreeMap<ContentId, Arc<MarkerLook>>>>,
    pub net: Vec<Message>,
    pub session: Option<u64>,
    pub local: Option<LocalOut>,
}

pub(super) struct Start {
    pub big_root: PathBuf,
    pub packages: Vec<PathBuf>,
    pub dev: Option<PathBuf>,
    pub storage: PathBuf,
    pub server_packages: Vec<PathBuf>,
    pub server_storage: PathBuf,
}

pub(super) fn spawn(start: Start) -> (SyncSender<Job>, Receiver<Out>, Receiver<()>) {
    let (tx, jobs) = sync_channel::<Job>(QUEUE);
    let (outs, rx) = sync_channel::<Out>(QUEUE);
    let (done_tx, done) = sync_channel::<()>(1);
    let spawned = std::thread::Builder::new().name("scripts".into()).stack_size(STACK).spawn(move || {
        run(start, jobs, outs);
        let _ = done_tx.send(());
    });
    if let Err(e) = spawned {
        bevy::log::error!("scripts: the script host's thread could not start: {e}");
    }
    (tx, rx, done)
}

type HeldSession = (Vec<(String, PathBuf, ContentHash)>, PathBuf);
type Looks = Arc<BTreeMap<ContentId, Arc<MarkerLook>>>;

struct Worker {
    host: ScriptHost,
    content: Arc<GameContent>,
    packages: Vec<PathBuf>,
    dev: Option<PathBuf>,
    generation: Option<u64>,
    lines: Vec<(bool, String)>,
    last_stop: Option<ssx_script::host::monitor::StopRecord>,
    word: Option<Word>,
    clients_started: bool,
    start_waits: bool,
    client_names: BTreeSet<String>,
    held_off: BTreeSet<String>,
    catalogue_session: bool,
    looks: Option<Looks>,
    net_out: Vec<Message>,
    dropped: u64,
    session: BTreeMap<String, ContentHash>,
    held_session: Option<HeldSession>,
    #[cfg(feature = "net")]
    refused: u64,
    local: Option<local::Local>,
    server_packages: Vec<PathBuf>,
    server_storage: PathBuf,
    catalog: Option<Arc<Catalog>>,
}

fn run(start: Start, jobs: Receiver<Job>, outs: SyncSender<Out>) {
    let mut config = HostConfig::new(Side::Client);
    match ssx_files::create_dir_all(&start.storage) {
        Ok(()) => config.storage_root = Some(start.storage.clone()),
        Err(e) => bevy::log::warn!("scripts: no storage ({e})"),
    }
    let host = match ScriptHost::new(config) {
        Ok(h) => h,
        Err(e) => {
            bevy::log::error!("scripts: the client script host cannot start: {e}");
            return;
        }
    };
    let content = GameContent::of(&start.big_root);
    let mut w = Worker {
        host,
        content,
        packages: start.packages,
        dev: start.dev,
        generation: None,
        lines: Vec::new(),
        last_stop: None,
        word: None,
        clients_started: false,
        start_waits: false,
        client_names: BTreeSet::new(),
        held_off: BTreeSet::new(),
        catalogue_session: false,
        looks: None,
        net_out: Vec::new(),
        dropped: 0,
        session: BTreeMap::new(),
        held_session: None,
        #[cfg(feature = "net")]
        refused: 0,
        local: None,
        server_packages: start.server_packages,
        server_storage: start.server_storage,
        catalog: None,
    };
    w.refresh();
    while let Ok(job) = jobs.recv() {
        let answer = match job {
            Job::Frame(f, server) => {
                let local = server.and_then(|sf| w.local_before(*sf));
                let f = w.as_seen(*f);
                let report = match w.host.frame(f) {
                    Ok(r) => r,
                    Err(e) => {
                        w.note(true, format!("scripts: the frame was refused: {e}"));
                        FrameReport::default()
                    }
                };
                w.refresh();
                w.local_after();
                w.start_clients();
                w.collect();
                Some(w.out(true, report, local))
            }
            Job::Restart(root) => {
                w.restart(&root);
                None
            }
            Job::Clients(word) => {
                w.word(word);
                Some(w.out(false, FrameReport::default(), None))
            }
            Job::Session { .. } if !w.sees() => {
                w.note(true, "scripts: the server's client scripts came before its word on the client mods".into());
                None
            }
            Job::Session { packages, storage } if w.host.local_player().is_none() || w.local.is_some() => {
                w.end_local("a network session begins");
                w.note(false, "scripts: the server's client scripts wait for the server to name this player".into());
                w.held_session = Some((packages, storage));
                None
            }
            Job::Session { packages, storage } => {
                w.session(packages, &storage);
                None
            }
            Job::SessionScriptsOff => {
                w.session_scripts_off();
                None
            }
            #[cfg(feature = "net")]
            Job::Net(messages) => {
                w.net(messages);
                None
            }
            Job::Local { package, player } => {
                w.start_local(&package, &player);
                None
            }
            Job::EndLocal(why) => {
                w.end_local(&why);
                None
            }
            Job::Shutdown => {
                w.end_local("the game closes");
                return;
            }
        };
        if let Some(out) = answer
            && outs.send(out).is_err()
        {
            return;
        }
    }
    w.end_local("the game closes");
}

fn clean(s: &str) -> String {
    ssx_core::text::shown_max(s, MAX_LINE).to_string()
}

pub(super) fn hit_shape(h: Hit) -> MarkerHitShape {
    match h {
        Hit::None => MarkerHitShape::None,
        Hit::Sphere => MarkerHitShape::Sphere,
        Hit::Cylinder => MarkerHitShape::Cylinder,
        Hit::Ring => MarkerHitShape::Ring,
        Hit::Gate => MarkerHitShape::Gate,
    }
}

fn layer_name(l: Layer) -> &'static str {
    match l {
        Layer::Base => "L0 base",
        Layer::Port => "L1 port",
        Layer::ClientMods => "L2 client mods",
        Layer::Server => "L3 server",
    }
}

impl Worker {
    fn note(&mut self, problem: bool, line: String) {
        if self.lines.len() < MAX_LINES {
            self.lines.push((problem, line));
        }
    }

    fn catalogue(&mut self, blind: bool, g: u64) -> Option<Looks> {
        let t = std::time::Instant::now();
        let list = |r: &ssx_content::registry::Registry| {
            let mut ids = BTreeMap::new();
            for kind in Kind::ALL {
                for id in r.list(kind) {
                    if let Some(w) = r.winner(&id) {
                        ids.insert(id, ContentMeta { source: w.name.to_string(), layer: layer_name(w.layer) });
                    }
                }
            }
            ids
        };
        let ids = match blind {
            true => self.content.without_session(list)?,
            false => self.content.with_registry(list),
        };
        let mut cat = Catalog { ids, ..Catalog::default() };
        let disc = |source: &str| ContentMeta { source: source.to_string(), layer: layer_name(Layer::Base) };
        if let Some(s) = self.content.disc_sounds() {
            for id in s.by_id.keys() {
                cat.ids.entry(id.clone()).or_insert_with(|| disc("ssx"));
            }
        }
        if let Some(m) = self.content.disc_music() {
            for id in m.by_id.keys() {
                cat.ids.entry(id.clone()).or_insert_with(|| disc("ssx"));
            }
        }
        let mut looks = BTreeMap::new();
        let mut problems = Vec::new();
        for id in self.content.marker_ids() {
            match self.content.marker(&id) {
                Ok(look) => {
                    let d = &look.def;
                    let meta = MarkerMeta {
                        size: d.size,
                        colour: d.colour,
                        hit: hit_shape(d.hit),
                        sound: d.sound.clone(),
                        vertices: look.vertices(),
                        triangles: look.triangles(),
                    };
                    cat.markers.insert(id.clone(), meta);
                    looks.insert(id, Arc::new(look));
                }
                Err(e) => {
                    problems.push(format!("scripts: the marker {id} cannot be placed: {}", clean(&e.to_string())))
                }
            }
        }
        if blind && (self.content.session_policy().is_some() || self.content.generation() != g) {
            return None;
        }
        for line in problems {
            self.note(true, line);
        }
        self.generation = Some(g);
        self.catalogue_session = !blind;
        let (n, m) = (cat.ids.len(), cat.markers.len());
        let cat = Arc::new(cat);
        self.host.set_catalog(cat.clone());
        if let Some(l) = &self.local {
            l.set_catalog(cat.clone());
        }
        self.catalog = Some(cat);
        self.note(
            false,
            format!("scripts: the content catalogue: {n} ids, {m} markers ({:.2} s)", t.elapsed().as_secs_f32()),
        );
        Some(Arc::new(looks))
    }

    fn scripts_of(&self, root: &std::path::Path) -> Result<PackageScripts, String> {
        let src = PackageSource::open(root).map_err(|e| e.to_string())?;
        PackageScripts::from_source(&src, Side::Client, self.host.limits())
    }

    fn start_all(&mut self, only: Option<&BTreeSet<String>>) {
        let roots: Vec<PathBuf> = self.packages.iter().chain(self.dev.iter()).cloned().collect();
        let running = self.host.running();
        for root in roots {
            match self.scripts_of(&root) {
                Ok(p) if p.scripts.is_empty() => {}
                Ok(p) if only.is_some_and(|o| !o.contains(&p.name)) => {}
                Ok(p) if running.contains(&p.name) => {
                    if self.session.contains_key(&p.name) {
                        let line = format!("scripts: {} is held off: the server's resource of its name runs", p.name);
                        self.note(false, line);
                        self.client_names.insert(p.name.clone());
                        self.held_off.insert(p.name);
                    }
                }
                Ok(p) => {
                    let (name, n) = (p.name.clone(), p.scripts.len());
                    self.client_names.insert(name.clone());
                    match self.host.start(p) {
                        Ok(()) => self.note(false, format!("scripts: {name} started ({n} client scripts)")),
                        Err(e) => self.note(true, format!("scripts: {name} did not start: {}", clean(&e))),
                    }
                }
                Err(e) => self.note(true, format!("scripts: {}: {}", clean(&root.to_string_lossy()), clean(&e))),
            }
        }
        self.collect();
    }

    fn session(&mut self, packages: Vec<(String, PathBuf, ContentHash)>, storage: &std::path::Path) {
        let stale: Vec<String> = self
            .session
            .iter()
            .filter(|(n, v)| !packages.iter().any(|(m, _, w)| m == *n && w == *v))
            .map(|(n, _)| n.clone())
            .collect();
        for name in stale {
            self.drain_net();
            self.session.remove(&name);
            self.host.stop(&name, "the server stopped or changed it");
        }
        if let Err(e) = ssx_files::create_dir_all(storage) {
            self.note(true, format!("scripts: the server's storage folder: {}", clean(&e.to_string())));
        }
        for (name, folder, version) in packages {
            if self.session.contains_key(&name) {
                continue;
            }
            let opened = PackageSource::open_as(&folder, Origin::Server)
                .map_err(|e| e.to_string())
                .and_then(|src| PackageScripts::from_source(&src, Side::Client, self.host.limits()));
            let p = match opened {
                Ok(p) if p.name != name => {
                    let line = format!("scripts: the server's {}: its manifest names {}", clean(&name), clean(&p.name));
                    self.note(true, line);
                    continue;
                }
                Ok(p) if p.scripts.is_empty() => continue,
                Ok(p) => p,
                Err(e) => {
                    self.note(true, format!("scripts: the server's {}: {}", clean(&name), clean(&e)));
                    continue;
                }
            };
            if self.host.running().contains(&name) {
                self.drain_net();
                self.host.stop(&name, "the server's resource of its name runs in its place for the session");
                if self.client_names.contains(&name) {
                    self.held_off.insert(name.clone());
                }
            }
            let n = p.scripts.len();
            match self.host.start_with_storage(p, storage) {
                Ok(()) => {
                    self.session.insert(name.clone(), version);
                    self.note(false, format!("scripts: the server's {} started ({n} client scripts)", clean(&name)));
                    if let Err(e) = self.host.acknowledge_start(&name) {
                        self.note(
                            true,
                            format!("scripts: the server's {}: no acknowledgement: {}", clean(&name), clean(&e)),
                        );
                    }
                }
                Err(e) => {
                    self.note(true, format!("scripts: the server's {} did not start: {}", clean(&name), clean(&e)))
                }
            }
        }
        self.release_held_off();
        self.collect();
    }

    #[cfg(feature = "net")]
    fn net(&mut self, messages: Vec<(Option<String>, Body)>) {
        if !self.sees() {
            self.refused = self.refused.saturating_add(u64::try_from(messages.len()).unwrap_or(u64::MAX));
            let line =
                format!("scripts: {} of the server's messages came before its word on the client mods", messages.len());
            return self.note(true, line);
        }
        self.end_local("a network session begins");
        for (package, body) in messages {
            if let Err(e) = self.host.deliver(Peer::Server, package.as_deref(), body) {
                self.refused += 1;
                self.note(true, format!("scripts: the server's message refused: {}", clean(&e)));
            }
            if self.held_session.is_some()
                && self.host.local_player().is_some()
                && let Some((packages, storage)) = self.held_session.take()
            {
                self.session(packages, &storage);
            }
        }
        self.collect();
    }

    fn restart(&mut self, root: &std::path::Path) {
        if !self.clients_started {
            return;
        }
        let p = match self.scripts_of(root) {
            Ok(p) => p,
            Err(e) => {
                return self.note(true, format!("scripts: edit refused, the last good state stays: {}", clean(&e)));
            }
        };
        if self.session.contains_key(&p.name) {
            return self
                .note(false, format!("scripts: {} is held off: the server's resource of its name runs", p.name));
        }
        for (path, src) in &p.modules {
            if let Err(e) = ssx_script::compile::bytecode(path, src.as_bytes()) {
                let line = format!("scripts: edit refused, the last good state stays: {}", clean(&e.0));
                return self.note(true, line);
            }
        }
        let name = p.name.clone();
        let t = std::time::Instant::now();
        match self.host.restart(p) {
            Ok(()) => {
                let s = t.elapsed().as_secs_f32();
                self.note(false, format!("scripts: {name} restarted with its changed scripts in {s:.3} s"));
            }
            Err(e) => self.note(true, format!("scripts: {name} did not start again: {}", clean(&e))),
        }
        self.collect();
    }

    fn collect(&mut self) {
        let lines = outputs(&self.host, &mut self.last_stop, "scripts");
        for (problem, l) in lines {
            self.note(problem, l);
        }
    }

    fn out(&mut self, frame: bool, report: FrameReport, local: Option<LocalOut>) -> Out {
        self.drain_net();
        Out {
            frame,
            report,
            monitor: self.monitor(),
            lines: std::mem::take(&mut self.lines),
            looks: self.looks.take(),
            net: std::mem::take(&mut self.net_out),
            session: self.seen_session(),
            local,
        }
    }

    fn monitor(&self) -> String {
        let client = format!("{}\n{}", self.clients_line(), self.host.monitor().render());
        #[cfg(feature = "net")]
        let client = match self.refused {
            0 => client,
            n => format!("{client}\nthe server's messages refused: {n}"),
        };
        match self.local_monitor() {
            Some(server) => format!("{client}\n{server}"),
            None => client,
        }
    }
}

fn outputs(
    host: &ScriptHost,
    last_stop: &mut Option<ssx_script::host::monitor::StopRecord>,
    tag: &str,
) -> Vec<(bool, String)> {
    let mut lines = Vec::new();
    let last = host.take_last_output().into_iter();
    let running = host.running().into_iter().filter_map(|n| host.take_output(&n).map(|o| (n, o)));
    for (name, out) in last.chain(running).collect::<Vec<_>>() {
        let name = clean(&name);
        for l in out.lines {
            lines.push((false, format!("{tag}: [{name}] {}", clean(&l))));
        }
        if out.dropped > 0 {
            lines.push((true, format!("{tag}: [{name}] {} lines of output dropped", out.dropped)));
        }
    }
    let m = host.monitor();
    let from = last_stop.as_ref().and_then(|l| m.stops.iter().rposition(|s| s == l)).map_or(0, |k| k + 1);
    for s in &m.stops[from.min(m.stops.len())..] {
        let problem = !matches!(s.reason, ssx_script::host::monitor::StopReason::Requested(_));
        lines.push((problem, format!("{tag}: {} stopped: {}", clean(&s.package), clean(&s.reason.to_string()))));
    }
    *last_stop = m.stops.last().cloned();
    lines
}
