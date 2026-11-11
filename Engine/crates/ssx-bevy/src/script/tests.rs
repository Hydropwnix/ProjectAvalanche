use super::*;

#[test]
fn the_plugin_builds_and_its_systems_initialise() {
    let dir = std::env::temp_dir().join(format!("ssx-script-plugin-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (maps, sessions) in [(true, false), (false, true)] {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(ScriptPlugin {
            big_root: dir.join("no-disc"),
            packages: Vec::new(),
            dev: None,
            storage: dir.join("storage"),
            monitor: true,
            maps,
            sessions,
            server_packages: Vec::new(),
            server_storage: dir.join("server-storage"),
        });
        app.finish();
        app.cleanup();
        let world = app.world_mut();
        let mut schedules = world.remove_resource::<Schedules>().expect("the app's schedules");
        let mut built = 0;
        for (label, schedule) in schedules.iter_mut() {
            schedule.initialize(world).unwrap_or_else(|e| panic!("{label:?}: {e}"));
            built += 1;
        }
        world.insert_resource(schedules);
        assert!(built >= 4, "{built} schedules");
        assert_eq!(app.world().contains_resource::<ScriptBridge>(), maps);
        assert_eq!(app.world().contains_resource::<ScriptStarter>(), !maps);
        drop(app);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_host_comes_up_only_for_a_session_with_scripts() {
    let dir = std::env::temp_dir().join(format!("ssx-script-session-up-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(ScriptPlugin {
        big_root: dir.join("no-disc"),
        packages: Vec::new(),
        dev: None,
        storage: dir.join("storage"),
        monitor: false,
        maps: false,
        sessions: true,
        server_packages: Vec::new(),
        server_storage: dir.join("server-storage"),
    });
    app.finish();
    app.cleanup();
    app.world_mut().run_schedule(PreUpdate);
    assert!(!app.world().contains_resource::<ScriptBridge>(), "single player: no host");
    let mut live = crate::shell::live::LiveSession::default();
    app.world_mut().insert_resource(crate::shell::live::LiveSession::default());
    app.world_mut().run_schedule(PreUpdate);
    assert!(!app.world().contains_resource::<ScriptBridge>(), "a session without scripts: no host");
    live.content.scripts.push(crate::shell::live::ServerScripts {
        name: "hud".into(),
        folder: dir.join("hud"),
        version: ssx_content::ContentHash::of(b"hud"),
    });
    app.world_mut().insert_resource(live);
    app.world_mut().run_schedule(PreUpdate);
    assert!(!app.world().contains_resource::<ScriptBridge>(), "the player keeps servers' scripts off: no host");
    app.world_mut().resource_mut::<crate::shell::live::LiveSession>().content.scripts_allowed = true;
    app.world_mut().run_schedule(PreUpdate);
    assert!(app.world().contains_resource::<ScriptBridge>(), "the host is up for the session");
    assert!(app.world().contains_resource::<draw::DrawPool>() && !app.world().contains_resource::<ScriptStarter>());
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(feature = "net")]
#[test]
fn nothing_of_a_session_reaches_the_host_before_its_word() {
    use crate::shell::live::LiveSession;
    use bevy::ecs::system::RunSystemOnce;
    use snapshot::Seen;
    use ssx_content::policy::ClientScripts;
    use ssx_net::client::ScriptReceived;
    use ssx_script::host::outbox::Body;
    let (tx, jobs) = std::sync::mpsc::sync_channel(worker::QUEUE);
    let (_outs, rx) = std::sync::mpsc::sync_channel(1);
    let (_done, done) = std::sync::mpsc::sync_channel(1);
    let dir = std::env::temp_dir().join(format!("ssx-script-word-{}", std::process::id()));
    let content = ssx_assets::content::GameContent::of(&dir.join("no-disc"));
    let mut world = World::new();
    world.insert_resource(bridge_of(tx.clone(), rx, done, content, None));
    let first = LiveSession::default();
    let serial = first.serial.get();
    world.insert_resource(first);
    let message = |world: &mut World| {
        let item =
            ssx_script::host::wire::to_item(Some("srv"), &Body::Hello { player: ssx_script::value::ElementId(7) });
        world.resource_mut::<ScriptReceived>().items.push_back(item);
    };
    world.insert_resource(ScriptReceived::default());
    message(&mut world);
    let run = |world: &mut World| {
        world.run_system_once(policy).unwrap();
        world.run_system_once(net::inbound).unwrap();
    };
    let waiting = |world: &World| world.resource::<ScriptReceived>().items.len();
    for _ in 0..worker::QUEUE {
        tx.send(worker::Job::EndLocal("a full queue".into())).unwrap();
    }
    run(&mut world);
    assert!(world.resource::<ScriptBridge>().told.is_none(), "a full queue did not take the word");
    assert_eq!(waiting(&world), 1, "the server's messages wait for it");
    assert_eq!(jobs.try_iter().count(), worker::QUEUE, "nothing went with the queue full");
    run(&mut world);
    let pending = Some(Some((serial, ClientScripts::Pending)));
    assert!(matches!(jobs.try_iter().collect::<Vec<_>>()[..], [worker::Job::Clients(w)] if Some(w) == pending));
    assert_eq!(waiting(&world), 1, "a pending word lets nothing through");
    let live = world.resource::<LiveSession>();
    assert_eq!(world.resource::<ScriptBridge>().seen(Some(live)), Seen::Blind);
    world.resource_mut::<LiveSession>().content.client_scripts = ClientScripts::Refused;
    run(&mut world);
    let got = jobs.try_iter().collect::<Vec<_>>();
    let refused = Some((serial, ClientScripts::Refused));
    assert!(matches!(&got[..], [worker::Job::Clients(w), worker::Job::Net(m)] if *w == refused && m.len() == 1));
    let b = world.resource::<ScriptBridge>();
    assert!(b.clear && b.epoch == 1, "the hold clears what they left up");
    assert_eq!(b.seen(Some(world.resource::<LiveSession>())), Seen::Session);
    world.insert_resource(LiveSession::default());
    let next = world.resource::<LiveSession>().serial.get();
    assert_ne!(next, serial);
    message(&mut world);
    world.run_system_once(net::inbound).unwrap();
    assert_eq!(waiting(&world), 1, "the last session's word is not the next one's");
    run(&mut world);
    let got = jobs.try_iter().collect::<Vec<_>>();
    assert!(matches!(&got[..], [worker::Job::Clients(Some((s, ClientScripts::Pending)))] if *s == next));
    world.remove_resource::<LiveSession>();
    run(&mut world);
    assert_eq!(waiting(&world), 0, "a session's leftovers reach no one");
    assert!(matches!(jobs.try_iter().collect::<Vec<_>>()[..], [worker::Job::Clients(None)]));
    assert_eq!(world.resource::<ScriptBridge>().seen(None), Seen::Own);
}

#[test]
fn a_suspension_takes_down_everything_the_scripts_left_up() {
    use ssx_script::host::game::{CameraOrder, Draw, FadeOrder, FrameReport, HudOrder, MarkerView, Orders, SoundOrder};
    use ssx_ui::game::values::id;
    let id = |s: &str| ssx_content::ContentId::parse(s).unwrap();
    let (mut state, mut draws, mut markers, mut sounds) = Default::default();
    let mut values = ssx_ui::game::values::UiValues::default();
    let mut fx = Effects {
        state: &mut state,
        draws: &mut draws,
        markers: &mut markers,
        sounds: &mut sounds,
        values: &mut values,
        glyphs: None,
    };
    let orders = Orders {
        draws: Some(vec![("p".into(), Draw::Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0, colour: 0xFF00_00FF })]),
        sounds: vec![SoundOrder { package: "p".into(), id: id("ssx:sound/hud/gate"), position: None }],
        camera: Some(CameraOrder::Rider(7)),
        fade: Some(FadeOrder { fade_in: false, seconds: 0.0, colour: 0xFF00_0000 }),
        hud: vec![HudOrder { package: "p".into(), title: "T".into(), text: "M".into(), seconds: 5.0 }],
        ..Orders::default()
    };
    let marker = MarkerView::placed(ssx_script::value::ElementId(9), "p", id("ava:marker/ring"), [0.0; 3]);
    let report = FrameReport { orders, markers: Some(vec![marker]), skipped: 0 };
    let place = orders::MusicPlace { big_root: "no-disc".into(), scope: None };
    fx.apply(report, place);
    assert_eq!(fx.draws.pending(), Some(1));
    assert_eq!(fx.markers.pending(), Some(1));
    assert_eq!(fx.sounds.waiting(), 1);
    assert!(!fx.state.idle());
    let title = ssx_ui::text::isolate::wrap("T");
    assert!(fx.values.bool(id::HUDMESSAGE_IS_ENABLED) && fx.values.str(id::HUDMESSAGE_TITLE) == title);
    fx.clear();
    assert_eq!(fx.draws.pending(), Some(0), "an empty draw list");
    assert_eq!(fx.markers.pending(), Some(0), "no markers");
    assert_eq!(fx.sounds.waiting(), 0, "the sounds waiting dropped");
    assert!(fx.state.idle(), "the camera the player's own, the fade clear");
    assert!(!fx.values.bool(id::HUDMESSAGE_IS_ENABLED), "the scripts' message off");
    assert!(!carries_orders(true, 0, 1));
    assert!(!carries_orders(false, 1, 1));
    assert!(carries_orders(true, 1, 1));
}

#[test]
fn the_scripts_message_never_overwrites_or_ends_the_games() {
    use ssx_ui::game::values::id;
    let mut v = ssx_ui::game::values::UiValues::default();
    let mut slot = orders::HudSlot::default();
    v.set_str(id::HUDMESSAGE_TITLE, "Tutorial");
    v.set_str(id::HUDMESSAGE_MESSAGE, "Hold A");
    v.set_bool(id::HUDMESSAGE_IS_ENABLED, true);
    assert!(!slot.show(&mut v, "Script".into(), "hi".into(), 2.0));
    assert_eq!((slot.refused, v.str(id::HUDMESSAGE_TITLE)), (1, "Tutorial"));
    slot.tick(&mut v, 10.0);
    assert!(v.bool(id::HUDMESSAGE_IS_ENABLED), "no timeout of the scripts' turns the game's message off");
    let mut v = ssx_ui::game::values::UiValues::default();
    let mut slot = orders::HudSlot::default();
    assert!(slot.show(&mut v, "Script".into(), "hi".into(), 2.0));
    assert!(slot.ours(&v));
    v.set_str(id::HUDMESSAGE_TITLE, "Tutorial");
    v.set_str(id::HUDMESSAGE_MESSAGE, "Hold A");
    v.set_bool(id::HUDMESSAGE_IS_ENABLED, true);
    slot.tick(&mut v, 0.1);
    assert!(!slot.ours(&v));
    slot.tick(&mut v, 10.0);
    slot.clear(&mut v);
    assert!(v.bool(id::HUDMESSAGE_IS_ENABLED) && v.str(id::HUDMESSAGE_TITLE) == "Tutorial", "the game's stays");
    let mut v = ssx_ui::game::values::UiValues::default();
    let mut slot = orders::HudSlot::default();
    assert!(slot.show(&mut v, "Script".into(), "hi".into(), 2.0));
    slot.tick(&mut v, 1.0);
    assert!(v.bool(id::HUDMESSAGE_IS_ENABLED));
    slot.tick(&mut v, 1.5);
    assert!(!v.bool(id::HUDMESSAGE_IS_ENABLED));
}

#[test]
fn a_play_music_flood_loads_one_song_at_a_time() {
    let id = |n: u32| ssx_content::ContentId::parse(&format!("ssx:music/licensed/{n}")).unwrap();
    let mut loads = orders::MusicLoads::<u32>::default();
    let mut started = Vec::new();
    let mut senders = Vec::new();
    let start = |i: &ssx_content::ContentId, started: &mut Vec<String>, senders: &mut Vec<_>| {
        started.push(i.to_string());
        let (tx, rx) = std::sync::mpsc::channel();
        senders.push(tx);
        Ok(rx)
    };
    for n in 0..60 {
        loads.order("p".into(), id(n), |i| start(i, &mut started, &mut senders));
    }
    assert_eq!(started, ["ssx:music/licensed/0"], "one load for 60 orders");
    assert_eq!(loads.folded, 58, "orders 2 to 59 only changed the song wanted");
    assert!(loads.poll(|i| start(i, &mut started, &mut senders)).is_none(), "the load is not in yet");
    senders[0].send(Ok(0)).unwrap();
    assert!(loads.poll(|i| start(i, &mut started, &mut senders)).is_none());
    assert_eq!(started, ["ssx:music/licensed/0", "ssx:music/licensed/59"]);
    senders[1].send(Ok(59)).unwrap();
    let got = loads.poll(|i| start(i, &mut started, &mut senders));
    assert_eq!(got.map(|(i, r)| (i.to_string(), r.ok())), Some(("ssx:music/licensed/59".into(), Some(59))));
    assert!(!loads.in_flight());
}

#[test]
fn the_screen_is_the_part_of_the_stage_the_window_shows() {
    use ssx_ui::render::ScaleMode;
    let near = |a: f32, b: f32| (a - b).abs() < 0.01;
    let s = draw::screen_view((1920, 1080), ScaleMode::NoBorder).unwrap();
    assert!(near(s.left, 0.0) && near(s.top, 0.0) && near(s.right, 1280.0) && near(s.bottom, 720.0), "{s:?}");
    let s = draw::screen_view((3440, 1440), ScaleMode::NoBorder).unwrap();
    assert!(near(s.top, 0.0) && near(s.bottom, 720.0) && near(s.left, -220.0) && near(s.right, 1500.0), "{s:?}");
    let s = draw::screen_view((1024, 768), ScaleMode::NoBorder).unwrap();
    assert!(near(s.top, 0.0) && near(s.bottom, 720.0) && near(s.left, 160.0) && near(s.right, 1120.0), "{s:?}");
    assert_eq!(draw::screen_view((0, 720), ScaleMode::NoBorder), None);
}
