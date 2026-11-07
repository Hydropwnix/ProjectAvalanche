use crate::events::EventType;
use crate::results::{self, GHOST_ROWS, GhostEntry, PADDING_CHARACTER, RunEnd};

pub const MAX_CLOCK_MS: u32 = 3_599_999;
pub const MAX_POINTS: u32 = 0x7FFF_FFFF;
pub const MAX_DESCENT_MM: u32 = u32::MAX - 1;
pub const MAX_TARGETS: usize = GHOST_ROWS;
pub const MAX_COUNT: u32 = 0x7FFF_FFFE;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Measure {
    Time,
    Points,
    Descent,
}

impl Measure {
    pub fn of(t: EventType) -> Self {
        if t.is_race() {
            Measure::Time
        } else if t.is_trick() {
            Measure::Points
        } else {
            Measure::Descent
        }
    }

    pub fn max(self) -> u32 {
        match self {
            Measure::Time => MAX_CLOCK_MS,
            Measure::Points => MAX_POINTS,
            Measure::Descent => MAX_DESCENT_MM,
        }
    }

    pub fn ascending(self) -> bool {
        self == Measure::Time
    }

    pub fn unit(self) -> &'static str {
        match self {
            Measure::Time => "ms",
            Measure::Points => "points",
            Measure::Descent => "mm",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rivals {
    None,
    Board,
    Racers,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TimeUp {
    Fail,
    Result,
}

impl TimeUp {
    pub fn of(t: EventType) -> Self {
        if t == EventType::SurviveToDeath { TimeUp::Result } else { TimeUp::Fail }
    }

    pub fn name(self) -> &'static str {
        match self {
            TimeUp::Fail => "fail",
            TimeUp::Result => "result",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RulesError {
    TargetCount { count: usize },
    NoBoard { event_type: EventType },
    OutOfRange { index: usize, value: u32, max: u32 },
    Order { index: usize },
    TimeLimit { ms: u32 },
    RaceResultAtTimeUp,
}

impl std::fmt::Display for RulesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RulesError::TargetCount { count } => {
                write!(f, "{count} targets: none or {MAX_TARGETS} (the board's rows: gold, silver, bronze)")
            }
            RulesError::NoBoard { event_type } => {
                write!(f, "{} ranks no board: it takes no targets", event_type.name())
            }
            RulesError::OutOfRange { index, value, max } => {
                write!(f, "target {}: {value} is outside 1 … {max}", index + 1)
            }
            RulesError::Order { index } => write!(
                f,
                "target {}: not strictly worse than target {} (best first: a medal below an equal or better one \
                 could never be won)",
                index + 1,
                index
            ),
            RulesError::TimeLimit { ms } => write!(f, "a time limit of {ms} ms: above 0, at most {MAX_CLOCK_MS}"),
            RulesError::RaceResultAtTimeUp => {
                write!(f, "a race has no result at its time limit (its value is the finish clock): `fail` only")
            }
        }
    }
}

impl std::error::Error for RulesError {}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Rules {
    event_type: EventType,
    targets: Vec<u32>,
    time_limit_ms: Option<u32>,
    time_up: TimeUp,
}

impl Rules {
    pub fn new(
        event_type: EventType,
        targets: &[u32],
        time_limit_ms: Option<u32>,
        time_up: Option<TimeUp>,
    ) -> Result<Self, RulesError> {
        check_targets(event_type, targets)?;
        if let Some(ms) = time_limit_ms
            && !(1..=MAX_CLOCK_MS).contains(&ms)
        {
            return Err(RulesError::TimeLimit { ms });
        }
        let time_up = time_up.unwrap_or(TimeUp::of(event_type));
        if time_up == TimeUp::Result && event_type.is_race() {
            return Err(RulesError::RaceResultAtTimeUp);
        }
        Ok(Rules { event_type, targets: targets.to_vec(), time_limit_ms, time_up })
    }

    pub fn plain(event_type: EventType, time_limit_ms: Option<u32>) -> Result<Self, RulesError> {
        Self::new(event_type, &[], time_limit_ms, None)
    }

    pub fn event_type(&self) -> EventType {
        self.event_type
    }

    pub fn measure(&self) -> Measure {
        Measure::of(self.event_type)
    }

    pub fn targets(&self) -> &[u32] {
        &self.targets
    }

    pub fn time_limit_ms(&self) -> Option<u32> {
        self.time_limit_ms
    }

    pub fn time_up(&self) -> TimeUp {
        self.time_up
    }

    pub fn rivals(&self) -> Rivals {
        match self.event_type {
            EventType::TrickBots | EventType::RaceBots => Rivals::Racers,
            t if results::has_ghost_rows(Some(t)) => Rivals::Board,
            _ => Rivals::None,
        }
    }

    pub fn board(&self) -> Vec<GhostEntry> {
        let rows = self.targets.iter().map(|&v| (v, PADDING_CHARACTER));
        results::ghost_board(rows, Some(self.event_type))
    }

    pub fn board_values(&self) -> Vec<u32> {
        self.board().iter().map(|e| e.value).collect()
    }

    pub fn outcome(&self, failed: bool, value: u32) -> (Option<usize>, bool) {
        results::explore_outcome(Some(self.event_type), failed, value, &self.board_values())
    }

    pub fn run_end(&self, failed: bool, value: u32) -> RunEnd {
        let (rank, won) = self.outcome(failed, value);
        RunEnd::explore(failed, value, rank, won)
    }

    pub fn rewinds_infinite(&self) -> Option<bool> {
        match self.event_type {
            EventType::FreeRide => None,
            t => Some(!t.is_survive()),
        }
    }

    pub fn heli_interaction(&self) -> Option<bool> {
        match self.event_type {
            EventType::FreeRide => None,
            t => Some(t.is_trick()),
        }
    }

    pub fn loops(&self) -> bool {
        self.event_type == EventType::SurviveToDeath
    }
}

pub fn check_targets(event_type: EventType, targets: &[u32]) -> Result<(), RulesError> {
    if !targets.is_empty() && targets.len() != MAX_TARGETS {
        return Err(RulesError::TargetCount { count: targets.len() });
    }
    if !targets.is_empty() && !results::has_ghost_rows(Some(event_type)) {
        return Err(RulesError::NoBoard { event_type });
    }
    let m = Measure::of(event_type);
    for (index, &value) in targets.iter().enumerate() {
        if value == 0 || value > m.max() {
            return Err(RulesError::OutOfRange { index, value, max: m.max() });
        }
        if index > 0 && !strictly_better(m, targets[index - 1], value) {
            return Err(RulesError::Order { index });
        }
    }
    Ok(())
}

fn strictly_better(m: Measure, a: u32, b: u32) -> bool {
    if m.ascending() { a < b } else { a > b }
}

pub fn is_better(t: EventType, new: u32, old: Option<u32>) -> bool {
    if new == u32::MAX {
        return false;
    }
    match old {
        None | Some(u32::MAX) => true,
        Some(old) => strictly_better(Measure::of(t), new, old),
    }
}

pub fn keeps(t: EventType, failed: bool, new: u32, old: Option<u32>) -> bool {
    is_better(t, new, old) && (!failed || t.is_survive())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RunOutcome {
    pub value: u32,
    pub failed: bool,
    pub rewinds: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Record {
    pub best: Option<u32>,
    pub attempts: u32,
    pub deaths: u32,
    pub fewest_rewinds: Option<u32>,
}

impl Record {
    pub fn add(&mut self, t: EventType, run: RunOutcome) -> bool {
        let better = keeps(t, run.failed, run.value, self.best);
        if better {
            self.best = Some(run.value);
        }
        if self.fewest_rewinds.is_none_or(|r| run.rewinds < r) {
            self.fewest_rewinds = Some(run.rewinds);
        }
        if run.failed && self.deaths < MAX_COUNT {
            self.deaths += 1;
        }
        if self.attempts < MAX_COUNT {
            self.attempts += 1;
        }
        better
    }
}

#[cfg(test)]
mod tests;
