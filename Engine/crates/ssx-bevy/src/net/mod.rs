mod chat;
mod errors;
mod racers;
mod round;
mod send;
mod session;
mod spectate;

use bevy::prelude::*;
use ssx_net::lightyear::link::RecvLinkConditioner;
use ssx_ui::game::fe::ava::browser::NetRequest;

use crate::ride::RideStepSet;
use crate::shell::game_mode::{GameUiRes, NetConnect};
use crate::shell::live::{LiveEnd, LiveOrigin, LiveSession};

pub fn register(app: &mut App) {
    app.set_error_handler(errors::handler);
    app.add_plugins(ssx_net::NetClientPlugin);
    app.init_resource::<send::Sent>().init_resource::<round::Rounds>().init_resource::<racers::ViewMaps>();
    fn live(a: Option<Res<LiveSession>>, b: Option<Res<session::NetSession>>) -> bool {
        a.is_some() && b.is_some()
    }
    app.add_systems(
        PreUpdate,
        (
            session::drive,
            session::connect_granted,
            round::track,
            round::align,
            racers::sync,
            racers::samples,
            chat::sync,
            spectate::hud,
            spectate::banner,
        )
            .chain()
            .after(ssx_net::NetSystems::Receive)
            .run_if(live),
    );
    app.add_systems(Update, (chat::keys, spectate::keys).run_if(live));
    #[cfg(not(feature = "scripts"))]
    app.add_systems(Update, |mut got: ResMut<ssx_net::client::ScriptReceived>| drop(got.take()));
    app.add_systems(FixedUpdate, (racers::step_live, round::gate).chain().before(RideStepSet).run_if(live));
    app.add_systems(FixedUpdate, (round::reports, spectate::camera_target).chain().after(RideStepSet).run_if(live));
    app.add_systems(PostUpdate, racers::clock.before(crate::ride::bots::RacersDrawSet).run_if(live));
    app.add_systems(PostUpdate, (send::control, send::sample).chain().before(ssx_net::NetSystems::Send).run_if(live));
}

pub fn connect_at_start(app: &mut App, connect: NetConnect) {
    app.add_systems(Startup, move |world: &mut World| {
        let spec = session::spec_of(&connect);
        let server = connect.server.clone();
        start(world, spec.map_err(|e| (server, e)));
    });
}

fn start(world: &mut World, spec: Result<session::JoinSpec, (String, String)>) {
    ssx_net::reset_session(world);
    let _ = errors::take();
    world.insert_resource(send::Sent::default());
    world.insert_resource(round::Rounds::default());
    let (origin, made) = match spec {
        Ok(spec) => {
            let origin = spec.origin;
            let server = spec.address.to_string();
            let big_root = world.get_resource::<crate::shell::session_content::SessionRoot>().map(|r| r.0.clone());
            match big_root {
                Some(root) => (origin, session::NetSession::start(spec, &root).map_err(|e| (server, e))),
                None => (origin, Err((server, "the game's data folder is not known".to_string()))),
            }
        }
        Err(e) => (LiveOrigin::Hub, Err(e)),
    };
    match made {
        Ok(s) => {
            world.insert_resource(s);
            world.insert_resource(LiveSession { origin, ..default() });
        }
        Err((server, why)) => {
            warn!("network: no session with {server}: {why}");
            world.insert_resource(session::NetSession::ended(server));
            let end = LiveEnd { reason: Some(why), refusal: None };
            world.insert_resource(LiveSession { origin, end: Some(end), ..default() });
        }
    }
}

pub fn request(commands: &mut Commands, r: NetRequest) {
    match r {
        NetRequest::Connect(c) => commands.queue(move |world: &mut World| {
            let spec = session::JoinSpec {
                address: c.address,
                name: c.name,
                password: c.password,
                origin: LiveOrigin::Browser,
                key: c.key,
            };
            start(world, Ok(spec));
        }),
        NetRequest::Cancel => commands.queue(|world: &mut World| {
            let client = world.get_resource_mut::<session::NetSession>().and_then(|mut s| s.cancel());
            if let Some(entity) = client {
                world.trigger(ssx_net::lightyear::prelude::Disconnect { entity });
                world.flush();
            }
            world.remove_resource::<LiveSession>();
            world.remove_resource::<session::NetSession>();
            ssx_net::reset_session(world);
            if let Some(mut ui) = world.get_resource_mut::<GameUiRes>() {
                ui.session_end();
            }
        }),
    }
}

pub fn finished(commands: &mut Commands) {
    commands.queue(|world: &mut World| {
        world.remove_resource::<session::NetSession>();
        ssx_net::reset_session(world);
        if let Some(mut ui) = world.get_resource_mut::<GameUiRes>() {
            ui.session_end();
        }
    });
}

pub fn test_conditioner() -> Option<RecvLinkConditioner> {
    let v = std::env::var("SSX_NET_CONDITIONS").ok()?;
    let p: Vec<&str> = v.split(',').map(str::trim).collect();
    let parsed = match p.as_slice() {
        [l, j, x] => match (l.parse::<u64>(), j.parse::<u64>(), x.parse::<f32>()) {
            (Ok(l), Ok(j), Ok(x)) => Some(ssx_net::harness::Conditions {
                latency: std::time::Duration::from_millis(l),
                jitter: std::time::Duration::from_millis(j),
                loss: x,
            }),
            _ => None,
        },
        _ => None,
    };
    match parsed.map(|c| c.conditioner()) {
        Some(Ok(c)) => {
            warn!("network: SSX_NET_CONDITIONS={v}: the link conditioner on what this client receives (a test option)");
            Some(c)
        }
        Some(Err(e)) => {
            error!("network: SSX_NET_CONDITIONS={v}: {e}; no conditioner");
            None
        }
        None => {
            error!("network: SSX_NET_CONDITIONS={v}: LATENCY_MS,JITTER_MS,LOSS; no conditioner");
            None
        }
    }
}
