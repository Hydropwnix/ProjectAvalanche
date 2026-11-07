use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

use ssx_assets::content::GameContent;
use ssx_assets::content::markers::MarkerLook;
use ssx_content::marker::Hit;
use ssx_content::package::Origin;
use ssx_content::{ContentHash, ContentId, Kind, Layer, PackageSource};
use ssx_script::host::game::{Catalog, ContentMeta, FrameReport, GameFrame, MarkerHitShape, MarkerMeta};
#[cfg(feature = "net")]
use ssx_script::host::outbox::{Body, Peer};
use ssx_script::host::outbox::{Destination, Message};
use ssx_script::host::package::PackageScripts;
use ssx_script::host::server::ServerFrame;
use ssx_script::host::{HostConfig, ScriptHost, Side};

mod local;
#[cfg(test)]
pub(super) use local::local_client_is_the_first_admitted;
pub(super) use local::{LOCAL_CLIENT, LocalOut};

const STACK: usize = 8 << 20;
const QUEUE: usize = 8;
const MAX_LINES: usize = 64;
const MAX_LINE: usize = 300;

pub(super) enum Job {
    Frame(Box<GameFrame>, Option<Box<ServerFrame>>),
    Restart(PathBuf),
    Suspend(String),
    Resume,
    Session {
        packages: Vec<(String, PathBuf, ContentHash)>,
        storage: PathBuf,
    },
    EndSession,
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

struct Worker {
    host: ScriptHost,
    content: Arc<GameContent>,
    packages: Vec<PathBuf>,
    dev: Option<PathBuf>,
    generation: Option<u64>,
    lines: Vec<(bool, String)>,
    last_stop: Option<ssx_script::host::monitor::StopRecord>,
    suspended: bool,
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
        suspended: false,
        session: BTreeMap::new(),
        held_session: None,
        #[cfg(feature = "net")]
        refused: 0,
        local: None,
        server_packages: start.server_packages,
        server_storage: start.server_storage,
        catalog: None,
    };
    let looks = w.catalogue();
    w.start_all();
    let mut pending_looks = Some(looks);
    while let Ok(job) = jobs.recv() {
        match job {
            Job::Frame(f, server) => {
                let local = server.and_then(|sf| w.local_before(*sf));
                let report = match w.host.frame(*f) {
                    Ok(r) => r,
                    Err(e) => {
                        w.note(true, format!("scripts: the frame was refused: {e}"));
                        FrameReport::default()
                    }
                };
                if w.generation != Some(w.content.generation()) {
                    pending_looks = Some(w.catalogue());
                }
                w.local_after();
                w.collect();
                let out = Out {
                    frame: true,
                    report,
                    monitor: w.monitor(),
                    lines: std::mem::take(&mut w.lines),
                    looks: pending_looks.take(),
                    net: w.outbound(),
                    local,
                };
                if outs.send(out).is_err() {
                    return;
                }
            }
            Job::Restart(root) => w.restart(&root),
            Job::Suspend(why) => {
                for name in w.host.running().into_iter().filter(|n| !w.session.contains_key(n)) {
                    w.host.stop(&name, &why);
                }
                w.suspended = true;
                w.collect();
                let out = Out {
                    frame: false,
                    report: FrameReport::default(),
                    monitor: w.monitor(),
                    lines: std::mem::take(&mut w.lines),
                    looks: None,
                    net: w.outbound(),
                    local: None,
                };
                if outs.send(out).is_err() {
                    return;
                }
            }
            Job::Resume => {
                w.suspended = false;
                w.start_all();
            }
            Job::Session { packages, storage } if w.host.local_player().is_none() || w.local.is_some() => {
                w.end_local("a network session begins");
                w.note(false, "scripts: the server's client scripts wait for the server to name this player".into());
                w.held_session = Some((packages, storage));
            }
            Job::Session { packages, storage } => w.session(packages, &storage),
            #[cfg(feature = "net")]
            Job::Net(messages) => w.net(messages),
            Job::Local { package, player } => w.start_local(&package, &player),
            Job::EndLocal(why) => w.end_local(&why),
            Job::EndSession => {
                for name in std::mem::take(&mut w.session).into_keys() {
                    w.host.stop(&name, "the session ended");
                }
                w.held_session = None;
                #[cfg(feature = "net")]
                {
                    w.refused = 0;
                }
                w.host.leave_session();
                w.collect();
                if !w.suspended {
                    w.start_all();
                }
            }
            Job::Shutdown => {
                w.end_local("the game closes");
                return;
            }
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

    fn catalogue(&mut self) -> Arc<BTreeMap<ContentId, Arc<MarkerLook>>> {
        let t = std::time::Instant::now();
        self.generation = Some(self.content.generation());
        let mut cat = Catalog::default();
        self.content.with_registry(|r| {
            for kind in Kind::ALL {
                for id in r.list(kind) {
                    if let Some(w) = r.winner(&id) {
                        cat.ids.insert(id, ContentMeta { source: w.name.to_string(), layer: layer_name(w.layer) });
                    }
                }
            }
        });
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
                    self.note(true, format!("scripts: the marker {id} cannot be placed: {}", clean(&e.to_string())))
                }
            }
        }
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
        Arc::new(looks)
    }

    fn scripts_of(&self, root: &std::path::Path) -> Result<PackageScripts, String> {
        let src = PackageSource::open(root).map_err(|e| e.to_string())?;
        PackageScripts::from_source(&src, Side::Client, self.host.limits())
    }

    fn start_all(&mut self) {
        let roots: Vec<PathBuf> = self.packages.iter().chain(self.dev.iter()).cloned().collect();
        let running = self.host.running();
        for root in roots {
            match self.scripts_of(&root) {
                Ok(p) if p.scripts.is_empty() => {}
                Ok(p) if running.contains(&p.name) => {
                    if self.session.contains_key(&p.name) {
                        let line = format!("scripts: {} is held off: the server's resource of its name runs", p.name);
                        self.note(false, line);
                    }
                }
                Ok(p) => {
                    let (name, n) = (p.name.clone(), p.scripts.len());
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
                self.host.stop(&name, "the server's resource of its name runs in its place for the session");
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
        self.collect();
    }

    #[cfg(feature = "net")]
    fn net(&mut self, messages: Vec<(Option<String>, Body)>) {
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

    fn outbound(&mut self) -> Vec<Message> {
        self.host.drain(usize::MAX).messages.into_iter().filter(|m| m.to == Destination::Server).collect()
    }

    fn restart(&mut self, root: &std::path::Path) {
        if self.suspended {
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

    fn monitor(&self) -> String {
        let client = self.host.monitor().render();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn package(dir: &std::path::Path) -> PathBuf {
        let root = dir.join("p");
        std::fs::create_dir_all(root.join("client")).unwrap();
        std::fs::write(
            root.join("resource.toml"),
            "[resource]\nname = \"p\"\nversion = \"1.0.0\"\napi = 1\n[scripts]\nclient = [\"client/*.luau\"]\n",
        )
        .unwrap();
        std::fs::write(
            root.join("client/main.luau"),
            "print('started') addEventHandler('onClientRender', root, function() draw.rect(0, 0, 1, 1, 0) end)",
        )
        .unwrap();
        root
    }

    #[test]
    fn a_session_without_a_policy_runs_no_client_script() {
        let dir = std::env::temp_dir().join(format!("ssx-script-session-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let start = Start {
            big_root: dir.join("no-disc"),
            packages: vec![package(&dir)],
            dev: None,
            storage: dir.join("storage"),
            server_packages: Vec::new(),
            server_storage: dir.join("server-storage"),
        };
        let (tx, rx, done) = spawn(start);
        let frame = |tx: &SyncSender<Job>| tx.send(Job::Frame(Box::default(), None)).unwrap();
        let wait = || rx.recv_timeout(std::time::Duration::from_secs(60)).expect("the host answers");
        frame(&tx);
        let out = wait();
        assert!(out.lines.iter().any(|(_, l)| l.contains("[p] started")), "{:?}", out.lines);
        assert_eq!(out.report.orders.draws.map(|d| d.len()), Some(1));
        assert!(!ssx_content::policy::ClientScripts::SESSION_DEFAULT.runs());
        tx.send(Job::Suspend(ssx_content::policy::ClientScripts::SESSION_DEFAULT.reason().into())).unwrap();
        let note = wait();
        assert!(!note.frame && note.report.orders.draws.is_none());
        assert!(
            note.monitor.contains("stopped p") && note.monitor.contains("client_scripts policy"),
            "{}",
            note.monitor
        );
        assert!(note.lines.iter().any(|(_, l)| l.contains("p stopped")), "{:?}", note.lines);
        frame(&tx);
        let out = wait();
        assert!(out.frame);
        assert_eq!(out.report.orders.draws.map(|d| d.len()), Some(0), "no script draws in the session");
        tx.send(Job::Resume).unwrap();
        frame(&tx);
        let out = wait();
        assert_eq!(out.report.orders.draws.map(|d| d.len()), Some(1), "the session ended: the scripts run again");
        tx.send(Job::Shutdown).unwrap();
        done.recv_timeout(std::time::Duration::from_secs(10)).expect("the host goes away");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn gamemode(dir: &std::path::Path) -> PathBuf {
        let root = dir.join("gm");
        std::fs::create_dir_all(root.join("client")).unwrap();
        std::fs::create_dir_all(root.join("server")).unwrap();
        let manifest = "[resource]\nname = \"gm\"\nversion = \"1.0.0\"\napi = 1\n[scripts]\n\
                        client = [\"client/*.luau\"]\nserver = [\"server/*.luau\"]\n";
        std::fs::write(root.join("resource.toml"), manifest).unwrap();
        let client = "print('client sees ' .. tostring(localPlayer ~= nil))\n\
                      if localPlayer then triggerServerEvent('gm:hello', localPlayer, 1) end\n";
        std::fs::write(root.join("client/main.luau"), client).unwrap();
        let server = r#"
            addEvent("gm:hello", true)
            addEventHandler("onPlayerJoin", root, function() print("join " .. getPlayerName(source)) end)
            addEventHandler("gm:hello", root, function(n) print("hello " .. n .. " from " .. getPlayerName(client)) end)
            addEventHandler("onRoundStart", root, function(round, drop) print("round " .. round .. " " .. drop) end)
            addEventHandler("onRiderCheckpoint", root, function(cp, split, clock)
                print("gate " .. split .. " " .. clock)
                endRound(getPlayerInstance(getRiderPlayer(source)), "one gate is enough")
            end)
            addEventHandler("onPlayerQuit", root, function(why) print("quit " .. why) end)
        "#;
        std::fs::write(root.join("server/main.luau"), server).unwrap();
        root
    }

    #[test]
    fn a_gamemodes_server_scripts_run_in_the_local_host() {
        use ssx_script::host::server::{EndRound, InstanceFrame, PlayerView, ServerEvent};
        let dir = std::env::temp_dir().join(format!("ssx-script-local-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let gm = gamemode(&dir);
        let start = Start {
            big_root: dir.join("no-disc"),
            packages: vec![gm.clone()],
            dev: None,
            storage: dir.join("storage"),
            server_packages: vec![gm],
            server_storage: dir.join("server-storage"),
        };
        let (tx, rx, done) = spawn(start);
        let wait = || rx.recv_timeout(std::time::Duration::from_secs(60)).expect("the host answers");
        let frame = |events: Vec<ServerEvent>, ms: u64| {
            let now = std::time::Duration::from_millis(ms);
            let instance = InstanceFrame {
                id: 1,
                mode: "gm:gamemode/race".into(),
                drop: "gm:map/a".into(),
                state: "running",
                round: 1,
                clock_ms: 0.0,
                standings: Vec::new(),
            };
            let player = PlayerView {
                client: LOCAL_CLIENT,
                name: "Player".into(),
                instance: Some(1),
                rider: Some(0),
                progress: "ready",
                status: "riding",
                ping_ms: 0,
                position: Some([0.0; 3]),
                velocity: None,
                run: None,
            };
            let sf = ServerFrame { now, instances: vec![instance], players: vec![player], events };
            Job::Frame(Box::new(GameFrame { now, ..GameFrame::default() }), Some(Box::new(sf)))
        };
        tx.send(Job::Local { package: "gm".into(), player: "Player".into() }).unwrap();
        tx.send(frame(vec![ServerEvent::RoundStart { instance: 1, round: 1, drop: "gm:map/a".into() }], 33)).unwrap();
        let out = wait();
        let lines: Vec<&str> = out.lines.iter().map(|(_, l)| l.as_str()).collect();
        for want in [
            "scripts: [gm] client sees false",
            "scripts: local server: [gm] join Player",
            "scripts: [gm] client sees true",
            "scripts: local server: [gm] hello 1 from Player",
            "scripts: local server: [gm] round 1 gm:map/a",
        ] {
            assert!(lines.contains(&want), "{want}: {lines:#?}");
        }
        assert_eq!(out.local.as_ref().and_then(|l| l.running.as_deref()), Some("gm"));
        assert!(out.monitor.contains("local server (gm)"), "{}", out.monitor);
        tx.send(frame(vec![ServerEvent::Checkpoint { client: LOCAL_CLIENT, split: 0, clock_ms: 900 }], 66)).unwrap();
        let out = wait();
        let end = EndRound { instance: 1, round: 1, reason: "gm: one gate is enough".into() };
        assert_eq!(out.local.map(|l| l.end_round), Some(vec![end]));
        tx.send(Job::EndLocal("the level ended".into())).unwrap();
        tx.send(Job::Frame(Box::default(), None)).unwrap();
        let out = wait();
        let lines: Vec<&str> = out.lines.iter().map(|(_, l)| l.as_str()).collect();
        assert!(lines.contains(&"scripts: local server: [gm] quit the level ended"), "{lines:#?}");
        assert!(lines.contains(&"scripts: [gm] client sees false"), "{lines:#?}");
        assert!(out.local.is_none(), "the host is gone");
        tx.send(Job::Shutdown).unwrap();
        done.recv_timeout(std::time::Duration::from_secs(10)).expect("the host goes away");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
