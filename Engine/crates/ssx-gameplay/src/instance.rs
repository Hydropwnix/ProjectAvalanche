use std::collections::BTreeMap;

use ssx_net_proto::instance::Level;
use ssx_net_proto::{Appearance, ClientControl, InstanceSpec, InstanceState, Mode, PlayerStatus};

use crate::round::{CycleRules, Role, Run};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    Joined,
    Dressed(Appearance),
    Loaded(Appearance),
    Ready(Appearance),
    Away,
}

impl Progress {
    pub fn appearance(self) -> Option<Appearance> {
        match self {
            Self::Joined | Self::Away => None,
            Self::Dressed(a) | Self::Loaded(a) | Self::Ready(a) => Some(a),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Joined => "joined",
            Self::Dressed(_) => "dressed",
            Self::Loaded(_) => "loaded",
            Self::Ready(_) => "ready",
            Self::Away => "away",
        }
    }

    pub fn status(self) -> PlayerStatus {
        match self {
            Self::Joined => PlayerStatus::Joining,
            Self::Dressed(_) => PlayerStatus::Loading,
            Self::Loaded(_) => PlayerStatus::Intro,
            Self::Ready(_) => PlayerStatus::Riding,
            Self::Away => PlayerStatus::Away,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Refusal {
    Order,
    EarlyStart,
    ReportOrder,
    ReportEarly,
    ReportClock,
    ReportFuture,
    Spectator,
    ReportInstance,
    FutureRound,
    NoRounds,
}

impl Refusal {
    pub fn name(self) -> &'static str {
        match self {
            Self::Order => "out of order",
            Self::EarlyStart => "the GO before the round's start",
            Self::ReportOrder => "a report out of the run's order",
            Self::ReportEarly => "a report before the start",
            Self::ReportClock => "a report's clock beyond the ticks since the start",
            Self::ReportFuture => "a report from later than the server's time",
            Self::Spectator => "a spectator's start or report",
            Self::ReportInstance => "a report of another instance",
            Self::FutureRound => "a round that has not begun",
            Self::NoRounds => "a round's message in freeride",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused {
    pub rule: Refusal,
    pub detail: String,
}

impl Refused {
    pub(crate) fn new(rule: Refusal, detail: impl Into<String>) -> Self {
        Self { rule, detail: detail.into() }
    }
}

impl core::fmt::Display for Refused {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        write!(f, "{}: {}", self.rule.name(), self.detail)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Accepted {
    pub progress: bool,
    pub standings: bool,
    pub stale: bool,
    pub dropped: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    Full,
    Present,
}

#[derive(Clone, Debug)]
pub struct Member {
    pub progress: Progress,
    pub role: Role,
    pub run: Run,
}

#[derive(Clone, Debug)]
pub struct Instance {
    pub(crate) id: u16,
    pub(crate) spec: InstanceSpec,
    pub(crate) state: InstanceState,
    pub(crate) capacity: usize,
    pub(crate) members: BTreeMap<u16, Member>,
    pub(crate) cycle: Option<crate::round::Cycle>,
}

impl Instance {
    pub fn freeride(id: u16, spec: InstanceSpec, capacity: usize) -> Self {
        debug_assert_eq!(spec.mode, Mode::Freeride);
        Self { id, spec, state: InstanceState::Running, capacity, members: BTreeMap::new(), cycle: None }
    }

    pub fn rounds(
        id: u16,
        spec: InstanceSpec,
        capacity: usize,
        rules: CycleRules,
        rotation: Vec<Level>,
        now: u32,
    ) -> Self {
        debug_assert!(spec.mode.has_rounds());
        let cycle = crate::round::Cycle::new(rules, rotation, now);
        Self { id, spec, state: InstanceState::Lobby, capacity, members: BTreeMap::new(), cycle: Some(cycle) }
    }

    pub fn id(&self) -> u16 {
        self.id
    }

    pub fn spec(&self) -> &InstanceSpec {
        &self.spec
    }

    pub fn state(&self) -> InstanceState {
        self.state
    }

    pub fn round(&self) -> u16 {
        self.cycle.as_ref().map_or(0, |c| c.round)
    }

    pub fn start_tick(&self) -> Option<u32> {
        self.cycle.as_ref().and_then(|c| c.start)
    }

    pub fn admit(&mut self, rider: u16) -> Result<(), Admission> {
        if self.members.contains_key(&rider) {
            return Err(Admission::Present);
        }
        if self.members.len() >= self.capacity {
            return Err(Admission::Full);
        }
        let role = match self.state {
            InstanceState::Countdown | InstanceState::Running | InstanceState::Results if self.cycle.is_some() => {
                Role::Spectator
            }
            _ => Role::Rider,
        };
        self.members.insert(rider, Member { progress: Progress::Joined, role, run: Run::default() });
        Ok(())
    }

    pub fn leave(&mut self, rider: u16) -> bool {
        let left = self.members.remove(&rider);
        if let (Some(m), Some(c)) = (&left, self.cycle.as_mut()) {
            c.left(rider, m);
        }
        left.is_some()
    }

    pub fn progress(&self, rider: u16) -> Option<Progress> {
        self.members.get(&rider).map(|m| m.progress)
    }

    pub fn member(&self, rider: u16) -> Option<&Member> {
        self.members.get(&rider)
    }

    pub fn members(&self) -> impl Iterator<Item = (u16, Progress)> + '_ {
        self.members.iter().map(|(&r, m)| (r, m.progress))
    }

    pub fn len(&self) -> usize {
        self.members.len()
    }

    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    pub fn status(&self, rider: u16) -> Option<PlayerStatus> {
        let m = self.members.get(&rider)?;
        Some(match m.progress {
            Progress::Joined => PlayerStatus::Joining,
            Progress::Away => PlayerStatus::Away,
            _ if m.role == Role::Spectator => PlayerStatus::Spectating,
            _ if m.run.over() => PlayerStatus::Finished,
            p => p.status(),
        })
    }

    pub fn relay_roles(&self, rider: u16) -> (bool, bool) {
        let Some(m) = self.members.get(&rider) else { return (false, false) };
        let (sends, takes) = match m.progress {
            Progress::Joined | Progress::Away => (false, false),
            Progress::Dressed(_) => (true, false),
            Progress::Loaded(_) | Progress::Ready(_) => (true, true),
        };
        (sends && m.role == Role::Rider, takes)
    }

    pub fn control(&mut self, rider: u16, message: &ClientControl, now: u32) -> Result<Accepted, Refused> {
        let name = message.name();
        let Some(m) = self.members.get(&rider) else {
            return Err(Refused::new(Refusal::Order, format!("{name} while not in the instance")));
        };
        let progress = m.progress;
        let order = |p: Progress| Refused::new(Refusal::Order, format!("{name} while {}", p.name()));
        let stale = |round: u16| self.cycle.as_ref().is_some_and(|c| c.older(round));
        let mut started = false;
        let next = match (progress, message) {
            (Progress::Joined | Progress::Away, ClientControl::Appearance(a)) => Progress::Dressed(*a),
            (Progress::Dressed(_) | Progress::Loaded(_) | Progress::Ready(_), ClientControl::LevelLeft) => {
                Progress::Away
            }
            (_, ClientControl::Loaded { round } | ClientControl::Ready { round, .. }) if stale(*round) => {
                return Ok(Accepted { stale: true, ..Accepted::default() });
            }
            (Progress::Dressed(a), ClientControl::Loaded { round }) => {
                self.this_round(*round, name)?;
                Progress::Loaded(a)
            }
            (Progress::Loaded(a), ClientControl::Ready { round, .. }) => {
                self.this_round(*round, name)?;
                if let Some(c) = self.cycle.as_mut() {
                    c.start_run(rider, m.role, now)?;
                    started = true;
                }
                Progress::Ready(a)
            }
            (p, ClientControl::StartAck { round }) => {
                let Some(c) = self.cycle.as_mut() else {
                    return Err(Refused::new(Refusal::NoRounds, name));
                };
                return c.start_ack(rider, self.members.get_mut(&rider), p, *round, now);
            }
            (p, ClientControl::Report(report)) => {
                let Some(c) = self.cycle.as_mut() else {
                    return Err(Refused::new(Refusal::NoRounds, name));
                };
                if report.instance != self.id {
                    return Err(Refused::new(
                        Refusal::ReportInstance,
                        format!("instance {} (this rider's: {})", report.instance, self.id),
                    ));
                }
                let at = crate::round::At { mode: self.spec.mode, state: self.state };
                return c.report(rider, self.members.get_mut(&rider), p, report, at);
            }
            (p, _) => return Err(order(p)),
        };
        let left = next == Progress::Away;
        if let Some(m) = self.members.get_mut(&rider) {
            m.progress = next;
            if started {
                m.run.started = Some(now);
            }
        }
        let mut standings = false;
        if left && let Some(c) = self.cycle.as_mut() {
            standings = c.level_left(rider, self.members.get_mut(&rider), self.state);
        }
        Ok(Accepted { progress: true, standings, ..Accepted::default() })
    }

    fn this_round(&self, round: u16, name: &str) -> Result<(), Refused> {
        let current = self.round();
        if round == current {
            return Ok(());
        }
        let rule = if self.cycle.is_some() { Refusal::FutureRound } else { Refusal::NoRounds };
        Err(Refused::new(rule, format!("{name} of round {round} (this round: {current})")))
    }
}

#[cfg(test)]
mod tests;
