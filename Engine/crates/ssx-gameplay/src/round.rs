use std::collections::BTreeMap;

use ssx_net_proto::instance::Level;
use ssx_net_proto::{Check, InstanceState, Mode, ProgressReport, ReportKind, StandingRow, StandingState, TICK_HZ};

use crate::instance::{Accepted, Instance, Member, Progress, Refusal, Refused};

pub const MIN_COUNTDOWN_TICKS: u32 = 3 * TICK_HZ;

pub const REPORT_CLOCK_SLACK_TICKS: f64 = 0.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CycleRules {
    pub min_players: u16,
    pub lobby_wait: u32,
    pub load_timeout: u32,
    pub countdown: u32,
    pub round_limit: u32,
    pub results: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Role {
    #[default]
    Rider,
    Spectator,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Run {
    pub started: Option<u32>,
    pub acked: Option<u32>,
    pub split: Option<u8>,
    pub clock_ms: u32,
    pub latest_ms: u32,
    pub points: u32,
    pub descent_mm: u32,
    pub rewinds: u16,
    pub loops: u16,
    pub time: f64,
    pub state: Option<StandingState>,
    pub reports: u32,
    pub invalid: Option<Check>,
}

impl Run {
    pub fn over(&self) -> bool {
        matches!(
            self.state,
            Some(StandingState::Finished | StandingState::Failed | StandingState::Left | StandingState::Invalidated)
        )
    }
}

#[derive(Clone, Debug)]
pub struct Cycle {
    pub rules: CycleRules,
    pub rotation: Vec<Level>,
    pub at: usize,
    pub round: u16,
    pub since: u32,
    pub gathered: Option<u32>,
    pub start: Option<u32>,
    pub gone: BTreeMap<u16, Run>,
    pub rows: Vec<StandingRow>,
    pub rows_changed: bool,
    pub rules_required: bool,
    pub rules_ready: bool,
    pub carry: Changes,
    pub stop: Option<u32>,
    pub results_at_limit: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct At {
    pub mode: Mode,
    pub state: InstanceState,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    pub state: bool,
    pub start: Option<(u16, u32, Vec<u16>)>,
    pub roles: bool,
    pub standings: bool,
}

impl Cycle {
    pub(crate) fn new(rules: CycleRules, rotation: Vec<Level>, now: u32) -> Self {
        Self {
            rules,
            rotation,
            at: 0,
            round: 0,
            since: now,
            gathered: None,
            start: None,
            gone: BTreeMap::new(),
            rows: Vec::new(),
            rows_changed: true,
            results_at_limit: false,
            rules_required: false,
            rules_ready: true,
            carry: Changes::default(),
            stop: None,
        }
    }

    pub fn older(&self, round: u16) -> bool {
        (1..0x8000).contains(&self.round.wrapping_sub(round))
    }

    pub(crate) fn start_run(&mut self, rider: u16, role: Role, now: u32) -> Result<(), Refused> {
        if role == Role::Spectator {
            return Err(Refused::new(Refusal::Spectator, format!("rider {rider} watches this round")));
        }
        match self.start {
            Some(t) if now >= t => Ok(()),
            Some(t) => Err(Refused::new(Refusal::EarlyStart, format!("at tick {now}, the start is {t}"))),
            None => Err(Refused::new(Refusal::EarlyStart, format!("at tick {now}, before the countdown"))),
        }
    }
}

impl Instance {
    pub fn tick(&mut self, now: u32) -> Changes {
        let Some(mut c) = self.cycle.take() else { return Changes::default() };
        let mut out = std::mem::take(&mut c.carry);
        let present = self.members.values().filter(|m| m.progress != Progress::Away).count();
        let elapsed = now.saturating_sub(c.since);
        let stopping = self.stop_step(&mut c, now, &mut out);
        match self.state {
            _ if stopping => {}
            InstanceState::Lobby => {
                let enough = present >= usize::from(c.rules.min_players);
                if !enough {
                    c.gathered = None;
                } else if c.gathered.is_none() {
                    c.gathered = Some(now);
                }
                let waited = c.gathered.is_some_and(|g| now.saturating_sub(g) >= c.rules.lobby_wait);
                if enough && (waited || present >= self.capacity) && c.rules_ready {
                    self.enter(&mut c, InstanceState::Loading, now, &mut out);
                }
            }
            InstanceState::Loading => {
                let loaded = |m: &Member| matches!(m.progress, Progress::Loaded(_));
                let all = self.members.values().filter(|m| m.progress != Progress::Away).all(loaded);
                if all || elapsed >= c.rules.load_timeout {
                    let mut riders = Vec::new();
                    for (&r, m) in &mut self.members {
                        m.run = Run::default();
                        m.role = if loaded(m) { Role::Rider } else { Role::Spectator };
                        if m.role == Role::Rider {
                            riders.push(r);
                        }
                    }
                    out.roles = true;
                    if riders.len() < usize::from(c.rules.min_players.max(1)) {
                        for m in self.members.values_mut() {
                            m.role = Role::Rider;
                        }
                        c.gathered = None;
                        self.enter(&mut c, InstanceState::Lobby, now, &mut out);
                    } else {
                        let t = now.saturating_add(c.rules.countdown.max(MIN_COUNTDOWN_TICKS));
                        c.start = Some(t);
                        c.gone.clear();
                        for &r in &riders {
                            if let Some(m) = self.members.get_mut(&r) {
                                m.run.state = Some(StandingState::Racing);
                            }
                        }
                        out.start = Some((c.round, t, riders));
                        self.enter(&mut c, InstanceState::Countdown, now, &mut out);
                        c.rows_changed = true;
                    }
                }
            }
            InstanceState::Countdown => {
                if c.start.is_some_and(|t| now >= t) {
                    self.enter(&mut c, InstanceState::Running, now, &mut out);
                }
            }
            InstanceState::Running => {
                let racing =
                    self.members.values().any(|m| m.role == Role::Rider && m.run.state.is_some() && !m.run.over());
                let limit = c.start.is_some_and(|t| now.saturating_sub(t) >= c.rules.round_limit);
                if limit && racing {
                    self.end_at_limit(&c);
                }
                if !racing || limit {
                    self.enter(&mut c, InstanceState::Results, now, &mut out);
                    c.rows_changed = true;
                }
            }
            InstanceState::Results => {
                if elapsed >= c.rules.results {
                    self.next_drop(&mut c, now, &mut out);
                }
            }
        }
        if std::mem::take(&mut c.rows_changed) {
            let rows = self.standings_rows(&c);
            if rows != c.rows {
                c.rows = rows;
                out.standings = true;
            }
        }
        self.cycle = Some(c);
        out
    }

    fn next_drop(&mut self, c: &mut Cycle, now: u32, out: &mut Changes) {
        c.at = (c.at + 1) % c.rotation.len().max(1);
        if let Some(level) = c.rotation.get(c.at) {
            self.spec.level.clone_from(level);
        }
        c.round = c.round.wrapping_add(1);
        (c.start, c.gathered) = (None, None);
        c.gone.clear();
        c.rules_ready = !c.rules_required;
        c.results_at_limit = false;
        for m in self.members.values_mut() {
            m.role = Role::Rider;
            m.run = Run::default();
            if let Progress::Loaded(a) | Progress::Ready(a) = m.progress {
                m.progress = Progress::Dressed(a);
            }
        }
        out.roles = true;
        c.rows_changed = true;
        self.enter(c, InstanceState::Lobby, now, out);
    }

    fn end_at_limit(&mut self, c: &Cycle) {
        if !c.results_at_limit {
            return;
        }
        for m in self.members.values_mut() {
            if m.role == Role::Rider && m.run.state == Some(StandingState::Racing) {
                m.run.state = Some(StandingState::Finished);
            }
        }
    }

    fn enter(&mut self, c: &mut Cycle, state: InstanceState, now: u32, out: &mut Changes) {
        self.state = state;
        c.since = now;
        out.state = true;
    }

    fn standings_rows(&self, c: &Cycle) -> Vec<StandingRow> {
        let last = self.state == InstanceState::Results;
        let row = |rider: u16, run: &Run| {
            let tick = run.time.max(0.0).floor().min(f64::from(u32::MAX));
            StandingRow {
                rider,
                state: run.state.unwrap_or(StandingState::Racing),
                split: run.split,
                clock_ms: run.clock_ms,
                points: run.points,
                descent_mm: run.descent_mm,
                tick: tick as u32,
                fraction: ((run.time - tick).clamp(0.0, 1.0) * 255.0) as u8,
                invalid: run.invalid,
            }
        };
        let mut rows: Vec<StandingRow> = self
            .members
            .iter()
            .filter(|(_, m)| m.role == Role::Rider && m.run.state.is_some())
            .map(|(&r, m)| row(r, &m.run))
            .chain(c.gone.iter().map(|(&r, run)| row(r, run)))
            .collect();
        crate::standings::order(self.spec.mode, &mut rows, last);
        rows
    }

    pub fn standings(&self) -> &[StandingRow] {
        self.cycle.as_ref().map_or(&[], |c| &c.rows)
    }

    pub fn require_rules(&mut self) {
        if let Some(c) = self.cycle.as_mut() {
            (c.rules_required, c.rules_ready) = (true, false);
        }
    }

    pub fn set_rules_ready(&mut self, ready: bool) {
        if let Some(c) = self.cycle.as_mut() {
            c.rules_ready = ready;
        }
    }

    pub fn rules_ready(&self) -> bool {
        self.cycle.as_ref().is_none_or(|c| c.rules_ready)
    }

    pub fn set_results_at_limit(&mut self, results: bool) {
        if let Some(c) = self.cycle.as_mut() {
            c.results_at_limit = results;
        }
    }

    pub fn skip_drop(&mut self, now: u32) -> bool {
        if self.state != InstanceState::Lobby {
            return false;
        }
        let Some(mut c) = self.cycle.take() else { return false };
        let mut out = std::mem::take(&mut c.carry);
        self.next_drop(&mut c, now, &mut out);
        c.carry = out;
        self.cycle = Some(c);
        true
    }

    pub fn rotation(&self) -> &[Level] {
        self.cycle.as_ref().map_or(&[], |c| &c.rotation)
    }

    pub fn invalidate(&mut self, rider: u16, check: Check) -> bool {
        let Some(c) = self.cycle.as_mut() else { return false };
        let run = match self.members.get_mut(&rider) {
            Some(m) if m.role == Role::Rider && m.run.state.is_some() => &mut m.run,
            _ => match c.gone.get_mut(&rider) {
                Some(run) => run,
                None => return false,
            },
        };
        if run.invalid.is_some() {
            return false;
        }
        (run.invalid, run.state) = (Some(check), Some(StandingState::Invalidated));
        c.rows_changed = true;
        true
    }

    pub fn refresh_standings(&mut self) -> bool {
        let Some(mut c) = self.cycle.take() else { return false };
        let mut changed = false;
        if std::mem::take(&mut c.rows_changed) {
            let rows = self.standings_rows(&c);
            if rows != c.rows {
                c.rows = rows;
                changed = true;
            }
        }
        self.cycle = Some(c);
        changed
    }
}

impl Cycle {
    pub(crate) fn level_left(&mut self, _rider: u16, m: Option<&mut Member>, state: InstanceState) -> bool {
        let Some(m) = m else { return false };
        let in_round = matches!(state, InstanceState::Countdown | InstanceState::Running);
        if in_round && m.role == Role::Rider && m.run.state.is_some() && !m.run.over() {
            m.run.state = Some(StandingState::Left);
            self.rows_changed = true;
            return true;
        }
        false
    }

    pub(crate) fn left(&mut self, rider: u16, m: &Member) {
        if m.role == Role::Rider && m.run.state.is_some() {
            let mut run = m.run;
            if !run.over() {
                run.state = Some(StandingState::Left);
            }
            self.gone.insert(rider, run);
            self.rows_changed = true;
        }
    }

    pub(crate) fn start_ack(
        &mut self,
        rider: u16,
        m: Option<&mut Member>,
        progress: Progress,
        round: u16,
        now: u32,
    ) -> Result<Accepted, Refused> {
        if self.older(round) {
            return Ok(Accepted { stale: true, ..Accepted::default() });
        }
        if round != self.round {
            return Err(Refused::new(Refusal::FutureRound, format!("a start's acknowledgement of round {round}")));
        }
        let Some(m) = m else { return Err(Refused::new(Refusal::Order, "not in the instance")) };
        if m.role == Role::Spectator {
            return Err(Refused::new(Refusal::Spectator, format!("rider {rider} watches this round")));
        }
        if self.start.is_none()
            || m.run.acked.is_some()
            || !matches!(progress, Progress::Loaded(_) | Progress::Ready(_))
        {
            return Err(Refused::new(Refusal::Order, format!("a start's acknowledgement while {}", progress.name())));
        }
        m.run.acked = Some(now);
        Ok(Accepted::default())
    }

    pub(crate) fn report(
        &mut self,
        rider: u16,
        m: Option<&mut Member>,
        progress: Progress,
        r: &ProgressReport,
        at: At,
    ) -> Result<Accepted, Refused> {
        let At { mode, state } = at;
        let kind = r.kind.name();
        if self.older(r.round) || (r.round == self.round && state == InstanceState::Results) {
            return Ok(Accepted { stale: true, ..Accepted::default() });
        }
        if r.round != self.round {
            return Err(Refused::new(Refusal::FutureRound, format!("a {kind} of round {}", r.round)));
        }
        let Some(m) = m else { return Err(Refused::new(Refusal::Order, "not in the instance")) };
        if m.role == Role::Spectator {
            return Err(Refused::new(Refusal::Spectator, format!("a {kind} of rider {rider}, who watches")));
        }
        let (Some(t), Some(_), Progress::Ready(_)) = (self.start, m.run.started, progress) else {
            return Err(Refused::new(Refusal::ReportEarly, format!("a {kind} before the rider's start")));
        };
        if state != InstanceState::Running {
            return Err(Refused::new(Refusal::ReportOrder, format!("a {kind} while the instance is {}", state.name())));
        }
        let executions = f64::from(r.sample.execution());
        let allowed_ms = (executions + REPORT_CLOCK_SLACK_TICKS) * 1000.0 / f64::from(TICK_HZ);
        if f64::from(r.clock_ms) > allowed_ms {
            return Err(Refused::new(
                Refusal::ReportClock,
                format!("a {kind} with the clock {} ms at execution {executions:.0} (the start {t})", r.clock_ms),
            ));
        }
        let time = f64::from(t) + executions;
        let run = &mut m.run;
        if run.invalid.is_some() {
            return Ok(Accepted { dropped: true, ..Accepted::default() });
        }
        let order = |why: &str| Err(Refused::new(Refusal::ReportOrder, format!("a {kind}: {why}")));
        if run.over() {
            return order("after the run's end");
        }
        if run.reports > 0 && r.loops == run.loops && r.clock_ms < run.latest_ms {
            return order("its clock before the last report's");
        }
        if run.reports > 0 && time < run.time {
            return order("its execution before the last report's");
        }
        if r.loops < run.loops {
            return order("its loops fewer than the last report's");
        }
        match r.kind {
            ReportKind::Split => {
                let next_loop = r.loops > run.loops;
                if !next_loop && r.split.is_some_and(|k| run.split.is_some_and(|last| k <= last)) {
                    return order("a split not after the last");
                }
                run.split = r.split;
            }
            ReportKind::Points if mode != Mode::Trick => return order("points outside a trick round"),
            ReportKind::Points => {}
            ReportKind::Finish => run.state = Some(StandingState::Finished),
            ReportKind::Failed => run.state = Some(StandingState::Failed),
        }
        if !(mode == Mode::Race && r.kind == ReportKind::Failed) {
            run.clock_ms = r.clock_ms;
        }
        (run.latest_ms, run.points, run.descent_mm) = (r.clock_ms, r.points, r.descent_mm);
        (run.rewinds, run.loops, run.time) = (r.rewinds, r.loops, time);
        run.reports += 1;
        self.rows_changed = true;
        Ok(Accepted { progress: run.over(), standings: true, ..Accepted::default() })
    }
}

mod stop;
pub use stop::StopPlan;
mod script;

#[cfg(test)]
mod tests;
