use bevy::prelude::*;
use ssx_net::lightyear::prelude::LocalTimeline;
use ssx_net::{ClientRoster, ServerClock, ViewClock};
use ssx_net_proto::{InstanceState, Mode, RiderId, StandingRow, StandingState};
use ssx_physics::session::bots::live::LiveView;
use ssx_ui::game::session_board::{Entrant, SessionHud, SessionKind, SessionRacer};
use ssx_ui::game::values::id;

use super::round::Rounds;
use crate::camera::chase::SpectateTarget;
use crate::camera::graph::Inputs;
use crate::ride::Ride;
use crate::shell::game_level::LevelGame;
use crate::shell::game_mode::GameUiRes;
use crate::shell::live::{LiveLevel, LiveSession};

pub fn nearest(riders: &[u16], place: usize, count: usize) -> Vec<u16> {
    let mut out = Vec::with_capacity(count);
    for d in 1..riders.len() {
        for k in [place.checked_sub(d), place.checked_add(d)].into_iter().flatten() {
            if out.len() < count
                && let Some(&r) = riders.get(k)
            {
                out.push(r);
            }
        }
        if out.len() >= count {
            break;
        }
    }
    out
}

fn this_rounds(roster: &ClientRoster, rounds: &Rounds) -> Option<Vec<StandingRow>> {
    let (r, rows) = roster.standings.as_ref()?;
    (Some(*r) == rounds.round).then(|| rows.clone())
}

fn kind(mode: Mode) -> SessionKind {
    match mode {
        Mode::Trick => SessionKind::Trick,
        Mode::Survive => SessionKind::Survive,
        Mode::Race | Mode::Freeride => SessionKind::Race,
    }
}

fn racer(roster: &ClientRoster, fe: Option<&ssx_ui::game::fe::FrontEnd>, row: &StandingRow) -> SessionRacer {
    let player = roster.players.get(&RiderId(row.rider));
    let glyphs = fe.and_then(|f| f.ava.glyphs.as_ref());
    let name = player.map_or_else(|| format!("#{}", row.rider), |p| ssx_ui::game::fe::ava::shown(&p.name, glyphs));
    let character = player.and_then(|p| p.appearance.map(|a| a.character));
    let picture = fe
        .and_then(|fe| fe.career.riders.iter().find(|r| Some(r.key) == character))
        .map_or_else(String::new, |r| r.icon.clone());
    SessionRacer {
        name,
        picture,
        place: None,
        finish_ms: (row.state == StandingState::Finished).then_some(row.clock_ms),
        out: matches!(row.state, StandingState::Failed | StandingState::Left),
        points: row.points,
        descent_mm: row.descent_mm,
    }
}

fn board_order(
    mode: Mode,
    rows: &[StandingRow],
    own: Option<u16>,
    ride: &Ride,
    roster: &ClientRoster,
    fe: Option<&ssx_ui::game::fe::FrontEnd>,
) -> Vec<u16> {
    let course = ride.session.event.setup.course.map(|c| (c.start_y, c.end_y));
    let drawn_y =
        |id: u16| ride.session.bots.as_ref().and_then(|b| b.live.iter().find(|l| l.id == id)).map(|l| l.position.y);
    let start_y = course.map_or(0.0, |c| c.0);
    let mut ids: Vec<u16> = rows.iter().map(|r| r.rider).collect();
    if let Some(o) = own
        && !ids.contains(&o)
    {
        ids.push(o);
    }
    let entrants: Vec<Entrant> = ids
        .iter()
        .map(|&id| {
            if Some(id) == own {
                let (s, x) = (&ride.state, &ride.session);
                return Entrant {
                    name: String::new(),
                    picture: String::new(),
                    player: true,
                    finish_ms: x.event.finish_ms,
                    out: x.event.result.as_ref().is_some_and(|r| r.failed),
                    y: s.position.y,
                    points: s.score.score.max(0.0) as u32,
                    descent_mm: x.event.descent_mm(s.score.descent),
                    place: None,
                };
            }
            let row = rows.iter().find(|r| r.rider == id);
            let s = row.map(|row| racer(roster, fe, row)).unwrap_or_default();
            Entrant {
                name: String::new(),
                picture: String::new(),
                player: false,
                finish_ms: s.finish_ms,
                out: s.out,
                y: drawn_y(id).unwrap_or(start_y),
                points: s.points,
                descent_mm: s.descent_mm,
                place: None,
            }
        })
        .collect();
    match ssx_ui::game::session_board::order(kind(mode), &entrants, course) {
        Some(order) => order.into_iter().filter_map(|k| ids.get(k).copied()).collect(),
        None => rows.iter().map(|r| r.rider).collect(),
    }
}

pub fn hud(
    rounds: Res<Rounds>,
    roster: Res<ClientRoster>,
    ui: Res<GameUiRes>,
    (ride, level): (Option<ResMut<Ride>>, Option<ResMut<LevelGame>>),
) {
    let (Some(mut ride), Some(mut level)) = (ride, level) else { return };
    let (Some(mode), Some(rows), Some(own)) =
        (rounds.mode, this_rounds(&roster, &rounds), roster.own.map(|(r, _)| r.0))
    else {
        level.set_session_hud(None);
        if let Some(race) = ride.session.bots.as_mut() {
            race.set_live_order(None);
        }
        return;
    };
    if rounds.spectator || !rows.iter().any(|r| r.rider == own) {
        level.set_session_hud(None);
        return;
    }
    let ranked = board_order(mode, &rows, Some(own), &ride, &roster, ui.fe.as_ref());
    let Some(place) = ranked.iter().position(|&r| r == own) else {
        level.set_session_hud(None);
        return;
    };
    let slots = ssx_physics::event::MAX_RACERS;
    let order = nearest(&ranked, place, slots);
    let at = |id: u16| ranked.iter().position(|&r| r == id);
    let top = order.iter().filter_map(|&r| at(r)).chain([place]).min().unwrap_or(0);
    let riders = order
        .iter()
        .filter_map(|&r| {
            let row = rows.iter().find(|x| x.rider == r)?;
            Some((r, SessionRacer { place: at(r), ..racer(&roster, ui.fe.as_ref(), row) }))
        })
        .collect();
    if let Some(race) = ride.session.bots.as_mut() {
        race.set_live_order(Some(order));
    }
    level.set_session_hud(Some(SessionHud { kind: kind(mode), riders, player_place: Some(place), place_offset: top }));
}
pub fn spectate_inputs(view: &LiveView, finished: bool, back_press: bool) -> Inputs {
    use ssx_formats::ghost::FLAG_IN_AIR;
    let (root, pose) = (view.root, &view.pose);
    let [x, y, z, w] = view.rotation;
    let rotation = Quat::from_xyzw(x, y, z, w).normalize();
    let velocity = Vec3::from_array(pose.velocity);
    Inputs {
        target_pos: root,
        target_up: rotation * Vec3::Y,
        velocity,
        // TODO(guess): `FLT_TargetSpeed` is the target's `FLT_PlayerSpeed` (m/s, `update_player_speed`), which a sample
        // does not carry: the block's `VEC_Velocity` (m per 1/60-s tick) × 60 stands in; the ground ray's normal, the
        // lean, the wipeout, the boost and the grind (the velocity word's two bits are not unpacked by `GhostPose`) are
        // the camera's defaults (up, 0, false); settle: the sample's fields the 3P graph reads (what
        // `QueryTask_Cam_GetTarget` 0x82EB4E58 relays) carried by a later protocol, or the block's grind and boost bits
        // unpacked.
        speed: velocity.length() * 60.0,
        ground_normal: Vec3::Y,
        lean: 0.0,
        airborne: pose.flags & FLAG_IN_AIR != 0,
        wipeout: false,
        boosting: false,
        grinding: false,
        dead: false,
        track_finished: finished,
        aggregate_mass: 0.0,
        bailed_by_avalanche: false,
        alive: true,
        heli_position: Vec3::ZERO,
        heli_interact: false,
        heli_time: 0.0,
        back_press,
    }
}

fn watched(rounds: &Rounds, rows: &[StandingRow], race: &ssx_physics::session::bots::BotRace) -> Option<u16> {
    let shown = |id: u16| race.live.iter().any(|r| r.id == id && !r.pose.is_empty());
    rounds
        .watching
        .filter(|&id| shown(id))
        .or_else(|| rows.iter().map(|r| r.rider).find(|&id| shown(id)))
        .or_else(|| race.live.iter().find(|r| !r.pose.is_empty()).map(|r| r.id))
}

pub fn camera_target(
    mut rounds: ResMut<Rounds>,
    roster: Res<ClientRoster>,
    ride: Option<Res<Ride>>,
    (clock, view, timeline): (Res<ServerClock>, Res<ViewClock>, Res<LocalTimeline>),
    target: Option<ResMut<SpectateTarget>>,
    mut commands: Commands,
) {
    let watching = rounds.spectator && rounds.mode.is_some();
    let Some(ride) = ride.filter(|_| watching) else {
        if target.is_some_and(|t| t.rider.is_some()) {
            commands.insert_resource(SpectateTarget::default());
        }
        return;
    };
    let rows = this_rounds(&roster, &rounds).unwrap_or_default();
    let Some(race) = ride.session.bots.as_ref() else { return };
    let rider = watched(&rounds, &rows, race);
    rounds.watching = rider;
    let now = clock.server_time(f64::from(timeline.tick().0)).and_then(|t| view.of(t));
    let view = rider.zip(now).and_then(|(id, now)| race.live.iter().find(|r| r.id == id)?.pose.view(now));
    let finished = rider.is_some_and(|id| rows.iter().any(|r| r.rider == id && r.state == StandingState::Finished));
    let inputs = view.map(|v| spectate_inputs(&v, finished, ride.back_press));
    commands.insert_resource(SpectateTarget { rider: inputs.and(rider), inputs });
}

pub fn keys(
    mut rounds: ResMut<Rounds>,
    roster: Res<ClientRoster>,
    ride: Option<Res<Ride>>,
    (keys, pads): (Res<ButtonInput<KeyCode>>, Query<&Gamepad>),
    ui: Res<GameUiRes>,
) {
    if !rounds.spectator || ui.text_focused() || ui.chat_held() {
        return;
    }
    let pressed = |b: GamepadButton| pads.iter().any(|p| p.just_pressed(b));
    let step: i64 = if keys.any_just_pressed([KeyCode::ArrowRight, KeyCode::KeyE])
        || pressed(GamepadButton::RightTrigger)
        || pressed(GamepadButton::DPadRight)
    {
        1
    } else if keys.any_just_pressed([KeyCode::ArrowLeft, KeyCode::KeyQ])
        || pressed(GamepadButton::LeftTrigger)
        || pressed(GamepadButton::DPadLeft)
    {
        -1
    } else {
        return;
    };
    let Some(race) = ride.as_ref().and_then(|r| r.session.bots.as_ref()) else { return };
    let rows = this_rounds(&roster, &rounds).unwrap_or_default();
    let mut order: Vec<u16> = rows.iter().map(|r| r.rider).collect();
    order.extend(race.live.iter().map(|r| r.id).filter(|id| !rows.iter().any(|r| r.rider == *id)));
    order.retain(|&id| race.live.iter().any(|r| r.id == id && !r.pose.is_empty()));
    if order.is_empty() {
        return;
    }
    let at = rounds.watching.and_then(|w| order.iter().position(|&id| id == w)).unwrap_or(0) as i64;
    let next = order[(at + step).rem_euclid(order.len() as i64) as usize];
    info!("network: the spectator watches rider {next}");
    rounds.watching = Some(next);
}

pub fn banner(
    rounds: Res<Rounds>,
    live: Res<LiveSession>,
    roster: Res<ClientRoster>,
    (clock, timeline, fixed): (Res<ServerClock>, Res<LocalTimeline>, Res<Time<Fixed>>),
    ride: Option<Res<Ride>>,
    mut ui: ResMut<GameUiRes>,
) {
    let Some(state) = rounds.state.filter(|_| rounds.mode.is_some() && live.level == LiveLevel::Up) else {
        ui.session_banner("");
        return;
    };
    let players = roster.players.values().filter(|p| Some(p.instance) == roster.own.map(|(_, i)| i)).count();
    let glyphs = ui.fe.as_ref().and_then(|f| f.ava.glyphs.clone());
    let name =
        |id: u16| roster.players.get(&RiderId(id)).map(|p| ssx_ui::game::fe::ava::shown(&p.name, glyphs.as_ref()));
    let at_start = ride.as_ref().is_some_and(|r| super::round::waiting(r));
    let finished = ride.as_ref().is_some_and(|r| r.session.event.result.is_some());
    let own = roster.own.map(|(r, _)| r.0);
    let invalid = this_rounds(&roster, &rounds)
        .and_then(|rows| rows.iter().find(|r| Some(r.rider) == own).and_then(|r| r.invalid));
    let text = match state {
        _ if rounds.spectator => match rounds.watching.and_then(name) {
            Some(n) => ui.0.loc.format("TXT_AVA_ROUND_WATCHING", &[&n]),
            None => ui.0.loc.text("TXT_AVA_ROUND_WATCHING_NONE"),
        },
        InstanceState::Lobby if at_start => ui.0.loc.format("TXT_AVA_ROUND_LOBBY", &[&players.to_string()]),
        InstanceState::Loading if at_start => ui.0.loc.text("TXT_AVA_ROUND_LOADING"),
        InstanceState::Countdown | InstanceState::Running if at_start => {
            let now = super::round::view_time(&clock, &timeline, &fixed);
            match (rounds.start, now) {
                (Some(t), Some(now)) if now < f64::from(t) => {
                    let s = ((f64::from(t) - now) / f64::from(ssx_net_proto::TICK_HZ)).ceil().max(1.0);
                    ui.0.loc.format("TXT_AVA_ROUND_START_IN", &[&(s as u32).to_string()])
                }
                _ => ui.0.loc.text("TXT_AVA_ROUND_GO"),
            }
        }
        InstanceState::Running if invalid.is_some() => {
            let why = invalid
                .map_or_else(String::new, |c| ui.0.loc.text(ssx_ui::game::fe::ava::session::results::check_key(c)));
            ui.0.loc.format("TXT_AVA_ROUND_INVALIDATED", &[&why])
        }
        InstanceState::Running if finished => ui.0.loc.text("TXT_AVA_ROUND_FINISHED"),
        _ => String::new(),
    };
    ui.session_banner(&text);
    if rounds.spectator
        && let Some(mode) = rounds.mode
        && let Some(rows) = this_rounds(&roster, &rounds)
        && let Some(r) = ride.as_ref()
    {
        let course = r.session.event.setup.course.map(|c| (c.start_y, c.end_y));
        let start_y = course.map_or(0.0, |c| c.0);
        let live_y = |id: u16| {
            r.session.bots.as_ref().and_then(|b| b.live.iter().find(|l| l.id == id)).map_or(start_y, |l| l.position.y)
        };
        let ranked = board_order(mode, &rows, None, r, &roster, ui.fe.as_ref());
        let entrants: Vec<Entrant> = ranked
            .iter()
            .take(ssx_physics::event::MAX_RACERS + 1)
            .enumerate()
            .filter_map(|(place, &id)| {
                let row = rows.iter().find(|x| x.rider == id)?;
                let s = racer(&roster, ui.fe.as_ref(), row);
                Some(Entrant {
                    name: s.name,
                    picture: s.picture,
                    player: false,
                    finish_ms: s.finish_ms,
                    out: s.out,
                    y: live_y(id),
                    points: s.points,
                    descent_mm: s.descent_mm,
                    place: Some(place),
                })
            })
            .collect();
        let ui = &mut ui.0;
        ui.values.set_bool(id::RACE_STARTED, true);
        ui.values.set_bool(id::DYNAMIC_LEADERBOARD_ENABLED, true);
        let hud = SessionHud { kind: kind(mode), riders: Default::default(), player_place: None, place_offset: 0 };
        let t = r.session.event.setup.event_type.unwrap_or(ssx_physics::event::EventType::RaceClock);
        ssx_ui::game::session_board::update(&mut ui.values, &mut ui.loc, t, &hud, &entrants, course);
    }
}
