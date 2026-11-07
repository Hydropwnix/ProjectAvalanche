mod draw;
mod local;
mod markers;
#[cfg(feature = "net")]
mod net;
mod orders;
mod overlay;
mod snapshot;
mod sounds;
mod worker;

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use ssx_assets::content::dev::DevWatch;

use crate::ride::RideStepSet;

pub struct ScriptPlugin {
    pub big_root: PathBuf,
    pub packages: Vec<PathBuf>,
    pub dev: Option<Arc<DevWatch>>,
    pub storage: PathBuf,
    pub monitor: bool,
    pub maps: bool,
    pub sessions: bool,
    pub server_packages: Vec<(String, PathBuf)>,
    pub server_storage: PathBuf,
}

impl ScriptPlugin {
    pub fn wanted(&self) -> bool {
        self.now() || self.sessions
    }

    fn now(&self) -> bool {
        !self.packages.is_empty() || self.dev.is_some() || self.maps
    }
}

#[derive(Resource)]
struct ScriptStarter {
    big_root: PathBuf,
    storage: PathBuf,
    monitor: bool,
}

#[derive(Resource)]
pub(crate) struct ScriptBridge {
    tx: SyncSender<worker::Job>,
    rx: Mutex<Receiver<worker::Out>>,
    done: Mutex<Receiver<()>>,
    in_flight: bool,
    epoch: u64,
    sent_epoch: u64,
    clear: bool,
    gone: Option<String>,
    gone_handled: bool,
    dev: Option<(Arc<DevWatch>, u64)>,
    suspended: Option<String>,
    session: Option<u64>,
    session_off: Option<u64>,
    pub(crate) monitor: String,
    pub(crate) skipped: u64,
    content: Arc<ssx_assets::content::GameContent>,
}

impl ScriptBridge {
    fn send(&mut self, job: worker::Job) -> bool {
        match self.tx.try_send(job) {
            Ok(()) => true,
            Err(std::sync::mpsc::TrySendError::Full(_)) => false,
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                self.gone.get_or_insert_with(|| "the script host's thread ended".into());
                false
            }
        }
    }

    #[cfg(feature = "net")]
    fn send_or_return(&mut self, job: worker::Job) -> Result<(), worker::Job> {
        match self.tx.try_send(job) {
            Ok(()) => Ok(()),
            Err(std::sync::mpsc::TrySendError::Full(job)) => Err(job),
            Err(std::sync::mpsc::TrySendError::Disconnected(job)) => {
                self.gone.get_or_insert_with(|| "the script host's thread ended".into());
                Err(job)
            }
        }
    }

    fn take(&mut self) -> Option<worker::Out> {
        let r = self.rx.lock().ok()?.try_recv();
        match r {
            Ok(out) => {
                if out.frame {
                    self.in_flight = false;
                }
                Some(out)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.gone.get_or_insert_with(|| "the script host's thread ended".into());
                self.in_flight = false;
                None
            }
        }
    }
}

#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ScriptFrameSet;

fn bridge(start: worker::Start, dev: Option<&Arc<DevWatch>>) -> ScriptBridge {
    let content = ssx_assets::content::GameContent::of(&start.big_root);
    let (tx, rx, done) = worker::spawn(start);
    ScriptBridge {
        tx,
        rx: Mutex::new(rx),
        done: Mutex::new(done),
        in_flight: false,
        epoch: 0,
        sent_epoch: 0,
        clear: false,
        gone: None,
        gone_handled: false,
        dev: dev.map(|d| (d.clone(), d.status().scripts)),
        suspended: None,
        session: None,
        session_off: None,
        monitor: String::new(),
        skipped: 0,
        content,
    }
}

fn parts(world: &mut World, monitor: bool) {
    world.init_resource::<snapshot::Inbox>();
    world.init_resource::<local::LocalScripts>();
    world.init_resource::<markers::MarkerScene>();
    world.init_resource::<draw::DrawPool>();
    world.init_resource::<orders::OrderState>();
    world.init_resource::<sounds::ScriptSounds>();
    world.init_resource::<crate::ui::UiOverlay>();
    world.insert_resource(overlay::Overlay { shown: monitor });
}

fn start_for_session(
    mut commands: Commands,
    starter: Res<ScriptStarter>,
    live: Option<Res<crate::shell::live::LiveSession>>,
    #[cfg(feature = "net")] got: Option<Res<ssx_net::client::ScriptReceived>>,
) {
    let Some(live) = live else { return };
    let c = &live.content;
    #[cfg(feature = "net")]
    let server_scripts = got.is_some_and(|g| !g.items.is_empty());
    #[cfg(not(feature = "net"))]
    let server_scripts = false;
    if !(c.markers || (c.scripts_allowed && !c.scripts.is_empty()) || server_scripts) {
        return;
    }
    info!("scripts: the session brings client scripts or markers: the script host starts");
    let start = worker::Start {
        big_root: starter.big_root.clone(),
        packages: Vec::new(),
        dev: None,
        storage: starter.storage.clone(),
        server_packages: Vec::new(),
        server_storage: starter.storage.clone(),
    };
    let monitor = starter.monitor;
    let b = bridge(start, None);
    commands.remove_resource::<ScriptStarter>();
    commands.queue(move |world: &mut World| {
        parts(world, monitor);
        world.insert_resource(b);
        let _ = world.run_system_cached(overlay::spawn);
    });
}

impl Plugin for ScriptPlugin {
    fn build(&self, app: &mut App) {
        if self.now() {
            let start = worker::Start {
                big_root: self.big_root.clone(),
                packages: self.packages.clone(),
                dev: self.dev.as_ref().map(|d| d.status().root),
                storage: self.storage.clone(),
                server_packages: self.server_packages.iter().map(|(_, f)| f.clone()).collect(),
                server_storage: self.server_storage.clone(),
            };
            let b = bridge(start, self.dev.as_ref());
            parts(app.world_mut(), self.monitor);
            app.world_mut().resource_mut::<local::LocalScripts>().names =
                self.server_packages.iter().map(|(n, _)| n.clone()).collect();
            app.insert_resource(b).add_systems(Startup, overlay::spawn);
        } else {
            app.insert_resource(ScriptStarter {
                big_root: self.big_root.clone(),
                storage: self.storage.clone(),
                monitor: self.monitor,
            })
            .add_systems(
                PreUpdate,
                start_for_session.after(bevy::input::InputSystems).run_if(resource_exists::<ScriptStarter>),
            );
        }
        let up = resource_exists::<ScriptBridge>;
        app.add_systems(
            FixedUpdate,
            snapshot::ride_events.after(RideStepSet).run_if(resource_exists::<crate::ride::Ride>).run_if(up),
        )
        .add_systems(PreUpdate, (snapshot::keys, policy).chain().after(bevy::input::InputSystems).run_if(up))
        .add_systems(
            Update,
            (
                frame,
                orders::fade,
                markers::level,
                markers::gates,
                markers::sync,
                markers::animate,
                draw::overlay,
                sounds::play,
                orders::hud_timeout,
                orders::music,
                overlay::update,
            )
                .chain()
                .in_set(ScriptFrameSet)
                .after(RideStepSet)
                .run_if(up),
        )
        .add_systems(
            FixedUpdate,
            orders::camera_target.after(RideStepSet).run_if(resource_exists::<crate::ride::Ride>).run_if(up),
        );
        let riding = resource_exists::<crate::ride::Ride>;
        app.add_systems(FixedUpdate, local::apply.before(RideStepSet).run_if(riding).run_if(up))
            .add_systems(FixedUpdate, local::observe.after(RideStepSet).run_if(riding).run_if(up))
            .add_systems(Update, local::decide.before(ScriptFrameSet).after(RideStepSet).run_if(up));
        app.add_systems(Last, shutdown_on_exit.run_if(up));
        #[cfg(feature = "net")]
        app.add_systems(
            Update,
            net::inbound
                .before(ScriptFrameSet)
                .after(RideStepSet)
                .run_if(up)
                .run_if(resource_exists::<ssx_net::client::ScriptReceived>),
        );
    }
}

fn policy(mut bridge: ResMut<ScriptBridge>, live: Option<Res<crate::shell::live::LiveSession>>) {
    let content = live.as_ref().map(|l| &l.content);
    let hold = content.map(|c| c.client_scripts).filter(|p| !p.runs());
    match (&bridge.suspended, hold) {
        (None, Some(policy)) => {
            let why = policy.reason().to_string();
            warn!("scripts: {why}: the client mods' scripts are stopped for the session");
            if bridge.send(worker::Job::Suspend(why.clone())) {
                bridge.suspended = Some(why);
                bridge.epoch += 1;
                bridge.clear = true;
            }
        }
        (Some(_), None) => {
            match live.is_some() {
                true => info!("scripts: the server allows client scripts: the client mods' scripts start again"),
                false => info!("scripts: the session ended: the client mods' scripts start again"),
            }
            if bridge.send(worker::Job::Resume) {
                bridge.suspended = None;
            }
        }
        _ => {}
    }
    let run =
        content.filter(|c| c.scripts_allowed && !c.scripts.is_empty()).and_then(|c| Some((c, c.storage.clone()?)));
    if let Some(c) = content.filter(|c| !c.scripts_allowed && !c.scripts.is_empty())
        && bridge.session_off != Some(c.generation)
    {
        bridge.session_off = Some(c.generation);
        info!(
            "scripts: the server's {} resources with client scripts do not run (Options, Multiplayer)",
            c.scripts.len()
        );
    }
    let want = run.as_ref().map(|(c, _)| c.generation);
    if want == bridge.session {
        return;
    }
    let job = match run {
        Some((c, storage)) => worker::Job::Session {
            packages: c.scripts.iter().map(|s| (s.name.clone(), s.folder.clone(), s.version)).collect(),
            storage,
        },
        None => worker::Job::EndSession,
    };
    if bridge.send(job) {
        bridge.session = want;
    }
}

#[derive(bevy::ecs::system::SystemParam)]
struct Routes<'w> {
    state: ResMut<'w, orders::OrderState>,
    draws: ResMut<'w, draw::DrawPool>,
    markers: ResMut<'w, markers::MarkerScene>,
    sounds: ResMut<'w, sounds::ScriptSounds>,
    ui: ResMut<'w, crate::shell::game_mode::GameUiRes>,
    music: Option<Res<'w, crate::audio::music::Music>>,
    #[cfg(feature = "net")]
    script_send: Option<ResMut<'w, ssx_net::client::ScriptSend>>,
}

struct Effects<'a> {
    state: &'a mut orders::OrderState,
    draws: &'a mut draw::DrawPool,
    markers: &'a mut markers::MarkerScene,
    sounds: &'a mut sounds::ScriptSounds,
    values: &'a mut ssx_ui::game::values::UiValues,
    glyphs: Option<&'a std::collections::BTreeSet<char>>,
}

impl Effects<'_> {
    fn apply(&mut self, report: ssx_script::host::game::FrameReport, place: orders::MusicPlace) {
        let mut o = report.orders;
        if let Some(d) = o.draws.take() {
            self.draws.set(d);
        }
        if let Some(m) = report.markers {
            self.markers.set(m);
        }
        self.sounds.push(std::mem::take(&mut o.sounds));
        let preload = std::mem::take(&mut o.preload);
        self.state.take(o, self.values, self.glyphs, place);
        for id in preload {
            match id.kind() {
                ssx_content::Kind::Texture => _ = self.draws.image(&id),
                ssx_content::Kind::Sound => self.sounds.preload(id),
                _ => {}
            }
        }
    }

    fn clear(&mut self) {
        self.draws.set(Vec::new());
        self.markers.set(Vec::new());
        self.sounds.clear();
        self.state.clear(self.values);
    }
}

fn carries_orders(frame: bool, sent_epoch: u64, epoch: u64) -> bool {
    frame && sent_epoch == epoch
}

fn frame(
    mut bridge: ResMut<ScriptBridge>,
    mut inbox: ResMut<snapshot::Inbox>,
    view: snapshot::FrameView,
    mut r: Routes,
    mut local: ResMut<local::LocalScripts>,
) {
    if bridge.gone.is_some() && !bridge.gone_handled {
        let why = bridge.gone.clone().unwrap_or_default();
        error!("scripts: {why}; no script runs from now on");
        bridge.gone_handled = true;
        bridge.clear = true;
    }
    if r.sounds.content.is_none() {
        r.sounds.content = Some(bridge.content.clone());
        r.draws.content = Some(bridge.content.clone());
    }
    let player = local.asked().then(|| local::player_name(&r.ui, view.rider_name().as_deref()));
    let ui = &mut r.ui.0;
    let typing = ui.text_focused();
    let glyphs = ui.fe.as_ref().and_then(|f| f.ava.glyphs.clone());
    let loc = &mut ui.loc;
    let mut fx = Effects {
        state: &mut r.state,
        draws: &mut r.draws,
        markers: &mut r.markers,
        sounds: &mut r.sounds,
        values: &mut ui.values,
        glyphs: glyphs.as_ref(),
    };
    if std::mem::take(&mut bridge.clear) {
        fx.clear();
        if let Some(m) = r.music.as_ref() {
            m.end_script_song();
        }
        info!("scripts: what the scripts left up is taken down (their draws, markers, fade, camera, message, sounds)");
    }
    if bridge.gone_handled {
        return;
    }
    if let Some((watch, seen)) = bridge.dev.clone() {
        let s = watch.status();
        if s.scripts != seen && s.broken.is_empty() && bridge.send(worker::Job::Restart(s.root.clone())) {
            bridge.dev = Some((watch, s.scripts));
        }
    }
    if let Some(mut out) = bridge.take() {
        #[cfg(feature = "net")]
        net::outbound(r.script_send.as_deref_mut(), std::mem::take(&mut out.net));
        #[cfg(not(feature = "net"))]
        drop(std::mem::take(&mut out.net));
        for (problem, line) in &out.lines {
            if *problem { warn!("{line}") } else { info!("{line}") }
        }
        bridge.monitor = out.monitor;
        bridge.skipped = out.report.skipped;
        if out.frame {
            local.answer(out.local);
        }
        if let Some(looks) = out.looks {
            fx.markers.set_looks(looks);
        }
        if carries_orders(out.frame, bridge.sent_epoch, bridge.epoch) {
            let place = orders::MusicPlace {
                big_root: bridge.content.big_root().to_path_buf(),
                scope: r.music.as_ref().map(|m| m.scope().clone()),
            };
            fx.apply(out.report, place);
        }
    }
    if bridge.in_flight || (bridge.suspended.is_some() && bridge.session.is_none()) {
        return;
    }
    let content = bridge.content.clone();
    let f = view.build(&mut inbox, fx.state, &content, fx.values, loc, typing);
    let server = match (view.ride(), player) {
        (Some(ride), Some(name)) => local::server_frame(f.now, ride, &mut local, &name).map(Box::new),
        _ => None,
    };
    if bridge.send(worker::Job::Frame(Box::new(f), server)) {
        bridge.in_flight = true;
        bridge.sent_epoch = bridge.epoch;
    }
}

const EXIT_WAIT: std::time::Duration = std::time::Duration::from_secs(7);

fn shutdown_on_exit(mut bridge: ResMut<ScriptBridge>, mut exits: MessageReader<AppExit>) {
    if exits.read().next().is_none() {
        return;
    }
    let start = std::time::Instant::now();
    while !bridge.send(worker::Job::Shutdown) && bridge.gone.is_none() && start.elapsed() < EXIT_WAIT {
        let _ = bridge.take();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let left = EXIT_WAIT.saturating_sub(start.elapsed());
    let done = bridge.done.lock().map(|d| d.recv_timeout(left));
    match done {
        Ok(Ok(())) | Ok(Err(std::sync::mpsc::RecvTimeoutError::Disconnected)) => {
            info!("scripts: the script host stopped its packages")
        }
        _ => warn!("scripts: the script host did not stop within {} s: the game exits without it", EXIT_WAIT.as_secs()),
    }
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn a_suspension_takes_down_everything_the_scripts_left_up() {
        use ssx_script::host::game::{
            CameraOrder, Draw, FadeOrder, FrameReport, HudOrder, MarkerView, Orders, SoundOrder,
        };
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
}
