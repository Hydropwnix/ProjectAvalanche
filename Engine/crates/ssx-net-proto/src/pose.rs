use ssx_formats::ghost::{POSE_BLOCK_SIZE, PoseBlock};

use crate::DecodeError;

pub const NET_POSE_SIZE: usize = 16 + POSE_BLOCK_SIZE;

pub const VIEW_LEAD_PER_TICK: f64 = 256.0;

pub const MAX_EXECUTION: u32 = (1 << 24) - 1;
pub const RELAYED_POSE_SIZE: usize = 2 + NET_POSE_SIZE;

const UNWRITTEN: [std::ops::Range<usize>; 2] = [0x24..0x30, 0x47..0x48];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoseFlag {
    Teleport,
    Rewind,
    Respawn,
    ImpactLatched,
}

impl PoseFlag {
    const fn bit(self) -> u32 {
        match self {
            PoseFlag::Teleport => 1 << 0,
            PoseFlag::Rewind => 1 << 1,
            PoseFlag::Respawn => 1 << 2,
            PoseFlag::ImpactLatched => 1 << 8,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PoseFlags(u32);

impl PoseFlags {
    const FRACTION_SHIFT: u32 = 16;
    const EPOCH_SHIFT: u32 = 24;
    pub const DEFINED: u32 = 0b111 | (1 << 8) | (0xFF << Self::FRACTION_SHIFT) | (0xFF << Self::EPOCH_SHIFT);

    pub fn from_bits(bits: u32) -> Result<Self, DecodeError> {
        if bits & !Self::DEFINED != 0 {
            return Err(DecodeError::ReservedFlags(bits & !Self::DEFINED));
        }
        Ok(Self(bits))
    }

    pub const fn bits(self) -> u32 {
        self.0
    }

    #[must_use]
    pub const fn with(self, flag: PoseFlag) -> Self {
        Self(self.0 | flag.bit())
    }

    pub const fn contains(self, flag: PoseFlag) -> bool {
        self.0 & flag.bit() != 0
    }

    pub const fn warp_epoch(self) -> u8 {
        (self.0 >> Self::EPOCH_SHIFT) as u8
    }

    #[must_use]
    pub const fn with_warp_epoch(self, epoch: u8) -> Self {
        Self((self.0 & !(0xFF << Self::EPOCH_SHIFT)) | ((epoch as u32) << Self::EPOCH_SHIFT))
    }

    pub const fn tick_fraction(self) -> u8 {
        (self.0 >> Self::FRACTION_SHIFT) as u8
    }

    #[must_use]
    pub const fn with_tick_fraction(self, fraction: u8) -> Self {
        Self((self.0 & !(0xFF << Self::FRACTION_SHIFT)) | ((fraction as u32) << Self::FRACTION_SHIFT))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NetPose {
    tick: u32,
    flags: PoseFlags,
    execution: u32,
    view_lead: i32,
    block: PoseBlock,
}

impl NetPose {
    pub fn new(tick: u32, flags: PoseFlags, block: PoseBlock) -> Result<Self, DecodeError> {
        check_block(&block)?;
        Ok(Self { tick, flags, execution: 0, view_lead: 0, block })
    }

    pub fn with_execution(mut self, execution: u32) -> Result<Self, DecodeError> {
        if execution > MAX_EXECUTION {
            return Err(DecodeError::Execution(execution));
        }
        self.execution = execution;
        Ok(self)
    }

    pub fn execution(&self) -> u32 {
        self.execution
    }

    pub fn at(time: f64, flags: PoseFlags, block: PoseBlock) -> Result<Self, DecodeError> {
        if !time.is_finite() || !(0.0..f64::from(u32::MAX)).contains(&time) {
            return Err(DecodeError::Time(time));
        }
        let tick = time.floor();
        let fraction = (((time - tick) * 256.0).floor() as u32).min(255) as u8;
        Self::new(tick as u32, flags.with_tick_fraction(fraction), block)
    }

    pub fn with_view_time(mut self, view: f64) -> Result<Self, DecodeError> {
        let lead = ((view - self.time()) * VIEW_LEAD_PER_TICK).round();
        if !lead.is_finite() || !(f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&lead) {
            return Err(DecodeError::ViewTime(view));
        }
        self.view_lead = lead as i32;
        Ok(self)
    }

    pub fn view_time(&self) -> f64 {
        self.time() + self.view_lead()
    }

    pub fn view_lead(&self) -> f64 {
        f64::from(self.view_lead) / VIEW_LEAD_PER_TICK
    }

    pub fn tick(&self) -> u32 {
        self.tick
    }

    pub fn time(&self) -> f64 {
        f64::from(self.tick) + f64::from(self.flags.tick_fraction()) / 256.0
    }

    pub fn flags(&self) -> PoseFlags {
        self.flags
    }

    pub fn block(&self) -> &PoseBlock {
        &self.block
    }

    pub fn encode(&self) -> [u8; NET_POSE_SIZE] {
        let mut out = [0u8; NET_POSE_SIZE];
        out[..4].copy_from_slice(&self.tick.to_be_bytes());
        out[4..8].copy_from_slice(&self.flags.bits().to_be_bytes());
        out[8..12].copy_from_slice(&self.execution.to_be_bytes());
        out[12..16].copy_from_slice(&self.view_lead.to_be_bytes());
        out[16..].copy_from_slice(&self.block.to_bytes());
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let bytes: &[u8; NET_POSE_SIZE] = bytes.try_into().map_err(|_| DecodeError::Size {
            kind: "pose sample",
            expected: NET_POSE_SIZE,
            actual: bytes.len(),
        })?;
        let (head, block) = bytes.split_at(16);
        let tick = u32::from_be_bytes([head[0], head[1], head[2], head[3]]);
        let flags = PoseFlags::from_bits(u32::from_be_bytes([head[4], head[5], head[6], head[7]]))?;
        let execution = u32::from_be_bytes([head[8], head[9], head[10], head[11]]);
        let view_lead = i32::from_be_bytes([head[12], head[13], head[14], head[15]]);
        if UNWRITTEN.iter().any(|r| block[r.clone()].iter().any(|&b| b != 0)) {
            return Err(DecodeError::UnwrittenBytes);
        }
        let block = PoseBlock::parse(block).map_err(|e| DecodeError::Block(e.to_string()))?;
        let pose = Self::new(tick, flags, block)?.with_execution(execution)?;
        Ok(Self { view_lead, ..pose })
    }
}

fn check_block(b: &PoseBlock) -> Result<(), DecodeError> {
    if !b.trajectory_rotation.iter().all(|v| v.is_finite()) {
        return Err(DecodeError::NonFinite("the trajectory rotation"));
    }
    if !b.trajectory_position.iter().all(|v| v.is_finite()) {
        return Err(DecodeError::NonFinite("the trajectory position"));
    }
    if b.pose_weight.to_bits() != 1.0f32.to_bits() {
        return Err(DecodeError::PoseWeight(b.pose_weight));
    }
    if b.unk_24 != [0; 12] || b.unk_47 != 0 {
        return Err(DecodeError::UnwrittenBytes);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RiderId(pub u16);

#[derive(Clone, Debug, PartialEq)]
pub struct RelayedPose {
    pub rider: RiderId,
    pub pose: NetPose,
}

impl RelayedPose {
    pub fn encode(&self) -> [u8; RELAYED_POSE_SIZE] {
        let mut out = [0u8; RELAYED_POSE_SIZE];
        out[..2].copy_from_slice(&self.rider.0.to_be_bytes());
        out[2..].copy_from_slice(&self.pose.encode());
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        if bytes.len() != RELAYED_POSE_SIZE {
            return Err(DecodeError::Size { kind: "relayed pose", expected: RELAYED_POSE_SIZE, actual: bytes.len() });
        }
        let rider = RiderId(u16::from_be_bytes([bytes[0], bytes[1]]));
        Ok(Self { rider, pose: NetPose::decode(&bytes[2..])? })
    }
}

#[cfg(test)]
mod tests;
