use std::collections::{BTreeSet, VecDeque};
use std::time::Duration;

use bevy::prelude::*;
use ssx_script::host::server::{InstanceFrame, PlayerView, RunView, ServerEvent, ServerFrame, StandingFrame};

use super::ScriptBridge;
use super::worker::{self, LOCAL_CLIENT, LocalOut};
use crate::ride::Ride;

pub(super) const LOCAL_INSTANCE: u16 = 1;
const MAX_EVENTS: usize = 64;

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Observed<'a> {
    pub steps: u64,
    pub go: bool,
    pub gates: &'a [u32],
    pub result: Option<(u32, u32, bool)>,
}

#[derive(Debug, Default)]
pub(super) struct LocalRound {
    pub round: u16,
    running: bool,
    ended: bool,
    gates: usize,
    seen_steps: u64,
    events: VecDeque<ServerEvent>,
    pub dropped: u64,
}

impl LocalRound {
    fn push(&mut self, e: ServerEvent) {
        if self.events.len() == MAX_EVENTS {
            self.events.pop_front();
            self.dropped += 1;
        }
        self.events.push_back(e);
    }

    pub(super) fn observe(&mut self, o: &Observed<'_>, drop: &str) {
        if o.steps < self.seen_steps && self.running {
            if !self.ended {
                self.push(ServerEvent::RoundEnd { instance: LOCAL_INSTANCE, round: self.round });
            }
            (self.running, self.ended, self.gates) = (false, false, 0);
        }
        self.seen_steps = o.steps;
        if o.go && !self.running {
            self.round = self.round.saturating_add(1);
            (self.running, self.ended, self.gates) = (true, false, 0);
            let start = ServerEvent::RoundStart { instance: LOCAL_INSTANCE, round: self.round, drop: drop.to_string() };
            self.push(start);
        }
        if !self.running || self.ended {
            return;
        }
        if o.gates.len() < self.gates {
            self.gates = 0;
        }
        for (k, &clock_ms) in o.gates.iter().enumerate().skip(self.gates) {
            let split = u8::try_from(k).unwrap_or(u8::MAX);
            self.push(ServerEvent::Checkpoint { client: LOCAL_CLIENT, split, clock_ms });
        }
        self.gates = o.gates.len();
        if let Some((clock_ms, points, failed)) = o.result {
            self.push(ServerEvent::Finish { client: LOCAL_CLIENT, clock_ms, points, failed });
            self.push(ServerEvent::RoundEnd { instance: LOCAL_INSTANCE, round: self.round });
            self.ended = true;
        }
    }

    fn take(&mut self) -> Vec<ServerEvent> {
        self.events.drain(..).collect()
    }
}

#[derive(Resource, Default)]
pub(super) struct LocalScripts {
    pub names: BTreeSet<String>,
    asked: Option<(u64, String)>,
    rides: u64,
    pub running: Option<String>,
    pub round: LocalRound,
    end_round: Option<(u16, String)>,
}

impl LocalScripts {
    pub(super) fn asked(&self) -> bool {
        self.asked.is_some()
    }

    pub(super) fn answer(&mut self, out: Option<LocalOut>) {
        let out = out.unwrap_or_default();
        self.running = out.running;
        for o in out.end_round {
            if o.instance == LOCAL_INSTANCE {
                self.end_round = Some((o.round, o.reason));
            }
        }
    }
}

fn wanted(ride: &Ride, names: &BTreeSet<String>, dev: Option<&str>) -> Option<String> {
    let id = ride.map.as_ref()?.mode.as_ref()?.gamemode.as_ref()?;
    let ns = id.namespace();
    (names.contains(ns) || dev == Some(ns)).then(|| ns.to_string())
}

pub(super) fn player_name(ui: &crate::shell::game_mode::GameUiRes, rider: Option<&str>) -> String {
    #[cfg(feature = "net")]
    {
        let _ = rider;
        let set = ui.0.fe.as_ref().and_then(|f| f.ava.options.settings.player_name.clone());
        set.unwrap_or_else(|| ssx_ui::game::fe::ava::port_settings::DEFAULT_PLAYER_NAME.into())
    }
    #[cfg(not(feature = "net"))]
    {
        let _ = ui;
        rider.filter(|n| !n.is_empty()).map_or_else(|| "Player".into(), str::to_string)
    }
}

pub(super) fn decide(
    mut bridge: ResMut<ScriptBridge>,
    mut local: ResMut<LocalScripts>,
    ride: Option<Res<Ride>>,
    live: Option<Res<crate::shell::live::LiveSession>>,
    ui: Res<crate::shell::game_mode::GameUiRes>,
    level: Option<Res<crate::shell::game_level::LevelGame>>,
) {
    if ride.as_ref().is_some_and(|r| r.is_added()) {
        local.rides += 1;
        local.round = LocalRound::default();
    }
    let dev = bridge.dev.as_ref().and_then(|(w, _)| w.status().name);
    let want = match (&ride, &live) {
        (Some(r), None) => wanted(r, &local.names, dev.as_deref()).map(|p| (local.rides, p)),
        _ => None,
    };
    if want == local.asked {
        return;
    }
    if want.is_some() && !bridge.alone() {
        return;
    }
    let job = match &want {
        Some((_, package)) => {
            let rider = level.as_ref().map(|l| l.script_view().0.rider.clone());
            let player = player_name(&ui, rider.as_deref());
            if local.asked.is_some() {
                if !bridge.send(worker::Job::EndLocal("the level ended".into())) {
                    return;
                }
                local.asked = None;
            }
            info!("scripts: the local script host starts for {}'s server scripts", ssx_core::text::shown(package));
            worker::Job::Local { package: package.clone(), player }
        }
        None => worker::Job::EndLocal(if live.is_some() { "a network session" } else { "the level ended" }.into()),
    };
    if bridge.send(job) {
        local.asked = want;
        local.end_round = None;
    }
}

pub(super) fn observe(ride: Res<Ride>, mut local: ResMut<LocalScripts>) {
    if !local.asked() {
        return;
    }
    let Some(map) = ride.map.as_ref() else { return };
    let result = ride.session.event.result.as_ref().map(|r| (r.time_ms, r.points, r.failed));
    let o = Observed { steps: ride.steps, go: ride.go_step.is_some(), gates: &map.gates.times, result };
    let drop = map.id.to_string();
    local.round.observe(&o, &drop);
}

pub(super) fn apply(mut ride: ResMut<Ride>, mut local: ResMut<LocalScripts>) {
    let Some((round, why)) = local.end_round.take() else { return };
    let e = &ride.session.event;
    let running = e.state == ssx_physics::event::EventState::Running && !e.waiting && e.result.is_none();
    if ride.map.is_none() || !running || round != local.round.round || ride.go_step.is_none() {
        info!("scripts: the local host's end of round {round} is let go (the run is not that round's, running)");
        return;
    }
    info!("scripts: the local host ends round {round}: {}", ssx_core::text::shown(&why));
    ride.session.event.time_up = true;
}

pub(super) fn server_frame(now: Duration, ride: &Ride, local: &mut LocalScripts, name: &str) -> Option<ServerFrame> {
    let map = ride.map.as_ref()?;
    let mode = map.mode.as_ref()?;
    let e = &ride.session.event;
    let s = &ride.state;
    let result = e.result.as_ref();
    let go = ride.go_step.is_some();
    let state = match (result, go) {
        (Some(_), _) => "results",
        (None, false) => "countdown",
        (None, true) => "running",
    };
    let row_state = match result {
        Some(r) if r.failed => "failed",
        Some(_) => "finished",
        None => "racing",
    };
    let points = result.map_or_else(|| s.score.score.max(0.0).min(u32::MAX as f32) as u32, |r| r.points);
    let clock = result.map_or_else(|| e.clock.ms(), |r| r.time_ms);
    let split = u8::try_from(map.gates.next).ok().and_then(|n| n.checked_sub(1));
    let in_round = go || result.is_some();
    let standings = if in_round {
        vec![StandingFrame { rider: 0, name: name.to_string(), state: row_state, clock_ms: clock, points, split }]
    } else {
        Vec::new()
    };
    let instance = InstanceFrame {
        id: LOCAL_INSTANCE,
        mode: mode.name.clone(),
        drop: map.id.to_string(),
        state,
        round: local.round.round,
        clock_ms: if state == "running" { f64::from(e.clock.ms()) } else { 0.0 },
        standings,
    };
    let status = match (result, go) {
        (Some(_), _) => "finished",
        (None, false) => "intro",
        (None, true) => "riding",
    };
    let v = s.velocity * ssx_physics::TICKS_PER_SECOND;
    let player = PlayerView {
        client: LOCAL_CLIENT,
        name: name.to_string(),
        instance: Some(LOCAL_INSTANCE),
        rider: Some(0),
        progress: "ready",
        status,
        ping_ms: 0,
        position: Some(s.traj.to_array()),
        velocity: Some(v.to_array()),
        run: in_round.then_some(RunView { split, clock_ms: clock, points, state: row_state }),
    };
    let events = local.round.take();
    Some(ServerFrame { now, instances: vec![instance], players: vec![player], events })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(r: &mut LocalRound) -> Vec<String> {
        r.take()
            .into_iter()
            .map(|e| match e {
                ServerEvent::RoundStart { round, drop, .. } => format!("start {round} {drop}"),
                ServerEvent::RoundEnd { round, .. } => format!("end {round}"),
                ServerEvent::Checkpoint { split, clock_ms, .. } => format!("gate {split} {clock_ms}"),
                ServerEvent::Finish { clock_ms, failed, .. } => format!("finish {clock_ms} {failed}"),
                ServerEvent::Shutdown { round_end_secs } => format!("shutdown {round_end_secs}"),
            })
            .collect()
    }

    #[test]
    fn the_local_round_follows_the_run() {
        let mut r = LocalRound::default();
        let at = |steps, go, gates, result| Observed { steps, go, gates, result };
        r.observe(&at(1, false, &[], None), "m:map/a");
        assert!(kinds(&mut r).is_empty());
        r.observe(&at(2, true, &[], None), "m:map/a");
        r.observe(&at(3, true, &[900], None), "m:map/a");
        r.observe(&at(4, true, &[900], None), "m:map/a");
        r.observe(&at(5, true, &[900, 2100], Some((5000, 7, false))), "m:map/a");
        r.observe(&at(6, true, &[900, 2100], Some((5000, 7, false))), "m:map/a");
        assert_eq!(kinds(&mut r), ["start 1 m:map/a", "gate 0 900", "gate 1 2100", "finish 5000 false", "end 1"]);
        r.observe(&at(1, false, &[], None), "m:map/a");
        r.observe(&at(2, true, &[400], None), "m:map/a");
        r.observe(&at(1, false, &[], None), "m:map/a");
        assert_eq!(kinds(&mut r), ["start 2 m:map/a", "gate 0 400", "end 2"]);
        r.observe(&at(2, true, &[300, 600], None), "m:map/a");
        r.observe(&at(3, true, &[], None), "m:map/a");
        r.observe(&at(4, true, &[350], None), "m:map/a");
        assert_eq!(kinds(&mut r), ["start 3 m:map/a", "gate 0 300", "gate 1 600", "gate 0 350"]);
        assert_eq!(r.dropped, 0);
    }

    #[test]
    fn the_local_client_is_the_first_admitted() {
        assert!(worker::local_client_is_the_first_admitted());
    }
}
