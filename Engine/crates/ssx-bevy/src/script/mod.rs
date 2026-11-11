mod draw;
mod local;
mod markers;
#[cfg(feature = "net")]
mod net;
mod orders;
mod overlay;
mod snapshot;
pub(crate) use snapshot::MenuCapture;
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
    told: Option<worker::Word>,
    session: Option<(u64, u64)>,
    session_off: Option<(u64, u64)>,
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

    fn seen(&self, live: Option<&crate::shell::live::LiveSession>) -> snapshot::Seen {
        use ssx_content::policy::ClientScripts;
        match (live, self.told) {
            (None, _) => snapshot::Seen::Own,
            (Some(l), Some(Some((s, p)))) if s == l.serial.get() && p != ClientScripts::Pending => {
                snapshot::Seen::Session
            }
            _ => snapshot::Seen::Blind,
        }
    }

    fn alone(&self) -> bool {
        self.told == Some(None)
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
    bridge_of(tx, rx, done, content, dev)
}

fn bridge_of(
    tx: SyncSender<worker::Job>,
    rx: Receiver<worker::Out>,
    done: Receiver<()>,
    content: Arc<ssx_assets::content::GameContent>,
    dev: Option<&Arc<DevWatch>>,
) -> ScriptBridge {
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
        told: None,
        session: None,
        session_off: None,
        monitor: String::new(),
        skipped: 0,
        content,
    }
}

fn parts(world: &mut World, monitor: bool) {
    world.init_resource::<snapshot::Inbox>();
    world.init_resource::<snapshot::MenuCapture>();
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
        .add_systems(
            PreUpdate,
            (snapshot::keys, snapshot::hold_menu_keys, policy)
                .chain()
                .after(bevy::input::InputSystems)
                .before(crate::shell::game_mode::pads_frame)
                .run_if(up),
        )
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
    use ssx_content::policy::ClientScripts;
    let word: worker::Word = live.as_ref().map(|l| (l.serial.get(), l.content.client_scripts));
    if bridge.told != Some(word) {
        if !bridge.send(worker::Job::Clients(word)) {
            return;
        }
        let holds =
            |w: Option<worker::Word>| matches!(w, Some(Some((_, p))) if p != ClientScripts::Pending && !p.runs());
        if holds(Some(word)) && !holds(bridge.told) {
            bridge.epoch += 1;
            bridge.clear = true;
        }
        if bridge.told.flatten().map(|(s, _)| s) != word.map(|(s, _)| s) {
            bridge.session = None;
        }
        bridge.told = Some(word);
    }
    let Some(live) = live.as_deref() else { return };
    if bridge.seen(Some(live)) != snapshot::Seen::Session {
        return;
    }
    let (c, serial) = (&live.content, live.serial.get());
    if !c.scripts_allowed && !c.scripts.is_empty() && bridge.session_off != Some((serial, c.generation)) {
        bridge.session_off = Some((serial, c.generation));
        info!(
            "scripts: the server's {} resources with client scripts do not run (Options, Multiplayer)",
            c.scripts.len()
        );
    }
    let run = c.storage.clone().filter(|_| c.scripts_allowed && !c.scripts.is_empty());
    let want = run.as_ref().map(|_| (serial, c.generation));
    if want == bridge.session {
        return;
    }
    let job = match run {
        Some(storage) => worker::Job::Session {
            packages: c.scripts.iter().map(|s| (s.name.clone(), s.folder.clone(), s.version)).collect(),
            storage,
        },
        None => worker::Job::SessionScriptsOff,
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
    capture: ResMut<'w, snapshot::MenuCapture>,
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
        r.capture.0 = false;
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
        let to_server = std::mem::take(&mut out.net);
        #[cfg(feature = "net")]
        if out.session.is_some() && out.session == view.live().map(|l| l.serial.get()) {
            net::outbound(r.script_send.as_deref_mut(), to_server);
        }
        #[cfg(not(feature = "net"))]
        drop(to_server);
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
            let capture = out.report.orders.capture;
            fx.apply(out.report, place);
            if r.capture.0 != capture {
                info!("scripts: the menu keys (arrows, Enter, Backspace) {}", if capture { "held" } else { "let go" });
                r.capture.0 = capture;
            }
        }
    }
    let holds =
        matches!(bridge.told, Some(Some((_, p))) if p != ssx_content::policy::ClientScripts::Pending && !p.runs());
    if bridge.told.is_none() || bridge.in_flight || (holds && bridge.session.is_none()) {
        return;
    }
    let content = bridge.content.clone();
    let seen = bridge.seen(view.live());
    let mut f = view.build(&mut inbox, fx.state, &content, fx.values, loc, seen);
    f.typing = typing;
    f.screen = fx.draws.screen;
    f.cursor = fx.draws.cursor;
    f.multiplayer = view.in_session() || multiplayer_screens(ui.fe.as_ref());
    let server = match (view.ride(), player) {
        (Some(ride), Some(name)) => local::server_frame(f.now, ride, &mut local, &name).map(Box::new),
        _ => None,
    };
    if bridge.send(worker::Job::Frame(Box::new(f), server)) {
        bridge.in_flight = true;
        bridge.sent_epoch = bridge.epoch;
    }
}

fn multiplayer_screens(fe: Option<&ssx_ui::game::fe::FrontEnd>) -> bool {
    #[cfg(feature = "net")]
    {
        fe.is_some_and(|fe| fe.ava.browser.active)
    }
    #[cfg(not(feature = "net"))]
    {
        let _ = fe;
        false
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
mod tests;
