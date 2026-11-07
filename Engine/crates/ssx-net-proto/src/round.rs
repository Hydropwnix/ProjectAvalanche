use crate::{DecodeError, NET_POSE_SIZE, NetPose};

pub const SPLITS: u8 = 9;

pub const MAX_CLOCK_MS: u32 = 3_599_999;

pub const MAX_POINTS: u32 = i32::MAX as u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReportKind {
    Split = 1,
    Points = 2,
    Finish = 3,
    Failed = 4,
}

impl ReportKind {
    fn from_byte(b: u8) -> Option<Self> {
        Some(match b {
            1 => Self::Split,
            2 => Self::Points,
            3 => Self::Finish,
            4 => Self::Failed,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Split => "split",
            Self::Points => "points",
            Self::Finish => "finish",
            Self::Failed => "failed",
        }
    }
}

pub const REPORT_SIZE: usize = 2 + 2 + 1 + 1 + 4 + 4 + 4 + 2 + 2 + NET_POSE_SIZE;

const NO_SPLIT: u8 = 0xFF;

#[derive(Clone, Debug, PartialEq)]
pub struct ProgressReport {
    pub instance: u16,
    pub round: u16,
    pub kind: ReportKind,
    pub split: Option<u8>,
    pub clock_ms: u32,
    pub points: u32,
    pub descent_mm: u32,
    pub rewinds: u16,
    pub loops: u16,
    pub sample: NetPose,
}

impl ProgressReport {
    pub fn new(r: ProgressReport) -> Result<Self, DecodeError> {
        check(&r)?;
        Ok(r)
    }

    pub fn time(&self) -> f64 {
        self.sample.time()
    }

    pub fn encode(&self) -> [u8; REPORT_SIZE] {
        let mut out = [0u8; REPORT_SIZE];
        out[0..2].copy_from_slice(&self.instance.to_be_bytes());
        out[2..4].copy_from_slice(&self.round.to_be_bytes());
        out[4] = self.kind as u8;
        out[5] = self.split.unwrap_or(NO_SPLIT);
        out[6..10].copy_from_slice(&self.clock_ms.to_be_bytes());
        out[10..14].copy_from_slice(&self.points.to_be_bytes());
        out[14..18].copy_from_slice(&self.descent_mm.to_be_bytes());
        out[18..20].copy_from_slice(&self.rewinds.to_be_bytes());
        out[20..22].copy_from_slice(&self.loops.to_be_bytes());
        out[22..].copy_from_slice(&self.sample.encode());
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let bytes: &[u8; REPORT_SIZE] = bytes.try_into().map_err(|_| DecodeError::Size {
            kind: "progress report",
            expected: REPORT_SIZE,
            actual: bytes.len(),
        })?;
        let u16_at = |k: usize| u16::from_be_bytes([bytes[k], bytes[k + 1]]);
        let u32_at = |k: usize| u32::from_be_bytes([bytes[k], bytes[k + 1], bytes[k + 2], bytes[k + 3]]);
        let kind = ReportKind::from_byte(bytes[4]).ok_or(DecodeError::Field("progress report: unknown kind"))?;
        let split = (bytes[5] != NO_SPLIT).then_some(bytes[5]);
        let r = ProgressReport {
            instance: u16_at(0),
            round: u16_at(2),
            kind,
            split,
            clock_ms: u32_at(6),
            points: u32_at(10),
            descent_mm: u32_at(14),
            rewinds: u16_at(18),
            loops: u16_at(20),
            sample: NetPose::decode(&bytes[22..])?,
        };
        check(&r)?;
        Ok(r)
    }
}

fn check(r: &ProgressReport) -> Result<(), DecodeError> {
    match (r.kind, r.split) {
        (ReportKind::Split, Some(k)) if k < SPLITS => {}
        (ReportKind::Split, _) => return Err(DecodeError::Field("progress report: a split's index is 0 to 8")),
        (_, Some(_)) => return Err(DecodeError::Field("progress report: only a split names a split")),
        (_, None) => {}
    }
    if r.clock_ms > MAX_CLOCK_MS {
        return Err(DecodeError::Field("progress report: the clock is beyond the event clock's 3599.999 s"));
    }
    if r.points > MAX_POINTS {
        return Err(DecodeError::Field("progress report: the points are beyond a result's"));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum StandingState {
    Racing = 1,
    Finished = 2,
    Failed = 3,
    Left = 4,
    Invalidated = 5,
}

impl StandingState {
    fn from_byte(b: u8) -> Option<Self> {
        Some(match b {
            1 => Self::Racing,
            2 => Self::Finished,
            3 => Self::Failed,
            4 => Self::Left,
            5 => Self::Invalidated,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Check {
    Speed = 1,
    Step = 2,
    Splits = 3,
    Finish = 4,
    Clock = 5,
    Tricks = 6,
}

impl Check {
    pub const ALL: [Check; 6] = [Check::Speed, Check::Step, Check::Splits, Check::Finish, Check::Clock, Check::Tricks];

    pub fn code(self) -> u8 {
        self as u8
    }

    pub fn from_code(code: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.code() == code)
    }

    pub fn index(self) -> usize {
        usize::from(self.code() - 1)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Speed => "speed",
            Self::Step => "step",
            Self::Splits => "splits",
            Self::Finish => "finish",
            Self::Clock => "clock",
            Self::Tricks => "tricks",
        }
    }

    pub fn reason(self) -> &'static str {
        match self {
            Self::Speed => "a speed the game's physics does not reach",
            Self::Step => "a move the game's physics does not make",
            Self::Splits => "a split out of order or at the wrong height",
            Self::Finish => "a finish outside the finish line",
            Self::Clock => "a clock that does not match the server's time",
            Self::Tricks => "trick points faster than the trick table allows",
        }
    }
}

pub const STANDING_SIZE: usize = 2 + 1 + 1 + 4 + 4 + 4 + 4 + 1 + 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StandingRow {
    pub rider: u16,
    pub state: StandingState,
    pub split: Option<u8>,
    pub clock_ms: u32,
    pub points: u32,
    pub descent_mm: u32,
    pub tick: u32,
    pub fraction: u8,
    pub invalid: Option<Check>,
}

impl StandingRow {
    pub fn encode(&self) -> [u8; STANDING_SIZE] {
        let mut out = [0u8; STANDING_SIZE];
        out[0..2].copy_from_slice(&self.rider.to_be_bytes());
        out[2] = self.state as u8;
        out[3] = self.split.unwrap_or(NO_SPLIT);
        out[4..8].copy_from_slice(&self.clock_ms.to_be_bytes());
        out[8..12].copy_from_slice(&self.points.to_be_bytes());
        out[12..16].copy_from_slice(&self.descent_mm.to_be_bytes());
        out[16..20].copy_from_slice(&self.tick.to_be_bytes());
        out[20] = self.fraction;
        out[21] = self.invalid.map_or(0, Check::code);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let b: &[u8; STANDING_SIZE] = bytes.try_into().map_err(|_| DecodeError::Size {
            kind: "standings row",
            expected: STANDING_SIZE,
            actual: bytes.len(),
        })?;
        let u32_at = |k: usize| u32::from_be_bytes([b[k], b[k + 1], b[k + 2], b[k + 3]]);
        let state = StandingState::from_byte(b[2]).ok_or(DecodeError::Field("standings: unknown state"))?;
        let split = match b[3] {
            NO_SPLIT => None,
            k if k < SPLITS => Some(k),
            _ => return Err(DecodeError::Field("standings: a split is 0 to 8")),
        };
        let row = StandingRow {
            rider: u16::from_be_bytes([b[0], b[1]]),
            state,
            split,
            clock_ms: u32_at(4),
            points: u32_at(8),
            descent_mm: u32_at(12),
            tick: u32_at(16),
            fraction: b[20],
            invalid: match (state, b[21]) {
                (StandingState::Invalidated, c) => {
                    Some(Check::from_code(c).ok_or(DecodeError::Field("standings: an invalidated row's check"))?)
                }
                (_, 0) => None,
                _ => return Err(DecodeError::Field("standings: a check on a row that is not invalidated")),
            },
        };
        if row.clock_ms > MAX_CLOCK_MS || row.points > MAX_POINTS {
            return Err(DecodeError::Field("standings: a clock or points beyond a report's"));
        }
        Ok(row)
    }

    pub fn time(&self) -> f64 {
        f64::from(self.tick) + f64::from(self.fraction) / 256.0
    }
}

#[cfg(test)]
mod tests;
