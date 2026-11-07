use bevy::prelude::*;
use ssx_net::lightyear::prelude::LocalTimeline;
use ssx_net::{ClientRoster, ServerClock, ServerSaid, ViewClock};
use ssx_net_proto::{InstanceState, Mode, NetPose, PlayerStatus, PoseFlag, PoseFlags, ProgressReport, ReportKind};

use super::session::NetSession;
use crate::ride::Ride;
use crate::ride::live::StartGate;
use crate::shell::game_mode::GameUiRes;
use crate::shell::live::{LiveLevel, LiveSession};
use ssx_ui::game::fe::ava::session::results::ResultsExit;

const _: () = assert!(ssx_net_proto::round::SPLITS as usize == ssx_physics::event::SPLITS);

#[derive(Resource, Default)]
pub struct Rounds {
    pub mode: Option<Mode>,
    pub state: Option<InstanceState>,
    pub round: Option<u16>,
    pub level: Option<ssx_net_proto::instance::Level>,
    pub spectator: bool,
    pub start: Option<u32>,
    pub go_given: Option<f64>,
    pub go_taken: Option<f64>,
    reported_step: u64,
    pub pending: Vec<ProgressReport>,
    held: Vec<Held>,
    pub result_sent: bool,
    pub results_shown: bool,
    pub watching: Option<u16>,
    aligned: Option<ssx_net::client::StepGrid>,
    pub refused_reports: u64,
    pub executions: Executions,
    pub handover: Option<NetPose>,
    pub second: Option<NetPose>,
    unheld_said: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Executions {
    carried: u64,
    last: u64,
    go: Option<u64>,
}

impl Executions {
    pub fn stepped(&mut self, steps: u64) {
        if steps < self.last {
            self.carried = self.carried.saturating_add(self.last);
        }
        self.last = steps;
    }

    pub fn go(&mut self, steps: u64) {
        self.go = Some(self.carried.saturating_add(steps).saturating_add(1));
    }

    pub fn index(&self, step: u64, rounds: bool) -> u32 {
        let at = self.carried.saturating_add(step);
        let since = match (rounds, self.go) {
            (false, _) => at,
            (true, Some(go)) => at.saturating_sub(go),
            (true, None) => 0,
        };
        u32::try_from(since.min(u64::from(ssx_net_proto::MAX_EXECUTION))).unwrap_or(ssx_net_proto::MAX_EXECUTION)
    }

    fn next_round(self) -> Self {
        Self { go: None, ..self }
    }
}

impl Rounds {
    fn start_known(&self) -> Option<u32> {
        (!self.spectator && self.mode.is_some()).then_some(self.start).flatten()
    }
}

pub fn waiting(ride: &Ride) -> bool {
    ride.go_step.is_none()
        && ride.state.start.as_ref().is_some_and(|l| l.intro.as_ref().map_or(l.idle(), |i| i.waits_for_go()))
}

pub fn track(
    session: Res<NetSession>,
    mut live: ResMut<LiveSession>,
    (roster, said): (Res<ClientRoster>, Res<ServerSaid>),
    mut rounds: ResMut<Rounds>,
    mut ui: ResMut<GameUiRes>,
    (mut ride, level): (Option<ResMut<Ride>>, Option<ResMut<crate::shell::game_level::LevelGame>>),
) {
    if !session.connected() || live.ending() {
        return;
    }
    if live.level != LiveLevel::Up {
        ui.session_level_left();
    }
    if let Err(why) = ui.session_results_exit_frame() {
        warn!("network: the session ends: {why}");
        live.end = Some(crate::shell::live::LiveEnd { reason: Some(why), refusal: None });
        return;
    }
    let Some(instance) = roster.instance.as_ref() else { return };
    let mode = instance.spec.mode.has_rounds().then_some(instance.spec.mode);
    rounds.mode = mode;
    ui.session_rounds(mode);
    if mode.is_none() {
        return;
    }
    let own = roster.own.and_then(|(r, _)| roster.players.get(&r));
    rounds.spectator = own.is_some_and(|p| p.status == Some(PlayerStatus::Spectating));
    let (state, round, on) = (instance.state, instance.round, instance.spec.level.clone());
    if rounds.state != Some(state) {
        info!("network: the round {round}: {} on {on}", state.name());
    }
    let next = rounds.round.is_some_and(|r| r != round);
    let old_level = rounds.level.clone();
    (rounds.state, rounds.round, rounds.level) = (Some(state), Some(round), Some(on.clone()));
    if next {
        next_round(&mut rounds, &mut live, &mut ui, ride, old_level.as_ref(), &on, round);
        return;
    }
    if let Some((r, t)) = said.start
        && r == round
        && rounds.start != Some(t)
    {
        info!("network: the round {round}'s start: tick {t}");
        rounds.start = Some(t);
    }
    if state == InstanceState::Results
        && rounds.go_taken.is_some()
        && let Some(r) = ride.as_mut()
        && r.session.event.round_over.is_none()
    {
        let map = r.map.is_some();
        r.session.event.end_round(map);
        let end = if r.session.event.round_over == Some(true) { "with its value" } else { "failed" };
        info!(
            "network: the round {round} is over: this player's run, if still on, ends as its time limit would ({end})"
        );
    }
    if state == InstanceState::Results
        && !rounds.results_shown
        && live.level == LiveLevel::Up
        && show_results(&mut rounds, &roster, &mut ui, ride)
        && let Some(mut level) = level
    {
        level.session_results_shown(true);
    }
}

fn next_round(
    rounds: &mut Rounds,
    live: &mut LiveSession,
    ui: &mut GameUiRes,
    ride: Option<ResMut<Ride>>,
    old_level: Option<&ssx_net_proto::instance::Level>,
    level: &ssx_net_proto::instance::Level,
    round: u16,
) {
    let shown = rounds.results_shown;
    *rounds = Rounds {
        mode: rounds.mode,
        state: rounds.state,
        round: rounds.round,
        level: rounds.level.clone(),
        spectator: rounds.spectator,
        refused_reports: rounds.refused_reports,
        executions: rounds.executions.next_round(),
        ..Rounds::default()
    };
    let other = old_level != Some(level);
    let next = match rounds.mode.filter(|_| other).map(|mode| super::session::session_level(ui, level, mode)) {
        Some(Ok(l)) => Some(l),
        Some(Err(why)) => {
            warn!("network: the session ends: {why}");
            live.end = Some(crate::shell::live::LiveEnd { reason: Some(why), refusal: None });
            return;
        }
        None => None,
    };
    if let Some(mut ride) = ride {
        ride.hold_pads(false);
        ride.start_gate = StartGate::Hold;
        if let Some(next) = next {
            info!("network: the round {round} on another level ({level}): the level is left for it");
            live.relaunch = Some(next.clone());
            live.ridden = Some(next);
            if shown {
                ui.session_leave_results(ResultsExit::Quit);
            }
            return;
        }
        if live.level != LiveLevel::Up {
            return;
        }
        if shown {
            info!(
                "network: the round {round} on the same level: from the results through the post-race menu's restart"
            );
            ui.session_leave_results(ResultsExit::Restart);
        } else if !waiting(&ride) {
            info!("network: the round {round} on the same level: the game's restart");
            ui.fe_call(ssx_ui::game::fe::pause::do_restart);
        }
    } else if let Some(next) = next {
        live.ridden = Some(next.clone());
        live.relaunch = Some(next);
    }
}

fn show_results(rounds: &mut Rounds, roster: &ClientRoster, ui: &mut GameUiRes, ride: Option<ResMut<Ride>>) -> bool {
    let Some(round) = rounds.round else { return false };
    let Some((r, rows)) = roster.standings.as_ref() else { return false };
    if *r != round {
        return false;
    }
    let Some(mut ride) = ride else { return false };
    let own = roster.own.map(|(r, _)| r.0);
    let icon = |character: Option<u32>| {
        let fe = ui.fe.as_ref()?;
        fe.career.riders.iter().find(|r| Some(r.key) == character).map(|r| r.icon.clone())
    };
    let results: Vec<ssx_ui::game::fe::ava::session::results::ResultRow> = rows
        .iter()
        .enumerate()
        .map(|(place, row)| {
            let player = roster.players.get(&ssx_net_proto::RiderId(row.rider));
            ssx_ui::game::fe::ava::session::results::ResultRow {
                name: player.map_or_else(|| format!("#{}", row.rider), |p| p.name.clone()),
                icon: icon(player.and_then(|p| p.appearance.map(|a| a.character))).unwrap_or_default(),
                state: row.state,
                invalid: row.invalid,
                clock_ms: row.clock_ms,
                points: row.points,
                descent_mm: row.descent_mm,
                medal: ssx_gameplay::standings::medal(rows, place),
                own: Some(row.rider) == own,
            }
        })
        .collect();
    let character = ride.character().to_string();
    let character_pic = icon(Some(ride.wire_key())).unwrap_or_default();
    let input = ssx_ui::game::fe::ava::session::results::SessionResults {
        event_type: ride.session.event.setup.event_type.map_or(8, |t| t as i32),
        rows: results,
        rewinds: ride.session.event.rewinds,
        character_pic,
        character,
    };
    let place = ui.session_results(&input);
    ui.fe_call(|fe, env| ssx_ui::game::fe::flow::results_posted(fe, env, false));
    ride.hold_pads(true);
    rounds.results_shown = true;
    info!("network: the round {round}'s results: {} riders, this player's place {place:?}", input.rows.len());
    true
}

pub fn align(
    mut rounds: ResMut<Rounds>,
    ride: Option<Res<Ride>>,
    (mut clock, timeline, mut fixed): (ResMut<ServerClock>, Res<LocalTimeline>, ResMut<Time<Fixed>>),
) {
    let Some(start) = rounds.start_known() else { return };
    if rounds.aligned.is_some() || !ride.is_some_and(|r| waiting(&r)) {
        return;
    }
    if !ssx_net::client::align_due(&clock, &timeline, &fixed, start) {
        return;
    }
    if let Some((shift, grid)) = ssx_net::client::align_steps(&mut clock, &timeline, &mut fixed) {
        info!("network: the round {:?}'s steps on the server's tick grid: moved {shift:.3} ticks", rounds.round);
        rounds.aligned = Some(grid);
    }
}

fn step_time(clock: &ServerClock, timeline: &LocalTimeline) -> Option<f64> {
    clock.server_time(f64::from(timeline.tick().0))
}

pub fn gate(
    mut rounds: ResMut<Rounds>,
    live: Res<LiveSession>,
    ride: Option<ResMut<Ride>>,
    (clock, timeline): (Res<ServerClock>, Res<LocalTimeline>),
) {
    let Some(mut ride) = ride else { return };
    if !live.rules.is_some_and(|r| r.rounds) {
        ride.start_gate = StartGate::Free;
        return;
    }
    if ride.state.start.is_none() {
        ride.start_gate = StartGate::Free;
        return;
    }
    if rounds.go_given.is_none()
        && let Some(i) = ride.state.start.as_mut().and_then(|l| l.intro.as_mut())
        && !i.hold_for_round()
        && !std::mem::replace(&mut rounds.unheld_said, true)
    {
        warn!("network: the intro's last scene runs already, it cannot wait for the round's start");
    }
    if rounds.go_given.is_some() {
        if ride.go_step.is_some() && rounds.go_taken.is_none() {
            rounds.go_taken = step_time(&clock, &timeline);
        }
        return;
    }
    let now = match rounds.aligned {
        Some(grid) => Some(clock.grid_time(timeline.tick().0, grid)),
        None => step_time(&clock, &timeline),
    };
    match (rounds.start_known(), now) {
        (Some(t), Some(now)) if now + 0.5 >= f64::from(t) && waiting(&ride) => {
            info!(
                "network: the round {:?}'s start: the GO given at step {} (the server's time {now:.3}, {:+.3} ticks from \
                 the start tick {t})",
                rounds.round,
                ride.steps + 1,
                now - f64::from(t)
            );
            rounds.go_given = Some(now);
            rounds.executions.go(ride.steps);
            ride.start_gate = StartGate::Go { pressed: false };
        }
        _ => ride.start_gate = StartGate::Hold,
    }
}

fn handover(rounds: &mut Rounds, ride: &Ride, (time, view): (f64, f64), rounds_mode: bool) {
    match step_sample(rounds, ride, (time, view), rounds_mode, "the hand-over's") {
        Some(p) => {
            info!(
                "network: the hand-over at step {} (execution {}, the server's time {time:.2})",
                ride.steps,
                p.execution()
            );
            rounds.handover = Some(p);
        }
        None => warn!("network: no sample of the hand-over's step {}: `Ready` takes the next one", ride.steps),
    }
}

fn second(rounds: &mut Rounds, ride: &Ride, (time, view): (f64, f64), rounds_mode: bool) {
    let Some(due) = rounds.handover.as_ref().and_then(|h| h.execution().checked_add(2)) else { return };
    if rounds.executions.index(ride.steps, rounds_mode) < due {
        return;
    }
    match step_sample(rounds, ride, (time, view), rounds_mode, "the second") {
        Some(p) => {
            if p.execution() != due {
                warn!("network: `Ready`'s second sample at execution {}, not {due}", p.execution());
            }
            rounds.second = Some(p);
        }
        None => warn!("network: no sample of step {} for `Ready`'s second: the next one's", ride.steps),
    }
}

fn step_sample(
    rounds: &Rounds,
    ride: &Ride,
    (time, view): (f64, f64),
    rounds_mode: bool,
    what: &str,
) -> Option<NetPose> {
    let blocks = ride.pose_blocks.as_ref()?;
    let (step, block) = blocks.newest.as_ref().filter(|(step, _)| *step == ride.steps)?;
    let flags = PoseFlags::default().with_warp_epoch(blocks.epoch);
    let execution = rounds.executions.index(*step, rounds_mode);
    NetPose::at(time, flags, block.clone())
        .and_then(|p| p.with_execution(execution)?.with_view_time(view))
        .inspect_err(|e| warn!("network: {what} sample of step {step} is refused by the wire's rules: {e}"))
        .ok()
}

pub fn reports(
    mut rounds: ResMut<Rounds>,
    live: Res<LiveSession>,
    ride: Option<Res<Ride>>,
    (clock, view, timeline): (Res<ServerClock>, Res<ViewClock>, Res<LocalTimeline>),
    roster: Res<ClientRoster>,
) {
    let Some(ride) = ride else { return };
    rounds.executions.stepped(ride.steps);
    let rounds_mode = live.rules.is_some_and(|r| r.rounds);
    if ride.state.start.is_some() {
        (rounds.handover, rounds.second) = (None, None);
    } else if (rounds.handover.is_none() || rounds.second.is_none())
        && (!rounds_mode || rounds.go_given.is_some())
        && let Some(time) = step_time(&clock, &timeline)
        && let Some(view) = view.of(time)
    {
        if rounds.handover.is_none() {
            handover(&mut rounds, &ride, (time, view), rounds_mode);
        } else {
            second(&mut rounds, &ride, (time, view), rounds_mode);
        }
    }
    let (Some(mode), Some(round)) = (rounds.mode, rounds.round) else { return };
    if rounds.spectator || !live.rules.is_some_and(|r| r.rounds) || rounds.go_given.is_none() || rounds.result_sent {
        return;
    }
    if ride.steps == rounds.reported_step {
        return;
    }
    rounds.reported_step = ride.steps;
    let (Some(e), Some(time), Some(instance)) =
        (ride.last_exec.as_ref(), step_time(&clock, &timeline), roster.own.map(|(_, i)| i))
    else {
        return;
    };
    let Some(view_time) = view.of(time) else { return };
    let x = &ride.session.event;
    let s = &ride.state;
    let points = s.score.score.max(0.0).min(ssx_net_proto::round::MAX_POINTS as f32) as u32;
    let descent_mm = x.descent_mm(s.score.descent);
    let rewinds = u16::try_from(x.rewinds).unwrap_or(u16::MAX);
    let loops = u16::try_from(x.loops).unwrap_or(u16::MAX);
    let held =
        |kind, split, clock_ms, points, descent_mm| Held { kind, split, clock_ms, points, descent_mm, rewinds, loops };
    if let Some((k, ms)) = e.event.split
        && let Ok(k) = u8::try_from(k)
    {
        rounds.held.push(held(ReportKind::Split, Some(k), ms, points, descent_mm));
    }
    if mode == Mode::Trick && s.score.combo_ended.is_some() {
        rounds.held.push(held(ReportKind::Points, None, x.clock.ms(), points, descent_mm));
    }
    if e.event.results
        && let Some(r) = x.result.as_ref()
    {
        let kind = if r.failed { ReportKind::Failed } else { ReportKind::Finish };
        rounds.held.push(held(kind, None, r.time_ms, r.points, r.descent_mm));
        rounds.result_sent = true;
    }
    if rounds.held.is_empty() {
        return;
    }
    let Some(blocks) = ride.pose_blocks.as_ref() else { return };
    let Some((step, block)) = blocks.newest.as_ref().filter(|(step, _)| *step == ride.steps) else {
        info!("network: no pose block of step {}: {} reports go with the next one's", ride.steps, rounds.held.len());
        return;
    };
    let mut flags = PoseFlags::default().with_warp_epoch(blocks.epoch);
    if blocks.rewinding {
        flags = flags.with(PoseFlag::Rewind);
    }
    let execution = rounds.executions.index(*step, true);
    let Ok(sample) =
        NetPose::at(time, flags, block.clone()).and_then(|p| p.with_execution(execution)?.with_view_time(view_time))
    else {
        rounds.refused_reports += 1;
        rounds.held.clear();
        return;
    };
    for h in std::mem::take(&mut rounds.held) {
        let report = ProgressReport {
            instance,
            round,
            kind: h.kind,
            split: h.split,
            clock_ms: h.clock_ms.min(ssx_net_proto::round::MAX_CLOCK_MS),
            points: h.points.min(ssx_net_proto::round::MAX_POINTS),
            descent_mm: h.descent_mm,
            rewinds: h.rewinds,
            loops: h.loops,
            sample: sample.clone(),
        };
        match ProgressReport::new(report) {
            Ok(r) => {
                info!(
                    "network: report of step {step}: {} {:?} at {} ms (execution {execution}, the server's time \
                     {time:.2})",
                    h.kind.name(),
                    h.split,
                    r.clock_ms
                );
                rounds.pending.push(r);
            }
            Err(e) => {
                rounds.refused_reports += 1;
                error!("network: a report the wire refuses (the port's fault): {e}");
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Held {
    kind: ReportKind,
    split: Option<u8>,
    clock_ms: u32,
    points: u32,
    descent_mm: u32,
    rewinds: u16,
    loops: u16,
}

pub fn view_time(clock: &ServerClock, timeline: &LocalTimeline, fixed: &Time<Fixed>) -> Option<f64> {
    clock.server_now(timeline, fixed)
}

#[cfg(test)]
mod tests {
    use super::Executions;

    #[test]
    fn the_execution_count_starts_at_the_go_and_carries_over_restarts() {
        let mut e = Executions::default();
        for steps in 1..=40 {
            e.stepped(steps);
        }
        assert_eq!((e.index(40, true), e.index(40, false)), (0, 40));
        e.go(40);
        e.stepped(41);
        assert_eq!(e.index(41, true), 0);
        for steps in 42..=100 {
            e.stepped(steps);
        }
        assert_eq!(e.index(100, true), 59);
        e.stepped(1);
        assert_eq!(e.index(1, true), 60);
        e.stepped(2);
        assert_eq!(e.index(2, true), 61);
        let mut e = e.next_round();
        e.stepped(3);
        assert_eq!(e.index(3, true), 0);
        e.go(3);
        e.stepped(4);
        e.stepped(5);
        assert_eq!(e.index(5, true), 1);
        let mut e = Executions::default();
        e.stepped(u64::MAX / 2);
        assert_eq!(e.index(u64::MAX / 2, false), ssx_net_proto::MAX_EXECUTION);
    }
}
