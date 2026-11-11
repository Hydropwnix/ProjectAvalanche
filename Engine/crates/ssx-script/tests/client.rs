use std::sync::Arc;
use std::time::Duration;

use ssx_content::ContentId;
use ssx_content::manifest::{Files, Scripts};
use ssx_script::Limits;
use ssx_script::host::game::{
    Catalog, ContentMeta, Draw, GameFrame, Ground, InstanceView, KeyEvent, LevelView, LocalState, MarkerBudget,
    MarkerHitShape, MarkerMeta, RideEvent, RiderView, ScreenView, StandingView,
};
use ssx_script::host::limits::Budgets;
use ssx_script::host::package::PackageScripts;
use ssx_script::host::{HostConfig, ScriptHost, Side};

fn relaxed(b: &mut Budgets, s: u64) {
    (b.call, b.call_backstop) = (Limits::MAX_CHECKPOINTS, Duration::from_secs(s));
    (b.chain, b.chain_backstop) = (u64::MAX, Duration::from_secs(2 * s));
    (b.tick, b.tick_checkpoints, b.tick_backstop) = (Duration::from_secs(2 * s), u64::MAX, Duration::from_secs(2 * s));
}

fn package(name: &str, files: &[(&str, &str)]) -> Result<PackageScripts, String> {
    let scripts = Scripts { client: vec!["client/**/*.luau".into()], server: vec![], shared: vec![] };
    let files_cfg = Files { include: vec!["lib/**".into()], ..Files::default() };
    let contents: Vec<(String, String)> = files.iter().map(|(p, s)| (p.to_string(), s.to_string())).collect();
    let limits = HostConfig::new(Side::Client).limits;
    PackageScripts::from_memory(name, Side::Client, &scripts, &files_cfg, toml::Table::new(), &contents, &limits)
}

fn id(s: &str) -> ContentId {
    s.parse().unwrap()
}

fn host() -> ScriptHost {
    let mut c = HostConfig::new(Side::Client);
    relaxed(&mut c.limits.budgets, 2);
    c.limits.game.frame_bound = Duration::from_secs(4);
    let h = ScriptHost::new(c).unwrap();
    let mut cat = Catalog::default();
    let meta = |source: &str| ContentMeta { source: source.into(), layer: "L1 port" };
    for (m, hit) in [("ava:marker/gate", MarkerHitShape::Gate), ("ava:marker/ring", MarkerHitShape::Ring)] {
        cat.ids.insert(id(m), meta("ava"));
        let sound = Some(id("ssx:sound/hud/gate"));
        cat.markers
            .insert(id(m), MarkerMeta { size: 4.0, colour: 0xFFFF_8000, hit, sound, vertices: 100, triangles: 60 });
    }
    cat.ids.insert(id("ssx:sound/hud/gate"), meta("ssx"));
    cat.ids.insert(id("ssx:texture/fe/icon"), meta("ssx"));
    cat.ids.insert(id("ssx:music/licensed/145"), meta("ssx"));
    h.set_catalog(Arc::new(cat));
    h
}

fn start(h: &ScriptHost, name: &str, src: &str) -> Result<(), String> {
    h.start(package(name, &[("client/main.luau", src)])?)
}

fn out(h: &ScriptHost, name: &str) -> Vec<String> {
    h.take_output(name).map(|o| o.lines).unwrap_or_default()
}

struct Flat(f32);

impl Ground for Flat {
    fn height(&self, _: f32, y: f32, _: f32) -> Option<f32> {
        (y > self.0).then_some(self.0)
    }
}

fn local(position: [f32; 3]) -> RiderView {
    let state = LocalState {
        velocity: [0.0, 0.0, 10.0],
        speed: 10.0,
        air: false,
        grind: false,
        trick: None,
        score: 1234.0,
        health: 0.5,
        finished: false,
    };
    RiderView { key: 0, name: "Elise".into(), position, local: Some(state) }
}

fn frame(t: u64, position: [f32; 3]) -> GameFrame {
    let budget = MarkerBudget { markers: 1024, vertices: 1_000_000, triangles: 1_000_000 };
    GameFrame {
        now: Duration::from_millis(t * 16),
        level: Some(LevelView { id: 1, drop: "ssx:drop/alps0101_drop".into(), budget }),
        ground: Some(Arc::new(Flat(5.0))),
        riders: vec![
            local(position),
            RiderView { key: 7, name: "Griff".into(), position: [3.0, 0.0, 0.0], local: None },
        ],
        instance: Some(InstanceView {
            mode: "race".into(),
            drop: "ssx:drop/alps0101_drop".into(),
            state: "running",
            clock_ms: 1500.0,
        }),
        standings: vec![StandingView {
            rider: Some(7),
            name: "Griff".into(),
            place: 1,
            value: 1500.0,
            finished: false,
        }],
        ..GameFrame::default()
    }
}

#[test]
fn a_frame_names_its_riders_and_raises_the_engine_events_where_the_game_decided_them() {
    let h = host();
    let src = r#"
        addEventHandler("onClientRiderLand", root, function(t, bail) print("land", t, bail, source == getLocalRider()) end)
        addEventHandler("onClientTrick", root, function(name, points) print("trick", name, points) end)
        addEventHandler("onClientKey", root, function(key, down) print("key", key, down) end)
        addEventHandler("onClientRender", root, function()
            local s = getRiderState(getLocalRider())
            draw.text("speed " .. s.speed, 10, 20, tocolor(255, 255, 255), { size = 24, align = "center" })
            draw.rect(0, 0, 100, 50, tocolor(0, 0, 0, 128))
        end)
        local riders = getRiders()
        print("riders", #riders)
    "#;
    start(&h, "hud", src).unwrap();
    let mut f = frame(1, [0.0, 10.0, 0.0]);
    f.events = vec![
        RideEvent::Land { air_time: 1.25, bail: false },
        RideEvent::Trick { name: "Tail Grab".into(), points: 750.0 },
    ];
    f.keys = vec![KeyEvent { key: "f5", down: true }];
    let r = h.frame(f).unwrap();
    let lines = out(&h, "hud");
    assert_eq!(lines[0], "riders\t0", "no level before the first frame: no riders");
    assert!(lines.contains(&"land\t1.25\tfalse\ttrue".to_string()), "{lines:?}");
    assert!(lines.contains(&"trick\tTail Grab\t750".to_string()), "{lines:?}");
    assert!(lines.contains(&"key\tf5\ttrue".to_string()), "{lines:?}");
    let draws = r.orders.draws.unwrap();
    assert_eq!(draws.len(), 2);
    assert!(matches!(&draws[0].1, Draw::Text { text, size, .. } if text == "speed 10" && *size == 24.0));
    assert!(matches!(draws[1].1, Draw::Rect { colour: 0x8000_0000, .. }));
    let mut f = frame(2, [0.0, 10.0, 1.0]);
    f.keys = vec![KeyEvent { key: "f5", down: false }];
    f.typing = true;
    h.frame(f).unwrap();
    assert!(!out(&h, "hud").iter().any(|l| l.starts_with("key")));
}

#[test]
fn the_reads_give_the_frames_state() {
    let h = host();
    let src = r#"
        addEventHandler("onClientRender", root, function()
            local me = getLocalRider()
            local s = getRiderState(me)
            print(s.name, s.position.y, s.speed, s.score, s.health, s.isLocal, s.air)
            for _, r in getRiders() do
                local o = getRiderState(r)
                if not o.isLocal then print("other", o.name, o.speed, o.position.x) end
            end
            local i = getInstance()
            print(i.mode, i.state, i.clock)
            local st = getStandings()
            print(#st, st[1].name, st[1].place, getRiderState(st[1].rider).name)
            print(getGroundHeight(10, 20), getContent("ava:marker/gate").source, #listContent("marker"))
            print(getContent("ssx:texture/none"), preloadContent("ssx:texture/fe/icon"), preloadContent("ssx:music/licensed/145"))
        end)
    "#;
    start(&h, "reads", src).unwrap();
    h.frame(frame(1, [1.0, 10.0, 2.0])).unwrap();
    let lines = out(&h, "reads");
    assert_eq!(lines[0], "Elise\t10\t10\t1234\t0.5\ttrue\tfalse");
    assert_eq!(lines[1], "other\tGriff\tnil\t3");
    assert_eq!(lines[2], "race\trunning\t1500");
    assert_eq!(lines[3], "1\tGriff\t1\tGriff");
    assert_eq!(lines[4], "5\tava\t2");
    assert_eq!(lines[5], "nil\ttrue\tfalse");
    let r = h.frame(frame(2, [1.0, 10.0, 3.0])).unwrap();
    assert_eq!(r.orders.preload, [id("ssx:texture/fe/icon")]);
}

#[test]
fn markers_count_against_the_package_and_the_level_and_go_with_the_level() {
    let h = host();
    let src = r#"
        addCommandHandler("place", function()
            local n, err = 0, nil
            while true do
                local ok, e = pcall(createMarker, "ava:marker/gate", { x = n, y = 0, z = 0 })
                if not ok then err = e break end
                n += 1
            end
            print(n, err)
        end)
    "#;
    start(&h, "course", src).unwrap();
    let (ok, e) = (h.execute_command(ssx_script::host::CommandOrigin::Console, "place"), out(&h, "course"));
    assert!(ok.is_ok());
    assert!(e[0].contains("no level is up"), "{e:?}");
    let mut f = frame(1, [0.0, 0.0, -10.0]);
    f.level.as_mut().unwrap().budget.triangles = 600;
    h.frame(f.clone()).unwrap();
    h.execute_command(ssx_script::host::CommandOrigin::Console, "place").unwrap();
    let e = out(&h, "course");
    assert!(e[0].starts_with("10\t") && e[0].contains("the level's marker budget"), "{e:?}");
    assert_eq!(h.markers().len(), 10);
    let mut f2 = frame(2, [0.0, 0.0, -10.0]);
    f2.level.as_mut().unwrap().id = 2;
    let r = h.frame(f2).unwrap();
    assert_eq!(r.markers.map(|m| m.len()), Some(0), "a new level: the last level's markers are gone");
    h.execute_command(ssx_script::host::CommandOrigin::Console, "place").unwrap();
    let e = out(&h, "course");
    assert!(e[0].starts_with("256\t") && e[0].contains("the package has 256 markers"), "{e:?}");
    h.stop("course", "test");
    assert_eq!(h.markers().len(), 0);
    let (held, vm) = h.holdings("course");
    assert_eq!((held.markers, held.elements, vm), (0, 0, false));
}

#[test]
fn a_marker_is_hit_along_the_riders_segment_with_its_sound() {
    let h = host();
    let src = r#"
        local gate
        addEventHandler("onClientRender", root, function()
            if not gate then gate = createMarker("ava:marker/gate", { x = 0, y = 0, z = 0 }, { heading = 0 }) end
        end)
        addEventHandler("onClientMarkerHit", root, function(rider, forward)
            print("hit", source == gate, rider == getLocalRider(), forward)
        end)
    "#;
    start(&h, "gates", src).unwrap();
    h.frame(frame(1, [0.0, 1.0, -20.0])).unwrap();
    h.frame(frame(2, [0.0, 1.0, -5.0])).unwrap();
    assert!(out(&h, "gates").is_empty(), "not reached yet");
    let r = h.frame(frame(3, [0.5, 1.0, 8.0])).unwrap();
    assert_eq!(out(&h, "gates"), ["hit\ttrue\ttrue\ttrue"]);
    assert_eq!(r.orders.sounds.len(), 1);
    assert_eq!(r.orders.sounds[0].id, id("ssx:sound/hud/gate"));
    h.frame(frame(4, [9.0, 1.0, -5.0])).unwrap();
    h.frame(frame(5, [9.0, 1.0, 5.0])).unwrap();
    assert!(out(&h, "gates").is_empty());
}

#[test]
fn the_levels_own_markers_are_hit_and_no_scripts_to_change() {
    use ssx_script::host::game::LevelMarker;
    let h = host();
    let src = r#"
        addEventHandler("onClientMarkerHit", root, function(rider, forward)
            print("hit", getElementType(source), getElementID(source), rider == getLocalRider(), forward)
            print(pcall(destroyElement, source), pcall(setMarkerPosition, source, { x = 1, y = 1, z = 1 }))
        end)
    "#;
    start(&h, "mode", src).unwrap();
    let gate = LevelMarker {
        node: "checkpoint gate".into(),
        id: id("ava:marker/gate"),
        position: [0.0, 0.0, 0.0],
        heading: 0.0,
        size: 4.0,
        colour: 0xFFFF_FFFF,
        hit: MarkerHitShape::Gate,
        sound: Some(id("ssx:sound/hud/gate")),
    };
    let level = Some(Arc::new(vec![gate]));
    let with = |t, p| GameFrame { level_markers: level.clone(), ..frame(t, p) };
    let r = h.frame(with(1, [0.0, 1.0, -5.0])).unwrap();
    let listed = r.markers.expect("the level's markers listed");
    assert!(listed.len() == 1 && listed[0].is_level() && listed[0].package.is_empty());
    let r = h.frame(with(2, [0.5, 1.0, 8.0])).unwrap();
    let lines = out(&h, "mode");
    assert_eq!(lines[0], "hit\tmarker\tcheckpoint gate\ttrue\ttrue");
    assert!(lines[1].starts_with("false") && lines[1].contains("\tfalse"), "{lines:?}");
    assert_eq!(r.orders.sounds.len(), 1);
    assert_eq!(r.orders.sounds[0].id, id("ssx:sound/hud/gate"));
    let mut next = frame(3, [0.0, 1.0, 0.0]);
    next.level.as_mut().unwrap().id = 2;
    let r = h.frame(next).unwrap();
    assert_eq!(r.markers.map(|m| m.len()), Some(0));
}

#[test]
fn draws_are_taken_only_in_render_and_bounded_per_frame() {
    let h = host();
    let src = r#"
        print(pcall(draw.rect, 0, 0, 1, 1, 0))
        addEventHandler("onClientRender", root, function()
            local n = 0
            while pcall(draw.rect, 0, 0, 1, 1, 0) do n += 1 end
            local ok, e = pcall(draw.rect, 0, 0, 1, 1, 0)
            print(n, e)
            print(pcall(draw.text, string.rep("x", 257), 0, 0))
            print(pcall(draw.image, 0, 0, 10, 10, "ava:marker/gate"))
            print(pcall(draw.text, "hi", 0, 0, 0, { size = 24, font = "x" }))
        end)
    "#;
    start(&h, "flood", src).unwrap();
    let first = out(&h, "flood");
    assert!(first[0].contains("only while onClientRender runs"), "{first:?}");
    let r = h.frame(frame(1, [0.0, 0.0, 0.0])).unwrap();
    let lines = out(&h, "flood");
    assert!(lines[0].starts_with("512\t") && lines[0].contains("draw calls"), "{lines:?}");
    assert!(lines[1].contains("at most 256 bytes"), "{lines:?}");
    assert!(lines[2].contains("not a texture id"), "{lines:?}");
    assert!(lines[3].contains("has no field \"font\""), "{lines:?}");
    assert_eq!(r.orders.draws.map(|d| d.len()), Some(512));
    let glyphs = r#"addEventHandler("onClientRender", root, function()
        local ok, e = pcall(function() for i = 1, 100 do draw.text(string.rep("g", 200), 0, 0) end end)
        print(ok, e)
    end)"#;
    start(&h, "glyphs", glyphs).unwrap();
    h.frame(frame(2, [0.0, 0.0, 0.0])).unwrap();
    let g = out(&h, "glyphs");
    assert!(g[0].contains("2200 glyphs (at most 2048)"), "{g:?}");
}

#[test]
fn only_bindable_keys_can_be_bound_and_binds_are_capped() {
    let h = host();
    let src = r#"
        print(pcall(bindKey, "f1", "down", print))
        print(pcall(bindKey, "f9", "down", print))
        print(pcall(bindKey, "tab", "down", print))
        bindKey("f5", "down", function(k, s) print("bind", k, s) end)
        -- each handler a closure of its own (Luau gives one function without upvalues the same object every time,
        -- and binding one function twice is no new bind)
        local n = 1
        print("again", bindKey("f5", "down", print), bindKey("f5", "down", print))
        n += 1
        while pcall(bindKey, "f6", "both", function() return n end) do n += 1 end
        print("binds", n, select(2, pcall(bindKey, "f7", "up", function() return n end)))
    "#;
    start(&h, "keys", src).unwrap();
    let lines = out(&h, "keys");
    for l in &lines[..3] {
        assert!(l.starts_with("false") && l.contains("bad argument #1"), "{l}");
    }
    assert_eq!(lines[3], "again\ttrue\tfalse", "one function bound twice binds once");
    assert!(lines[4].starts_with("binds\t64\t") && lines[4].contains("at most 64"), "{lines:?}");
    let mut f = frame(1, [0.0, 0.0, 0.0]);
    f.keys = vec![KeyEvent { key: "f5", down: true }, KeyEvent { key: "f5", down: false }];
    h.frame(f).unwrap();
    assert_eq!(out(&h, "keys"), ["bind\tf5\tdown", "f5\tdown"], "both binds of f5 down, in their order");
    h.stop("keys", "test");
    assert_eq!(h.holdings("keys").0.binds, 0);
}

#[test]
fn the_frame_bound_leaves_handlers_for_the_next_frame() {
    let mut c = HostConfig::new(Side::Client);
    relaxed(&mut c.limits.budgets, 2);
    c.limits.game.frame_bound = Duration::from_millis(20);
    let h = ScriptHost::new(c).unwrap();
    let src = r#"
        ran = 0
        local serial = 0
        for i = 1, 40 do
            serial += 1
            addEventHandler("onClientRender", root, function()
                local _ = serial
                ran += 1
                local x = 0 for k = 1, 300000 do x += k end
            end)
        end
        addCommandHandler("ran", function() print(ran) end)
    "#;
    start(&h, "heavy", src).unwrap();
    let r = h.frame(frame(1, [0.0, 0.0, 0.0])).unwrap();
    h.execute_command(ssx_script::host::CommandOrigin::Console, "ran").unwrap();
    let ran: u64 = out(&h, "heavy")[0].parse().unwrap();
    let m = h.monitor();
    assert!(ran >= 1 && r.skipped >= 1 && ran + r.skipped == 40, "{ran} ran, {} left", r.skipped);
    assert!(m.side_time.last >= Duration::from_millis(20), "stopped by the bound: {:?}", m.side_time.last);
    let r2 = h.frame(frame(2, [0.0, 0.0, 0.0])).unwrap();
    assert!(r2.skipped > r.skipped);
}

#[test]
fn every_module_compiles_at_the_start() {
    let h = host();
    let bad =
        package("broken", &[("client/main.luau", "print('main')"), ("lib/unused.luau", "local x = \n\n)")]).unwrap();
    let e = h.start(bad).unwrap_err();
    assert!(e.starts_with("lib/unused.luau:3:"), "{e}");
    let mut c = HostConfig::new(Side::Client);
    let b = &mut c.limits.budgets;
    (b.call, b.call_backstop) = (1_000, Duration::from_millis(10));
    (b.chain, b.chain_backstop) = (2_000, Duration::from_millis(20));
    let h = ScriptHost::new(c).unwrap();
    let big = format!("local t = {{}}\nlocal function never()\n{}end\nreturn 20000", "t[#t+1]=1\n".repeat(100_000));
    assert!(big.len() < ssx_script::MAX_SOURCE);
    let t = std::time::Instant::now();
    let code = ssx_script::compile::bytecode("big", big.as_bytes()).unwrap();
    assert!(t.elapsed() > Duration::from_millis(20), "the premise: its compile is longer than the chain's backstop");
    let lua = mlua::Lua::new();
    let before = ssx_script::cpu::charged();
    lua.load(&code[..]).set_mode(mlua::chunk::ChunkMode::Binary).into_function().unwrap();
    let load = Duration::from_nanos(ssx_script::cpu::charged().saturating_sub(before));
    drop(lua);
    let p = package("big", &[("client/main.luau", "print(require('lib/big'))"), ("lib/big.luau", &big)]).unwrap();
    h.start(p).unwrap();
    assert_eq!(out(&h, "big"), ["20000"]);
    let m = h.monitor();
    eprintln!(
        "load {load:?}; the top level's call: {} checkpoints, {:?} of CPU time",
        m.chain_peak_checkpoints, m.chain_peak
    );
    assert!(m.chain_peak_checkpoints < 100, "the call ran the module only: {}", m.chain_peak_checkpoints);
    assert!(m.chain_peak < load, "the load was not in the call: {:?} against {load:?}", m.chain_peak);
}

#[test]
fn hud_messages_camera_fade_sound_and_music_are_orders() {
    let h = host();
    let src = r#"
        addEventHandler("onClientRender", root, function()
            print(showHudMessage("Gate 1", "<b>go</b>", 2))
            print(pcall(showHudMessage, "again", "x"))
            for _, r in getRiders() do
                if not getRiderState(r).isLocal then setCameraTarget(r) end
            end
            fadeCamera(false, 0.5, tocolor(255, 255, 255))
            print(playSound("ssx:sound/hud/gate", { x = 1, y = 2, z = 3 }), pcall(playSound, "ssx:music/licensed/145"))
            playMusic("ssx:music/licensed/145")
            -- a flood: every further order this second refused with its reason
            local refused = 0
            for i = 1, 100 do
                local ok, e = pcall(playMusic, "ssx:music/licensed/145")
                if not ok and string.find(tostring(e), "one playMusic every 1000 ms per package", 1, true) then
                    refused += 1
                end
            end
            print("music refused", refused)
        end)
    "#;
    start(&h, "orders", src).unwrap();
    let r = h.frame(frame(1, [0.0, 0.0, 0.0])).unwrap();
    let lines = out(&h, "orders");
    assert_eq!(lines[0], "true");
    assert!(lines[1].contains("one HUD message every 1000 ms"), "{lines:?}");
    assert!(lines[2].starts_with("true\tfalse") && lines[2].contains("is not a sound id"), "{lines:?}");
    assert_eq!(lines[3], "music refused\t100", "one playMusic a second per package");
    let o = r.orders;
    assert_eq!(o.hud.len(), 1);
    assert_eq!(o.hud[0].text, "<b>go</b>", "the text as the script gave it: the game shows it as plain text");
    assert_eq!(o.camera, Some(ssx_script::host::game::CameraOrder::Rider(7)));
    assert!(o.fade.as_ref().is_some_and(|f| !f.fade_in && f.colour == 0xFFFF_FFFF));
    assert_eq!(o.sounds[0].position, Some([1.0, 2.0, 3.0]));
    assert_eq!(o.music.map(|m| m.1), Some(id("ssx:music/licensed/145")));
}

#[test]
fn type_definitions_are_never_scripts_or_modules() {
    let decl = "declare function getLocalRider(): Element?\n";
    let p = package(
        "typed",
        &[("client/main.luau", "print('typed')"), ("client/api.d.luau", decl), ("lib/types.d.luau", decl)],
    )
    .unwrap();
    assert_eq!(p.scripts.iter().map(|(f, _)| f.as_str()).collect::<Vec<_>>(), ["client/main.luau"]);
    assert!(p.modules.keys().all(|f| !f.ends_with(".d.luau")), "{:?}", p.modules.keys());
    let h = host();
    h.start(p).unwrap();
    assert_eq!(out(&h, "typed"), ["typed"]);
}

#[test]
fn an_endless_render_and_a_memory_bomb_stop_and_the_canary_goes_on() {
    let h = ScriptHost::new(HostConfig::new(Side::Client)).unwrap();
    let canary =
        r#"local n = 0 addEventHandler("onClientRender", root, function() n += 1 draw.rect(0, 0, 1, 1, 0) end)"#;
    start(&h, "canary", canary).unwrap();
    let spin = r#"addEventHandler("onClientRender", root, function() print("spinning") while true do end end)"#;
    start(&h, "spin", spin).unwrap();
    let bomb = r#"local t = {} addEventHandler("onClientRender", root, function()
        while true do t[#t + 1] = string.rep("x", 1048576) .. #t end end)"#;
    start(&h, "bomb", bomb).unwrap();
    let mut frames = 0u64;
    while h.running().len() > 1 && frames < 2 {
        frames += 1;
        h.frame(frame(frames, [0.0, 0.0, 0.0])).unwrap();
    }
    let stops: Vec<String> = h.monitor().stops.iter().map(|s| format!("{} {}", s.package, s.reason)).collect();
    let spin = stops.iter().find(|s| s.starts_with("spin ")).expect("spin stopped");
    assert!(spin.contains("ran past its call budget") && spin.contains("handler of onClientRender"), "{spin}");
    let bomb = stops.iter().find(|s| s.starts_with("bomb ")).expect("bomb stopped");
    assert!(bomb.contains("memory limit of 64 MiB") || bomb.contains("call budget"), "{bomb}");
    assert_eq!(h.running(), ["canary"]);
    let last = h.take_last_output();
    assert!(last.iter().any(|(p, o)| p == "spin" && o.lines == ["spinning"]), "{last:?}");
    assert!(h.take_last_output().is_empty());
    let r = h.frame(frame(frames + 1, [0.0, 0.0, 0.0])).unwrap();
    assert_eq!(r.orders.draws.map(|d| d.len()), Some(1), "the canary draws on");
}

#[test]
fn a_script_sees_the_window_and_the_part_of_the_stage_it_shows() {
    let h = host();
    let src = r#"
        addEventHandler("onClientRender", root, function()
            local s = getScreen()
            print(s.width, s.height, s.left, s.top, s.right, s.bottom)
        end)
    "#;
    start(&h, "menu", src).unwrap();
    h.frame(frame(1, [0.0; 3])).unwrap();
    assert_eq!(out(&h, "menu"), ["1280\t720\t0\t0\t1280\t720"], "the whole stage before the game has told");
    let wide = ScreenView { width: 3440, height: 1440, left: -220.0, top: 0.0, right: 1500.0, bottom: 720.0 };
    h.frame(GameFrame { screen: Some(wide), ..frame(2, [0.0; 3]) }).unwrap();
    assert_eq!(out(&h, "menu"), ["3440\t1440\t-220\t0\t1500\t720"], "an ultrawide window shows room beside the stage");
}

#[test]
fn a_package_holds_the_menu_keys_until_it_lets_go_or_stops() {
    let h = host();
    let src = r#"
        local n = 0
        addEventHandler("onClientRender", root, function()
            n += 1
            if n == 1 then print(captureMenuKeys(true)) end
        end)
        addEventHandler("onClientKey", root, function(key, down) print("key", key, down) end)
    "#;
    start(&h, "menu", src).unwrap();
    let r = h.frame(frame(1, [0.0; 3])).unwrap();
    assert_eq!(out(&h, "menu"), ["true"]);
    assert!(r.orders.capture, "the package holds the menu keys");
    let keys = vec![KeyEvent { key: "enter", down: true }];
    let r = h.frame(GameFrame { keys, ..frame(2, [0.0; 3]) }).unwrap();
    assert_eq!(out(&h, "menu"), ["key\tenter\ttrue"], "Enter reaches the scripts");
    assert!(r.orders.capture, "held until let go");
    h.stop("menu", "the test is over");
    let r = h.frame(frame(3, [0.0; 3])).unwrap();
    assert!(!r.orders.capture, "a stopped package lets go");
}

#[test]
fn a_script_sees_the_cursor_and_whether_the_game_is_in_multiplayer() {
    let h = host();
    let src = r#"
        addEventHandler("onClientRender", root, function()
            local c = getCursor()
            print(c and c.x or "none", c and c.y or "none", isMultiplayer())
        end)
    "#;
    start(&h, "menu", src).unwrap();
    h.frame(frame(1, [0.0; 3])).unwrap();
    assert_eq!(out(&h, "menu"), ["none\tnone\tfalse"]);
    h.frame(GameFrame { cursor: Some((640.0, 360.0)), multiplayer: true, ..frame(2, [0.0; 3]) }).unwrap();
    assert_eq!(out(&h, "menu"), ["640\t360\ttrue"]);
}

#[test]
fn open_folder_takes_only_an_existing_local_folder_and_not_in_multiplayer() {
    let h = host();
    let src = r#"
        addEventHandler("onClientRender", root, function()
            print(pcall(openFolder, "relative/path"))
            print(pcall(openFolder, "//server/share"))
            print(pcall(openFolder, "/no/such/folder/anywhere"))
        end)
    "#;
    start(&h, "menu", src).unwrap();
    h.frame(frame(1, [0.0; 3])).unwrap();
    let lines = out(&h, "menu");
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert!(lines.iter().all(|l| l.starts_with("false\t")), "{lines:?}");
    h.frame(GameFrame { multiplayer: true, ..frame(2, [0.0; 3]) }).unwrap();
    assert_eq!(out(&h, "menu"), ["true\tfalse", "true\tfalse", "true\tfalse"], "refused in multiplayer");
}
