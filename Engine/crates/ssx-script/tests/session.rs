use std::time::Duration;

use ssx_content::manifest::{Files, Scripts};
use ssx_script::Limits;
use ssx_script::host::limits::Budgets;
use ssx_script::host::loopback::Loopback;
use ssx_script::host::outbox::{Body, Peer};
use ssx_script::host::package::PackageScripts;
use ssx_script::host::{ClientId, CommandOrigin, HostConfig, ScriptHost, Side};
use ssx_script::value::{ElementId, NetValue, ValueLimits, encode_list};

fn relaxed(b: &mut Budgets, s: u64) {
    (b.call, b.call_backstop) = (Limits::MAX_CHECKPOINTS, Duration::from_secs(s));
    (b.chain, b.chain_backstop) = (u64::MAX, Duration::from_secs(2 * s));
    (b.tick, b.tick_checkpoints, b.tick_backstop) = (Duration::from_secs(2 * s), u64::MAX, Duration::from_secs(2 * s));
}

fn config(side: Side) -> HostConfig {
    let mut c = HostConfig::new(side);
    relaxed(&mut c.limits.budgets, 2);
    c
}

fn pkg(name: &str, side: Side, src: &str) -> PackageScripts {
    let path = if side == Side::Server { "server/main.luau" } else { "client/main.luau" };
    let scripts =
        Scripts { client: vec!["client/*.luau".into()], server: vec!["server/*.luau".into()], shared: vec![] };
    let files = [(path.to_string(), src.to_string())];
    PackageScripts::from_memory(
        name,
        side,
        &scripts,
        &Files::default(),
        toml::Table::new(),
        &files,
        &config(side).limits,
    )
    .unwrap()
}

fn out(h: &ScriptHost, name: &str) -> Vec<String> {
    h.take_output(name).map(|o| o.lines).unwrap_or_default()
}

fn session() -> Loopback {
    Loopback::new(config(Side::Server)).unwrap()
}

fn client(s: &Loopback, id: ClientId) -> &ScriptHost {
    s.client(id).unwrap()
}

#[test]
fn remote_events_need_the_declaration_and_the_senders_own_source() {
    let mut s = session();
    s.server
        .start(pkg(
            "game",
            Side::Server,
            "addEvent('hello', true) addEvent('quiet')\n\
             addEventHandler('hello', root, function(n) print('hello', n, client == source, getElementType(client)) end)\n\
             addEventHandler('quiet', root, function() print('quiet ran') end)",
        ))
        .unwrap();
    let (a, _) = s.join(config(Side::Client)).unwrap();
    client(&s, a)
        .start(pkg(
            "game",
            Side::Client,
            "triggerServerEvent('hello', localPlayer, 1)\n\
             print(pcall(triggerServerEvent, 'hello', resourceRoot, 2))\n\
             triggerServerEvent('hello', root, 3)\n\
             triggerServerEvent('quiet', localPlayer)",
        ))
        .unwrap();
    client(&s, a).start(pkg("other", Side::Client, "triggerServerEvent('hello', localPlayer, 4)")).unwrap();
    let local = out(client(&s, a), "game");
    assert!(local[0].contains("this client's own"), "{local:?}");
    s.step(Duration::from_millis(50));
    assert_eq!(out(&s.server, "game"), ["hello\t1\ttrue\tplayer"], "only the event from its own player ran");
    let refused = s.refused.join("\n");
    assert!(refused.contains("not the sender's to name as a source"), "{refused}");
    assert!(refused.contains("did not declare quiet remote"), "{refused}");
    assert!(refused.contains("other does not run here"), "{refused}");
    assert_eq!(s.server.monitor().refused_inbound, 3);
}

#[test]
fn a_forged_source_and_forged_values_are_refused_at_the_server() {
    let mut s = session();
    s.server.start(pkg("game", Side::Server, "addEvent('hello', true) addEventHandler('hello', root, print)")).unwrap();
    let (a, _) = s.join(config(Side::Client)).unwrap();
    let (_, b_player) = s.join(config(Side::Client)).unwrap();
    let args = encode_list(&[NetValue::Number(1.0)], &ValueLimits::REMOTE).unwrap();
    let forged = Body::Event { name: "hello".into(), source: b_player, args: args.clone() };
    assert!(s.server.deliver(Peer::Client(a), Some("game"), forged).unwrap_err().contains("not the sender's"));
    let bad =
        Body::Event { name: "hello".into(), source: ElementId(1), args: vec![1, 3, 0, 0, 0, 0, 0, 0, 0xF8, 0x7F, 9] };
    assert!(s.server.deliver(Peer::Client(a), Some("game"), bad).unwrap_err().contains("arguments"));
    let wrong_side =
        Body::Created { element: ElementId(9), kind: "x".into(), name: String::new(), parent: ElementId(1) };
    assert!(s.server.deliver(Peer::Client(a), None, wrong_side).unwrap_err().contains("does not take"));
    assert!(
        s.server.deliver(Peer::Server, Some("game"), Body::LatentCancel { transfer: 1 }).is_err(),
        "the server hears clients only"
    );
}

#[test]
fn client_events_reach_only_the_players_named() {
    let mut s = session();
    s.server
        .start(pkg(
            "game",
            Side::Server,
            "addCommandHandler('ping', function(p, cmd, who) triggerClientEvent(p, 'pong', root, who) end)",
        ))
        .unwrap();
    let (a, _) = s.join(config(Side::Client)).unwrap();
    let (b, _) = s.join(config(Side::Client)).unwrap();
    for id in [a, b] {
        client(&s, id)
            .start(pkg(
                "game",
                Side::Client,
                "addEvent('pong', true) addEventHandler('pong', root, function(w) print('pong', w) end)",
            ))
            .unwrap();
    }
    s.server.execute_command(CommandOrigin::Client(a), "ping me").unwrap();
    s.step(Duration::from_millis(50));
    assert_eq!(out(client(&s, a), "game"), ["pong\tme"]);
    assert!(out(client(&s, b), "game").is_empty(), "b was not named");
}

#[test]
fn a_client_writes_only_opened_keys_on_its_own_elements() {
    let mut s = session();
    s.server
        .start(pkg(
            "game",
            Side::Server,
            "addCommandHandler('open', function(p) setElementDataClientWritable(p, 'ready', true) setElementData(p, 'score', 10) end)\n\
             addEventHandler('onElementDataChange', root, function(key, old, new)\n\
               if client then print('change', key, new, client == source) end\n\
               if key == 'ready' and new == 'never' then cancelEvent() end end)",
        ))
        .unwrap();
    let (a, _) = s.join(config(Side::Client)).unwrap();
    let (b, _) = s.join(config(Side::Client)).unwrap();
    for id in [a, b] {
        s.server.execute_command(CommandOrigin::Client(id), "open").unwrap();
    }
    s.step(Duration::from_millis(50));
    client(&s, a).start(pkg(
        "game",
        Side::Client,
        "setElementData(localPlayer, 'ready', true)\n\
         setElementData(localPlayer, 'score', 99999)\n\
         for _, p in ipairs(getElementsByType('player')) do if p ~= localPlayer then setElementData(p, 'ready', 'forged') end end\n\
         addCommandHandler('never', function() setElementData(localPlayer, 'ready', 'never') end)",
    ))
    .unwrap();
    s.step(Duration::from_millis(50));
    client(&s, a).execute_command(CommandOrigin::Console, "never").unwrap();
    s.step(Duration::from_millis(50));
    s.step(Duration::from_millis(50));
    let server = out(&s.server, "game");
    assert_eq!(
        server,
        ["change\tready\ttrue\ttrue", "change\tready\tnever\ttrue"],
        "only the opened key on its own player"
    );
    let refused = s.refused.join("\n");
    assert!(refused.contains("\"score\" is not open"), "{refused}");
    assert!(refused.matches("is not open to this client").count() >= 2, "the other player's key too: {refused}");
    assert!(refused.contains("cancelled the write"), "{refused}");
    let c = client(&s, a);
    c.start(pkg(
        "check",
        Side::Client,
        "print('now', getElementData(localPlayer, 'score'), getElementData(localPlayer, 'ready'))",
    ))
    .unwrap();
    assert_eq!(out(c, "check"), ["now\t10\ttrue"], "the server's values were sent back");
}

#[test]
fn sync_modes_decide_who_sees_a_key() {
    let mut s = session();
    s.server
        .start(pkg(
            "game",
            Side::Server,
            "box = createElement('box', 'box')\n\
             setElementData(box, 'all', 1) setElementData(box, 'mine', 2, 'local') setElementData(box, 'some', 3, 'subscribers')\n\
             addCommandHandler('sub', function(p) addElementDataSubscriber(box, 'some', p) end)",
        ))
        .unwrap();
    let (a, _) = s.join(config(Side::Client)).unwrap();
    let (b, _) = s.join(config(Side::Client)).unwrap();
    s.server.execute_command(CommandOrigin::Client(a), "sub").unwrap();
    s.step(Duration::from_millis(50));
    let probe = "local box = getElementByID('box') print(getElementData(box, 'all'), getElementData(box, 'mine'), getElementData(box, 'some'))";
    client(&s, a).start(pkg("probe", Side::Client, probe)).unwrap();
    client(&s, b).start(pkg("probe", Side::Client, probe)).unwrap();
    assert_eq!(out(client(&s, a), "probe"), ["1\tnil\t3"]);
    assert_eq!(out(client(&s, b), "probe"), ["1\tnil\tnil"]);
}

#[test]
fn latent_events_arrive_at_their_rate_and_can_be_cancelled() {
    let mut s = session();
    s.server
        .start(pkg(
            "game",
            Side::Server,
            "addCommandHandler('send', function(p) big = triggerLatentClientEvent(p, 'big', 100000, root, string.rep('x', 60000), string.rep('y', 60000), 7)\n\
             other = triggerLatentClientEvent(p, 'big', 1000, root, string.rep('n', 5000)) end)\n\
             addCommandHandler('status', function() local st = getLatentEventStatus(big) print(st and st.percent, cancelLatentEvent(other), #getLatentEventHandles()) end)",
        ))
        .unwrap();
    let (a, _) = s.join(config(Side::Client)).unwrap();
    client(&s, a)
        .start(pkg(
            "game",
            Side::Client,
            "addEvent('big', true) addEventHandler('big', root, function(x, y, n) print('got', #x, #y, n) end)",
        ))
        .unwrap();
    s.server.execute_command(CommandOrigin::Client(a), "send").unwrap();
    s.step(Duration::from_millis(500));
    s.server.execute_command(CommandOrigin::Console, "status").unwrap();
    let status = out(&s.server, "game");
    let pct: f64 = status[0].split('\t').next().unwrap().parse().unwrap();
    assert!(pct > 20.0 && pct < 60.0, "about half a second of 120 KB at 100 kB/s: {status:?}");
    assert!(status[0].ends_with("\ttrue\t1"), "the slow one cancelled: {status:?}");
    assert!(out(client(&s, a), "game").is_empty(), "not whole yet");
    for _ in 0..12 {
        s.step(Duration::from_millis(100));
    }
    assert_eq!(out(client(&s, a), "game"), ["got\t60000\t60000\t7"]);
}

#[test]
fn a_receiver_holds_latent_senders_to_its_bounds() {
    let s = session();
    s.server.start(pkg("game", Side::Server, "addEvent('big', true)")).unwrap();
    s.server.connect_client(ClientId(1)).unwrap();
    let peer = Peer::Client(ClientId(1));
    let start = |t: u64, total: u32| Body::LatentStart { transfer: t, name: "big".into(), source: ElementId(1), total };
    assert!(s.server.deliver(peer, Some("game"), start(1, 17 << 20)).unwrap_err().contains("at most"));
    s.server.deliver(peer, Some("game"), start(2, 100)).unwrap();
    assert!(s.server.deliver(peer, Some("game"), start(2, 100)).unwrap_err().contains("already open"));
    let chunk = |offset: u32| Body::LatentChunk { transfer: 2, offset, bytes: vec![0; 10] };
    assert!(s.server.deliver(peer, None, chunk(50)).unwrap_err().contains("out of order"));
    assert!(s.server.deliver(peer, None, chunk(0)).unwrap_err().contains("not open"), "an out-of-order chunk drops it");
    for t in 10..26 {
        s.server.deliver(peer, Some("game"), start(t, 1 << 20)).unwrap();
    }
    assert!(s.server.deliver(peer, Some("game"), start(99, 10)).unwrap_err().contains("open from this peer"));
}

#[test]
fn a_stopped_server_package_vanishes_from_the_clients() {
    let mut s = session();
    s.server
        .start(pkg("game", Side::Server, "for i = 1, 10 do createElement('thing') end setElementData(root, 'k', 1)"))
        .unwrap();
    let (a, _) = s.join(config(Side::Client)).unwrap();
    let count = "print(#getElementsByType('thing'), getElementData(root, 'k'))";
    client(&s, a).start(pkg("probe", Side::Client, count)).unwrap();
    assert_eq!(out(client(&s, a), "probe"), ["10\t1"]);
    s.server.stop("game", "test");
    s.step(Duration::from_millis(50));
    client(&s, a).start(pkg("probe2", Side::Client, count)).unwrap();
    assert_eq!(out(client(&s, a), "probe2"), ["0\tnil"]);
}

#[test]
fn replication_over_its_bound_resyncs_from_a_snapshot() {
    let mut cfg = config(Side::Server);
    cfg.limits.outbox_messages = 64;
    let mut s = Loopback::new(cfg).unwrap();
    let (a, _) = s.join(config(Side::Client)).unwrap();
    s.server.start(pkg("game", Side::Server, "for i = 1, 500 do createElement('thing') end")).unwrap();
    s.step(Duration::from_millis(50));
    client(&s, a).start(pkg("probe", Side::Client, "print(#getElementsByType('thing'))")).unwrap();
    assert_eq!(out(client(&s, a), "probe"), ["500"], "the snapshot after the dropped replication has them all");
    assert!(s.refused.is_empty(), "{:?}", s.refused);
}

#[test]
fn a_hostile_server_cannot_flood_a_clients_mirror() {
    let mut cfg = config(Side::Client);
    cfg.limits.mirror_elements = 100;
    let c = ScriptHost::new(cfg).unwrap();
    for i in 0..100u64 {
        let body =
            Body::Created { element: ElementId(100 + i), kind: "x".into(), name: String::new(), parent: ElementId(1) };
        c.deliver(Peer::Server, None, body).unwrap();
    }
    let one_more =
        Body::Created { element: ElementId(999), kind: "x".into(), name: String::new(), parent: ElementId(1) };
    assert!(c.deliver(Peer::Server, None, one_more).unwrap_err().contains("at most 100"));
    let local =
        Body::Created { element: ElementId(1 << 52), kind: "x".into(), name: String::new(), parent: ElementId(1) };
    assert!(c.deliver(Peer::Server, None, local).is_err(), "the client's own id range");
    let below_local =
        Body::Created { element: ElementId(6000), kind: "x".into(), name: String::new(), parent: ElementId(1 << 52) };
    assert!(c.deliver(Peer::Server, None, below_local).is_err(), "below the client's own element");
    let bad_kind =
        Body::Created { element: ElementId(5000), kind: "\u{1b}[2J".into(), name: String::new(), parent: ElementId(1) };
    assert!(c.deliver(Peer::Server, None, bad_kind).is_err());
    let write = Body::DataWrite { element: ElementId(100), key: "k".into(), value: None };
    assert!(c.deliver(Peer::Server, None, write).unwrap_err().contains("does not take"));
}
