use super::*;
use ssx_content::policy::ClientScripts;
use ssx_script::host::outbox::Body;

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

fn package_with(dir: &std::path::Path, name: &str, script: &str) -> PathBuf {
    let root = dir.join(name);
    std::fs::create_dir_all(root.join("client")).unwrap();
    let manifest = format!(
        "[resource]\nname = \"{name}\"\nversion = \"1.0.0\"\napi = 1\n[scripts]\nclient = [\"client/*.luau\"]\n"
    );
    std::fs::write(root.join("resource.toml"), manifest).unwrap();
    std::fs::write(root.join("client/main.luau"), script).unwrap();
    root
}

struct Host {
    tx: SyncSender<Job>,
    rx: Receiver<Out>,
    done: Receiver<()>,
    lines: Vec<String>,
    dir: PathBuf,
}

impl Host {
    fn new(tag: &str, packages: impl FnOnce(&std::path::Path) -> Vec<PathBuf>) -> Host {
        let dir = std::env::temp_dir().join(format!("ssx-script-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let start = Start {
            big_root: dir.join("no-disc"),
            packages: packages(&dir),
            dev: None,
            storage: dir.join("storage"),
            server_packages: Vec::new(),
            server_storage: dir.join("server-storage"),
        };
        let (tx, rx, done) = spawn(start);
        Host { tx, rx, done, lines: Vec::new(), dir }
    }

    fn wait(&mut self) -> Out {
        let out = self.rx.recv_timeout(std::time::Duration::from_secs(60)).expect("the host answers");
        self.lines.extend(out.lines.iter().map(|(_, l)| l.clone()));
        out
    }

    fn word(&mut self, word: Word) -> Out {
        self.tx.send(Job::Clients(word)).unwrap();
        let out = self.wait();
        assert!(!out.frame, "a word's answer is a note");
        out
    }

    fn frame(&mut self, f: GameFrame) -> Out {
        self.tx.send(Job::Frame(Box::new(f), None)).unwrap();
        let out = self.wait();
        assert!(out.frame);
        out
    }

    fn settle(&mut self) -> Out {
        self.frame(GameFrame::default());
        self.frame(GameFrame::default())
    }

    fn count(&self, s: &str) -> usize {
        self.lines.iter().filter(|l| l.contains(s)).count()
    }

    fn end(self) {
        self.tx.send(Job::Shutdown).unwrap();
        self.done.recv_timeout(std::time::Duration::from_secs(10)).expect("the host goes away");
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn draws(out: &Out) -> Option<usize> {
    out.report.orders.draws.as_ref().map(Vec::len)
}

const ALLOWED: ClientScripts = ClientScripts::Allowed;
const PENDING: ClientScripts = ClientScripts::Pending;
const REFUSED: ClientScripts = ClientScripts::Refused;

#[test]
fn a_session_at_the_start_starts_the_client_mods_once_its_server_allows_them() {
    for (tag, policy) in [("connect-allowed", ALLOWED), ("connect-refused", REFUSED)] {
        let mut h = Host::new(tag, |d| vec![package(d)]);
        assert_eq!(draws(&h.frame(GameFrame::default())), Some(0), "no client mod before the game's word");
        let note = h.word(Some((7, PENDING)));
        assert!(note.monitor.contains("waiting for the server's client_scripts policy"), "{}", note.monitor);
        assert_eq!(draws(&h.frame(GameFrame::default())), Some(0), "none starts while the policy is pending");
        let note = h.word(Some((7, policy)));
        let out = h.settle();
        match policy {
            ALLOWED => assert_eq!(draws(&out), Some(1), "started at the server's allowance"),
            _ => {
                assert_eq!(draws(&out), Some(0), "a refusing server: none starts");
                assert!(
                    note.monitor.contains(&format!("off for this session: {}", REFUSED.reason())),
                    "{}",
                    note.monitor
                );
                assert!(!note.monitor.contains("stopped p"), "nothing was stopped: {}", note.monitor);
            }
        }
        h.word(None);
        assert_eq!(draws(&h.settle()), Some(1), "the session's end: they run");
        assert_eq!(h.count("[p] started"), 1, "{tag}: one start: {:#?}", h.lines);
        assert_eq!(h.count("p stopped"), 0, "{tag}: {:#?}", h.lines);
        h.end();
    }
}

#[test]
fn single_player_and_a_join_its_server_allows_or_that_fails_start_them_once() {
    let mut h = Host::new("join-allowed", |d| vec![package(d)]);
    h.word(None);
    assert_eq!(draws(&h.settle()), Some(1));
    assert_eq!(h.count("[p] started"), 1, "single player: one start");
    let note = h.word(Some((1, PENDING)));
    assert!(note.monitor.contains("running, seeing nothing of the session"), "{}", note.monitor);
    assert_eq!(draws(&h.frame(GameFrame::default())), Some(1), "they go on while the policy is read");
    h.word(None);
    h.word(Some((2, PENDING)));
    h.word(Some((2, ALLOWED)));
    assert_eq!(draws(&h.settle()), Some(1));
    h.word(None);
    assert_eq!(draws(&h.settle()), Some(1));
    assert_eq!(h.count("[p] started"), 1, "one start for single player and both joins: {:#?}", h.lines);
    assert_eq!(h.count("p stopped"), 0, "{:#?}", h.lines);
    h.end();
}

#[test]
fn a_refusing_servers_word_stops_them_once_with_its_reason() {
    let mut h = Host::new("join-refused", |d| vec![package(d)]);
    h.tx.send(Job::Clients(None)).unwrap();
    h.tx.send(Job::Frame(Box::default(), None)).unwrap();
    for word in [Some((3, PENDING)), Some((3, REFUSED))] {
        h.tx.send(Job::Clients(word)).unwrap();
    }
    let notes: Vec<Out> = (0..4).map(|_| h.wait()).collect();
    assert!(notes[1].frame && !notes[3].frame);
    let monitor = &notes[3].monitor;
    let stop = monitor.lines().find(|l| l.contains("stopped p at tick")).unwrap_or_default();
    assert!(stop.ends_with(&format!("stopped by the host: {}", REFUSED.reason())), "{monitor}");
    assert!(!monitor.contains(PENDING.reason()), "the pending reason nowhere on the monitor: {monitor}");
    assert!(monitor.contains(&format!("off for this session: {}", REFUSED.reason())), "{monitor}");
    assert_eq!(draws(&h.frame(GameFrame::default())), Some(0), "none runs in the session");
    h.word(None);
    assert_eq!(draws(&h.settle()), Some(1));
    assert_eq!(h.count("[p] started"), 2, "once before the session, once after it: {:#?}", h.lines);
    assert_eq!(h.count("p stopped"), 1, "{:#?}", h.lines);
    assert_eq!(h.count(PENDING.reason()), 0, "{:#?}", h.lines);
    h.end();
}

fn server_resource(dir: &std::path::Path) -> ssx_content::PackageSource {
    let srv = dir.join("srv");
    std::fs::create_dir_all(srv.join("models")).unwrap();
    let manifest = "[resource]\nname = \"srv\"\nversion = \"1.0.0\"\napi = 1\n[add]\n\"model/m\" = \"models/m.gltf\"\n";
    std::fs::write(srv.join("resource.toml"), manifest).unwrap();
    std::fs::write(srv.join("models/m.gltf"), br#"{"asset":{"version":"2.0"},"buffers":[]}"#).unwrap();
    ssx_content::PackageSource::open_as(&srv, Origin::Server).unwrap()
}

fn ride(ms: u64) -> GameFrame {
    use ssx_script::host::game::{InstanceView, LocalState, RiderView, SESSION_KEYS};
    let local = LocalState {
        velocity: [0.0; 3],
        speed: 0.0,
        air: false,
        grind: false,
        trick: None,
        score: 0.0,
        health: 1.0,
        finished: false,
    };
    GameFrame {
        now: std::time::Duration::from_millis(ms),
        riders: vec![
            RiderView { key: 0, name: "Elise".into(), position: [0.0; 3], local: Some(local) },
            RiderView { key: SESSION_KEYS + 3, name: "Rival".into(), position: [1.0, 0.0, 0.0], local: None },
        ],
        instance: Some(InstanceView {
            mode: "race".into(),
            drop: "ssx:drop/x".into(),
            state: "running",
            clock_ms: 0.0,
        }),
        ..GameFrame::default()
    }
}

#[test]
fn a_session_not_seen_shows_its_client_mods_nothing_of_it() {
    let eye = "print('started srv=' .. tostring(getContent('srv:model/m') ~= nil))\n\
               triggerServerEvent('host2:top', root)\n\
               addEventHandler('onClientRender', root, function()\n\
                   print('sees instance=' .. tostring(getInstance() ~= nil) .. ' riders=' .. #getRiders() ..\n\
                         ' srv=' .. tostring(getContent('srv:model/m') ~= nil))\n\
                   triggerServerEvent('host2:frame', root)\n\
               end)\n";
    let mut h = Host::new("blind", |d| vec![package_with(d, "eye", eye)]);
    let content = GameContent::of(&h.dir.join("no-disc"));
    let sent = |out: &Out| {
        out.net.iter().filter(|m| matches!(&m.body, Body::Event { name, .. } if name == "host2:frame")).count()
    };
    h.word(None);
    h.frame(ride(0));
    let out = h.frame(ride(1));
    assert_eq!(h.count("[eye] sees instance=true riders=1 srv=false"), 1, "{:#?}", h.lines);
    assert!(out.net.is_empty() && out.session.is_none(), "no session: nothing to a server");
    let mount = content.mount_session(ssx_content::policy::ClientMods::Presentation, vec![server_resource(&h.dir)]);
    h.word(Some((5, PENDING)));
    let out = h.frame(ride(2));
    assert_eq!(h.count("[eye] sees instance=false riders=0 srv=false"), 1, "{:#?}", h.lines);
    assert!(out.net.is_empty() && out.session.is_none());
    #[cfg(feature = "net")]
    h.tx.send(Job::Net(vec![(None, Body::Hello { player: ssx_script::value::ElementId(99) })])).unwrap();
    h.tx.send(Job::Session { packages: Vec::new(), storage: h.dir.join("srv-storage") }).unwrap();
    h.frame(ride(3));
    #[cfg(feature = "net")]
    assert_eq!(h.count("1 of the server's messages came before its word on the client mods"), 1, "{:#?}", h.lines);
    assert_eq!(h.count("the server's client scripts came before its word on the client mods"), 1);
    h.word(Some((5, ALLOWED)));
    let out = h.frame(ride(4));
    assert_eq!(h.count("[eye] sees instance=true riders=2 srv=true"), 1, "{:#?}", h.lines);
    assert_eq!((out.session, out.net.len(), sent(&out)), (Some(5), 1, 1), "this frame's message alone, to session 5");
    content.end_session(mount);
    h.word(None);
    let out = h.frame(ride(5));
    assert_eq!(h.count("[eye] sees instance=true riders=1 srv=false"), 2, "{:#?}", h.lines);
    assert!(out.net.is_empty() && out.session.is_none());
    assert_eq!(h.count("[eye] started"), 1, "{:#?}", h.lines);
    assert_eq!(h.count("2 messages of the client mods to a server dropped"), 2, "{:#?}", h.lines);
    h.end();
}

#[cfg(feature = "net")]
#[test]
fn a_sessions_end_forgets_it_before_the_client_mods_start_again() {
    use ssx_script::host::ROOT;
    use ssx_script::value::ElementId;
    let mut h = Host::new("session-end", |d| {
        vec![package_with(d, "lp", "print('started player=' .. tostring(localPlayer ~= nil))")]
    });
    h.word(Some((9, PENDING)));
    h.word(Some((9, REFUSED)));
    let me = ElementId(500);
    let created = Body::Created { element: me, kind: "player".into(), name: String::new(), parent: ROOT };
    h.tx.send(Job::Net(vec![(None, created), (None, Body::Hello { player: me })])).unwrap();
    h.word(Some((9, ALLOWED)));
    h.frame(GameFrame::default());
    assert_eq!(h.count("[lp] started player=true"), 1, "the server named the player: {:#?}", h.lines);
    h.word(Some((9, REFUSED)));
    h.word(None);
    h.frame(GameFrame::default());
    assert_eq!(h.count("[lp] started player=false"), 1, "started after the session forgot it: {:#?}", h.lines);
    assert_eq!(h.count("[lp] started"), 2);
    h.end();
}

#[test]
fn their_start_after_a_session_waits_for_its_content_to_be_taken_down() {
    let mut h = Host::new("lingering", |d| {
        vec![package_with(d, "eye", "print('started srv=' .. tostring(getContent('srv:model/m') ~= nil))")]
    });
    let content = GameContent::of(&h.dir.join("no-disc"));
    h.word(Some((4, PENDING)));
    let mount = content.mount_session(ssx_content::policy::ClientMods::Presentation, vec![server_resource(&h.dir)]);
    h.word(Some((4, REFUSED)));
    h.word(None);
    for _ in 0..2 {
        let out = h.frame(GameFrame::default());
        assert!(out.monitor.contains("waiting for the session's content to be taken down"), "{}", out.monitor);
    }
    assert_eq!(h.count("[eye] started"), 0, "{:#?}", h.lines);
    assert_eq!(h.count("wait for the session's content to be taken down"), 1, "said once: {:#?}", h.lines);
    content.end_session(mount);
    h.frame(GameFrame::default());
    assert_eq!(h.count("[eye] started srv=false"), 1, "{:#?}", h.lines);
    h.end();
}

#[test]
fn a_client_mods_timer_counts_from_its_start() {
    let timer = "setTimer(function() print('due') end, 10000, 1)";
    let mut h = Host::new("timer", |d| vec![package_with(d, "t", timer)]);
    let at = |s: f32| GameFrame { now: std::time::Duration::from_secs_f32(s), ..GameFrame::default() };
    h.frame(at(1.0));
    h.word(None);
    h.frame(at(60.0));
    h.frame(at(69.5));
    assert_eq!(h.count("[t] due"), 0, "{:#?}", h.lines);
    h.frame(at(70.5));
    assert_eq!(h.count("[t] due"), 1, "{:#?}", h.lines);
    h.end();
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
    tx.send(Job::Clients(None)).unwrap();
    assert!(!wait().frame);
    tx.send(Job::Frame(Box::default(), None)).unwrap();
    let note = wait();
    assert!(note.frame);
    tx.send(Job::Local { package: "gm".into(), player: "Player".into() }).unwrap();
    tx.send(frame(vec![ServerEvent::RoundStart { instance: 1, round: 1, drop: "gm:map/a".into() }], 33)).unwrap();
    let out = wait();
    let lines: Vec<&str> = note.lines.iter().chain(&out.lines).map(|(_, l)| l.as_str()).collect();
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
