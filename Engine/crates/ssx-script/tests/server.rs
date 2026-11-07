use std::sync::Arc;
use std::time::Duration;

use ssx_content::ContentId;
use ssx_content::manifest::{Files, Scripts};
use ssx_script::Limits;
use ssx_script::host::game::{
    Catalog, ContentMeta, GameFrame, LocalState, MarkerHitShape, MarkerMeta, RiderView, SESSION_KEYS,
};
use ssx_script::host::limits::Budgets;
use ssx_script::host::loopback::Loopback;
use ssx_script::host::package::PackageScripts;
use ssx_script::host::server::joins::JoinVerdict;
use ssx_script::host::server::{EndRound, InstanceFrame, PlayerView, RunView, ServerEvent, ServerFrame};
use ssx_script::host::{ClientId, HostConfig, ScriptHost, Side};

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
    let limits = config(side).limits;
    PackageScripts::from_memory(name, side, &scripts, &Files::default(), toml::Table::new(), &files, &limits).unwrap()
}

fn out(h: &ScriptHost, name: &str) -> Vec<String> {
    h.take_output(name).map(|o| o.lines).unwrap_or_default()
}

fn id(s: &str) -> ContentId {
    s.parse().unwrap()
}

fn catalog() -> Arc<Catalog> {
    let mut c = Catalog::default();
    let meta = MarkerMeta {
        size: 4.0,
        colour: 0xFF00_FF00,
        hit: MarkerHitShape::Sphere,
        sound: None,
        vertices: 8,
        triangles: 4,
    };
    c.markers.insert(id("ava:marker/ring"), meta);
    let source = ContentMeta { source: "ssx".into(), layer: "L0 base" };
    c.ids.insert(id("ava:marker/ring"), source.clone());
    c.ids.insert(id("ssx:sound/hud/gate"), source);
    Arc::new(c)
}

fn local() -> LocalState {
    LocalState {
        velocity: [0.0; 3],
        speed: 0.0,
        air: false,
        grind: false,
        trick: None,
        score: 0.0,
        health: 1.0,
        finished: false,
    }
}

fn player(client: u32, instance: u16, rider: u16, position: [f32; 3]) -> PlayerView {
    PlayerView {
        client: ClientId(client),
        name: format!("P{client}"),
        instance: Some(instance),
        rider: Some(rider),
        progress: "ready",
        status: "riding",
        ping_ms: 40,
        position: Some(position),
        velocity: Some([0.0, 0.0, 10.0]),
        run: Some(RunView { split: None, clock_ms: 1500, points: 0, state: "racing" }),
    }
}

fn race(id: u16, state: &'static str) -> InstanceFrame {
    InstanceFrame {
        id,
        mode: "race".into(),
        drop: "alps0101".into(),
        state,
        round: 3,
        clock_ms: 1500.0,
        standings: Vec::new(),
    }
}

#[test]
fn riders_are_the_servers_and_checkpoints_hit_on_the_server() {
    let mut s = Loopback::new(config(Side::Server)).unwrap();
    s.server.set_catalog(catalog());
    let src = r#"
        addEventHandler("onPlayerJoin", root, function() print("join " .. getPlayerName(source)) end)
        local cp
        addEventHandler("onRoundStart", root, function(round, drop)
            print("round " .. round .. " " .. drop)
            cp = createCheckpoint("ava:marker/ring", {x = 0, y = 0, z = 50}, 2)
        end)
        addEventHandler("onRiderCheckpoint", root, function(c, index, clock)
            local who = getRiderState(source).name
            print("checkpoint " .. tostring(c == cp) .. " " .. index .. " " .. clock .. " " .. who)
        end)
        addEventHandler("onRiderFinish", root, function(clock, points, failed)
            print("finish " .. clock .. " " .. tostring(failed))
            endRound(getPlayerInstance(getRiderPlayer(source)), "done")
        end)
    "#;
    s.server.start(pkg("race", Side::Server, src)).unwrap();
    let c1 = ScriptHost::new(config(Side::Client)).unwrap();
    c1.set_catalog(catalog());
    let (a, pa) = s.connect(c1, "P1").unwrap();
    let c2 = ScriptHost::new(config(Side::Client)).unwrap();
    c2.set_catalog(catalog());
    let (b, _) = s.connect(c2, "P2").unwrap();
    assert_eq!(out(&s.server, "race"), ["join P1", "join P2"]);
    assert_eq!(s.client(a).unwrap().local_player(), Some(pa));
    let now = |k: u64| Duration::from_millis(33 * k);
    let mut f = ServerFrame {
        now: now(1),
        instances: vec![race(1, "running")],
        players: vec![player(a.0, 1, 0, [0.0, 0.0, 0.0]), player(b.0, 1, 1, [5.0, 0.0, 0.0])],
        events: vec![ServerEvent::RoundStart { instance: 1, round: 3, drop: "alps0101".into() }],
    };
    assert_eq!(s.server.server_frame(f.clone()).unwrap().end_round, vec![]);
    s.carry();
    assert_eq!(out(&s.server, "race"), ["round 3 alps0101"]);
    let ca = s.client(a).unwrap();
    let rider = |key, local| RiderView { key, name: "x".into(), position: [0.0; 3], local };
    let frame = GameFrame {
        now: now(1),
        riders: vec![rider(0, Some(local())), rider(SESSION_KEYS + 1, None)],
        ..GameFrame::default()
    };
    ca.frame(frame).unwrap();
    ca.start(pkg(
        "hud",
        Side::Client,
        r#"
        local me = getLocalRider()
        print(getElementType(me) .. " " .. #getRiders() .. " " .. tostring(getElementParent(me) ~= root))
    "#,
    ))
    .unwrap();
    assert_eq!(out(ca, "hud"), ["rider 2 true"], "the local rider is the server's, below its instance");
    assert_eq!(ca.markers().len(), 1, "the server's checkpoint is drawn on the client");
    f.events = vec![ServerEvent::Checkpoint { client: a, split: 4, clock_ms: 1500 }];
    f.players[0].position = Some([0.0, 0.0, 30.0]);
    f.now = now(2);
    s.server.server_frame(f.clone()).unwrap();
    f.events = vec![ServerEvent::Finish { client: a, clock_ms: 61_000, points: 0, failed: false }];
    f.players[0].position = Some([0.0, 0.0, 70.0]);
    f.now = now(3);
    let orders = s.server.server_frame(f.clone()).unwrap();
    assert_eq!(
        out(&s.server, "race"),
        ["checkpoint false 4 1500 P1", "finish 61000 false", "checkpoint true 2 1500 P1"],
        "the course's split, the finish, then the checkpoint's hit"
    );
    let end = EndRound { instance: 1, round: 3, reason: "race: done".into() };
    assert_eq!(orders.end_round, vec![end], "the round's end ordered once");
    s.release(a, "left");
    s.carry();
    f.players.remove(0);
    f.now = now(4);
    s.server.server_frame(f).unwrap();
    s.carry();
    assert!(s.refused.is_empty(), "{:?}", s.refused);
}

#[test]
fn joins_are_held_accepted_and_refused_by_the_scripts() {
    let s = ScriptHost::new(config(Side::Server)).unwrap();
    let src = r#"
        local held
        addEventHandler("onPlayerConnecting", root, function(name, key, join)
            if name == "Banned" then cancelEvent() end
            if name == "Named" then refuseJoin(join, "not on the list") end
            if name == "Wait" then deferJoin(join); updateJoin(join, "checking"); held = join end
        end)
        addCommandHandler("let", function() acceptJoin(held) end)
    "#;
    s.start(pkg("gate", Side::Server, src)).unwrap();
    let t = Duration::ZERO;
    assert_eq!(s.player_connecting("Anyone", "00", t).1, JoinVerdict::Accept);
    assert!(matches!(s.player_connecting("Banned", "00", t).1, JoinVerdict::Refuse(_)));
    assert_eq!(s.player_connecting("Named", "00", t).1, JoinVerdict::Refuse("not on the list".into()));
    let (id, v) = s.player_connecting("Wait", "00", t);
    assert_eq!(v, JoinVerdict::Held);
    assert_eq!(s.join_verdict(id), None);
    assert_eq!(s.held_joins()[0].2, "gate: checking", "the package's hold and its text");
    s.execute_command(ssx_script::host::CommandOrigin::Console, "let").unwrap();
    assert_eq!(s.join_verdict(id), Some(JoinVerdict::Accept));
    assert!(s.held_joins().is_empty());
    let (id, _) = s.player_connecting("Wait", "00", t);
    assert_eq!(s.join_expired(id), Some("gate: checking".into()), "the bound passed: forgotten");
}

const GATE: &str = r#"
    local held = {}
    addEventHandler("onPlayerConnecting", root, function(name, key, join)
        deferJoin(join); updateJoin(join, "looking " .. name .. " up"); held[name] = join
    end)
    addCommandHandler("gate_let", function(_, _, name) acceptJoin(held[name]) end)
    addCommandHandler("gate_refuse", function(_, _, name) refuseJoin(held[name], "not on the list") end)
"#;
const QUICK: &str = r#"
    local held = {}
    addEventHandler("onPlayerConnecting", root, function(name, key, join) deferJoin(join); held[name] = join end)
    addCommandHandler("quick_let", function(_, _, name) acceptJoin(held[name]) end)
"#;

fn two_holders() -> ScriptHost {
    let s = ScriptHost::new(config(Side::Server)).unwrap();
    s.start(pkg("gate", Side::Server, GATE)).unwrap();
    s.start(pkg("quick", Side::Server, QUICK)).unwrap();
    s
}

fn console(s: &ScriptHost, line: &str) {
    s.execute_command(ssx_script::host::CommandOrigin::Console, line).unwrap();
}

#[test]
fn a_join_two_packages_hold_waits_for_both() {
    let s = two_holders();
    let (id, v) = s.player_connecting("Ann", "00", Duration::ZERO);
    assert_eq!(v, JoinVerdict::Held);
    console(&s, "quick_let Ann");
    assert_eq!(s.join_verdict(id), None, "gate still holds it");
    assert_eq!(s.held_joins()[0].2, "gate: looking Ann up", "only the open hold is waited for");
    console(&s, "gate_let Ann");
    assert_eq!(s.join_verdict(id), Some(JoinVerdict::Accept));
}

#[test]
fn one_packages_refusal_refuses_a_join_the_others_let_through() {
    let s = two_holders();
    let (id, _) = s.player_connecting("Bob", "00", Duration::ZERO);
    console(&s, "quick_let Bob");
    console(&s, "gate_refuse Bob");
    assert_eq!(s.join_verdict(id), Some(JoinVerdict::Refuse("not on the list".into())));
}

#[test]
fn an_accept_without_a_hold_changes_nothing() {
    let s = ScriptHost::new(config(Side::Server)).unwrap();
    s.start(pkg("gate", Side::Server, GATE)).unwrap();
    let other = r#"
        addEventHandler("onPlayerConnecting", root, function(name, key, join)
            print("accept without a hold: " .. tostring(acceptJoin(join)))
        end)
    "#;
    s.start(pkg("other", Side::Server, other)).unwrap();
    let (id, v) = s.player_connecting("Cid", "00", Duration::ZERO);
    assert_eq!(v, JoinVerdict::Held, "gate's hold stands");
    assert_eq!(s.join_expired(id), Some("gate: looking Cid up".into()));
}

#[test]
fn a_holder_that_stops_refuses_its_join() {
    let s = two_holders();
    let (held, _) = s.player_connecting("Dee", "00", Duration::ZERO);
    let (released, _) = s.player_connecting("Eve", "00", Duration::ZERO);
    console(&s, "gate_let Eve");
    assert!(s.stop("gate", "test"));
    assert_eq!(s.join_verdict(held), Some(JoinVerdict::Refuse("gate stopped while it held the join".into())));
    assert_eq!(s.join_verdict(released), None, "quick still holds Eve");
    console(&s, "quick_let Eve");
    assert_eq!(s.join_verdict(released), Some(JoinVerdict::Accept));
}

#[test]
fn a_holds_text_is_bounded_and_shown_safely() {
    let s = ScriptHost::new(config(Side::Server)).unwrap();
    let src = r#"
        addEventHandler("onPlayerConnecting", root, function(name, key, join)
            deferJoin(join)
            if name == "Esc" then updateJoin(join, "\27[31m" .. string.rep("é", 120)) end
            if name == "Long" then print("long: " .. tostring(pcall(updateJoin, join, string.rep("x", 257)))) end
            if name == "Refused" then refuseJoin(join, string.rep("ü", 100) .. "\226\128\174") end
        end)
    "#;
    s.start(pkg("gate", Side::Server, src)).unwrap();
    let (id, _) = s.player_connecting("Esc", "00", Duration::ZERO);
    let text = s.join_expired(id).unwrap();
    assert_eq!(text, format!("gate: [31m{}", "é".repeat(120)), "the escape character dropped");
    let (id, _) = s.player_connecting("Long", "00", Duration::ZERO);
    assert_eq!(s.join_expired(id).as_deref(), Some("gate"), "the hold kept, the long text refused");
    let JoinVerdict::Refuse(why) = s.player_connecting("Refused", "00", Duration::ZERO).1 else { panic!() };
    assert_eq!(why, "ü".repeat(100), "the bidi override dropped");
}

#[test]
fn a_message_to_everyone_stays_in_its_sources_instance() {
    let mut s = Loopback::new(config(Side::Server)).unwrap();
    s.server.set_catalog(catalog());
    let src = r#"
        addEvent("hello", true)
        addCommandHandler("tell", function()
            for _, r in ipairs(getRiders()) do triggerClientEvent(root, "hello", r, getRiderState(r).name) end
            showNotice(root, "Server", "notice", 2)
        end)
    "#;
    s.server.start(pkg("chat", Side::Server, src)).unwrap();
    let mut ids = Vec::new();
    for n in 0..3 {
        let c = ScriptHost::new(config(Side::Client)).unwrap();
        c.start(pkg(
            "chat",
            Side::Client,
            r#"
            addEvent("hello", true)
            addEventHandler("hello", root, function(n) print("hello " .. n) end)
        "#,
        ))
        .unwrap();
        ids.push(s.connect(c, &format!("P{}", n + 1)).unwrap().0);
    }
    let f = ServerFrame {
        now: Duration::from_millis(33),
        instances: vec![race(1, "running"), race(2, "lobby")],
        players: vec![
            player(ids[0].0, 1, 0, [0.0; 3]),
            player(ids[1].0, 1, 1, [0.0; 3]),
            player(ids[2].0, 2, 0, [0.0; 3]),
        ],
        events: Vec::new(),
    };
    s.server.server_frame(f).unwrap();
    s.carry();
    s.server.execute_command(ssx_script::host::CommandOrigin::Console, "tell").unwrap();
    s.carry();
    for (k, c) in ids.iter().enumerate() {
        let host = s.client(*c).unwrap();
        host.tick(Duration::from_millis(66));
        let mut lines = out(host, "chat");
        lines.sort();
        let want: &[&str] = if k < 2 { &["hello P1", "hello P2"] } else { &["hello P3"] };
        assert_eq!(lines, want, "client {k}");
        let report = host.frame(GameFrame { now: Duration::from_millis(99), ..GameFrame::default() }).unwrap();
        assert_eq!(report.orders.hud.len(), 1, "the notice on client {k}'s HUD");
    }
    assert!(s.refused.is_empty(), "{:?}", s.refused);
}

#[test]
fn a_marker_of_an_instance_is_that_instances_alone() {
    let mut s = Loopback::new(config(Side::Server)).unwrap();
    s.server.set_catalog(catalog());
    let src = r#"
        local hits = {}
        addEventHandler("onMarkerHit", root, function(rider, forward)
            print("hit " .. getElementID(source) .. " " .. getRiderState(rider).name)
        end)
        addCommandHandler("place", function()
            local i2 = getInstances()[2]
            local m = createMarker("ava:marker/ring", {x = 0, y = 0, z = 50}, { instance = i2 })
            setElementID(m, "two")
            local g = createMarker("ava:marker/ring", {x = 0, y = 0, z = 50})
            setElementID(g, "every")
            print(tostring(pcall(createMarker, "ava:marker/ring", {x = 0, y = 0, z = 50}, { instance = m })))
        end)
    "#;
    s.server.start(pkg("scope", Side::Server, src)).unwrap();
    let mut ids = Vec::new();
    for n in 0..2 {
        let c = ScriptHost::new(config(Side::Client)).unwrap();
        c.set_catalog(catalog());
        ids.push(s.connect(c, &format!("P{}", n + 1)).unwrap().0);
    }
    let now = |k: u64| Duration::from_millis(33 * k);
    let mut f = ServerFrame {
        now: now(1),
        instances: vec![race(1, "running"), race(2, "running")],
        players: vec![player(ids[0].0, 1, 0, [0.0; 3]), player(ids[1].0, 2, 0, [0.0; 3])],
        events: Vec::new(),
    };
    s.server.server_frame(f.clone()).unwrap();
    s.carry();
    s.server.execute_command(ssx_script::host::CommandOrigin::Console, "place").unwrap();
    s.carry();
    assert_eq!(out(&s.server, "scope"), ["false"], "a marker is no instance");
    let rider = |key, local| RiderView { key, name: "x".into(), position: [0.0; 3], local };
    for (k, c) in ids.iter().enumerate() {
        let host = s.client(*c).unwrap();
        host.frame(GameFrame { now: now(1), riders: vec![rider(0, Some(local()))], ..GameFrame::default() }).unwrap();
        let drawn = host.markers().iter().filter(|m| m.visible).count();
        assert_eq!(drawn, if k == 0 { 1 } else { 2 }, "client {k}: the instance's marker only in instance 2");
        assert_eq!(host.markers().len(), 2, "client {k}: both mirrored");
    }
    for p in &mut f.players {
        p.position = Some([0.0, 0.0, 70.0]);
    }
    f.now = now(2);
    s.server.server_frame(f).unwrap();
    let mut lines = out(&s.server, "scope");
    lines.sort();
    assert_eq!(lines, ["hit every P1", "hit every P2", "hit two P2"]);
    assert!(s.refused.is_empty(), "{:?}", s.refused);
}

#[test]
fn a_hostile_clients_items_are_refused_and_counted() {
    use ssx_script::host::outbox::{Body, Peer};
    use ssx_script::value::{NetValue, ValueLimits, encode_list};
    let s = ScriptHost::new(config(Side::Server)).unwrap();
    let src = r#"
        addEvent("ok", true)
        addEvent("quiet")
        addEventHandler("ok", root, function(n) print("ok " .. tostring(n) .. " from " .. getPlayerName(client)) end)
        addEventHandler("quiet", root, function() print("quiet") end)
        addEventHandler("onPlayerTriggerInvalidEvent", root, function(name, added, remote, package)
            print("invalid " .. name .. " " .. tostring(added) .. " " .. tostring(remote) .. " " .. package
                .. " from " .. getPlayerName(source))
        end)
        addEventHandler("onPlayerEventFlood", root, function(n)
            print("flood " .. n .. " " .. getPlayerName(source))
        end)
    "#;
    s.start(pkg("game", Side::Server, src)).unwrap();
    let (p1, _) = s.join_session(ClientId(1), "P1").unwrap();
    let (p2, _) = s.join_session(ClientId(2), "P2").unwrap();
    let args = encode_list(&[NetValue::Number(7.0)], &ValueLimits::REMOTE).unwrap();
    let ev = |name: &str, source| Body::Event { name: name.into(), source, args: args.clone() };
    let from = Peer::Client(ClientId(1));
    let before = s.monitor().refused_inbound;
    let mut refused = Vec::new();
    for (package, body) in [
        (Some("game"), ev("quiet", p1)),
        (Some("game"), ev("nothing", p1)),
        (Some("other"), ev("ok", p1)),
        (Some("game"), ev("ok", p2)),
        (None, ev("ok", p1)),
        (Some("game"), Body::LatentStart { transfer: 1, name: "ok".into(), source: p1, total: u32::MAX }),
        (Some("game"), Body::LatentChunk { transfer: 9, offset: 0, bytes: vec![1; 8] }),
    ] {
        refused.push(s.deliver(from, package, body).is_err());
    }
    for t in 10..27u64 {
        refused.push(
            s.deliver(from, Some("game"), Body::LatentStart { transfer: t, name: "ok".into(), source: p1, total: 4 })
                .is_err(),
        );
    }
    refused
        .push(s.deliver(from, Some("game"), Body::LatentChunk { transfer: 10, offset: 2, bytes: vec![1; 2] }).is_err());
    refused
        .push(s.deliver(from, Some("game"), Body::LatentChunk { transfer: 10, offset: 0, bytes: vec![1; 4] }).is_err());
    refused.push(s.deliver(from, Some("game"), ev("ok", p1)).is_err());
    s.tick(Duration::from_millis(33));
    s.player_flooded(ClientId(1), 12);
    s.player_flooded(ClientId(9), 3);
    let want: Vec<bool> = [true; 7].into_iter().chain([false; 16]).chain([true, true, true, false]).collect();
    assert_eq!(refused, want);
    assert_eq!(
        out(&s, "game"),
        [
            "invalid quiet true false game from P1",
            "invalid nothing false false game from P1",
            "invalid ok false false other from P1",
            "ok 7 from P1",
            "flood 12 P1",
        ]
    );
    assert_eq!(s.monitor().refused_inbound - before, 10, "each refusal counted");
}

#[test]
fn the_games_running_client_host_is_adopted_and_released() {
    let mut lb = Loopback::new(config(Side::Server)).unwrap();
    lb.server.set_catalog(catalog());
    let src = r#"
        addEvent("mode:ask", true)
        addEventHandler("onPlayerJoin", root, function() print("join " .. getPlayerName(source)) end)
        addEventHandler("onPlayerQuit", root, function(why) print("quit " .. getPlayerName(source) .. " " .. why) end)
        addEventHandler("mode:ask", root, function(n)
            print("asked " .. n .. " by " .. getPlayerName(client))
            triggerClientEvent(client, "mode:told", root, n + 1)
        end)
        addCommandHandler("poke", function() triggerClientEvent(root, "mode:told", root, 0) end)
    "#;
    lb.server.start(pkg("mode", Side::Server, src)).unwrap();
    let game = ScriptHost::new(config(Side::Client)).unwrap();
    game.set_catalog(catalog());
    game.start(pkg("other", Side::Client, r#"print("other runs")"#)).unwrap();
    let (id, player) = lb.adopt(&game, "Player").unwrap();
    assert_eq!(game.local_player(), Some(player));
    assert!(lb.adopt(&game, "Again").is_err(), "one adopted host");
    assert!(lb.connect(ScriptHost::new(config(Side::Client)).unwrap(), "P2").is_err(), "nothing joins beside it");
    let client_src = r#"
        addEvent("mode:told", true)
        addEventHandler("mode:told", root, function(n) print("told " .. n) end)
        triggerServerEvent("mode:ask", localPlayer, 41)
    "#;
    game.start(pkg("mode", Side::Client, client_src)).unwrap();
    lb.carry_with(Some(&game));
    lb.server.tick(Duration::from_millis(33));
    lb.carry_with(Some(&game));
    game.tick(Duration::from_millis(33));
    assert_eq!(out(&lb.server, "mode"), ["join Player", "asked 41 by Player"]);
    assert_eq!(out(&game, "mode"), ["told 42"]);
    assert_eq!(out(&game, "other"), ["other runs"]);
    lb.server.execute_command(ssx_script::host::CommandOrigin::Console, "poke").unwrap();
    lb.carry();
    assert!(lb.refused.iter().any(|r| r.contains("not handed")), "{:?}", lb.refused);
    assert_eq!(lb.release_adopted("the level ended"), Some(id));
    assert_eq!(out(&lb.server, "mode"), ["quit Player the level ended"]);
    game.leave_session();
    assert_eq!(game.local_player(), None);
}
