use anyhow::Result;
use glam::Vec3;
use ssx_assets::rig_anim::RigPose;
use ssx_formats::ghost::PoseBlock;

use super::{LandingRumble, RiderGraph, pose_source};
use crate::rider::RiderState;
use crate::session::Execution;
use crate::session::survive::SurviveStep;

pub const TELEPORT_M: f32 = 5.0;

#[derive(Default)]
pub struct LiveBlocks {
    rumble: LandingRumble,
    pub newest: Option<(u64, PoseBlock)>,
    pub impact: bool,
    pub teleport: bool,
    pub respawn: bool,
    pub rewinding: bool,
    pub epoch: u8,
    pub failures: u64,
    prev_root: Option<Vec3>,
    fresh: bool,
}

#[derive(Clone, Debug)]
pub struct BlockSample {
    pub step: u64,
    pub block: PoseBlock,
    pub impact: bool,
    pub teleport: bool,
    pub respawn: bool,
    pub rewinding: bool,
    pub epoch: u8,
}

impl LiveBlocks {
    pub fn record(
        &mut self,
        step: u64,
        s: &RiderState,
        g: &RiderGraph,
        posed: &RigPose,
        e: &Execution,
        rewinding: bool,
    ) -> Result<()> {
        let landing = self.rumble.step(s);
        let block = pose_source(s, g, posed, landing).map(|src| PoseBlock::record(&src));
        self.impact |= s.impact.impact_this_frame;
        let warped = self.prev_root.is_some_and(|p| p.distance(s.traj) > TELEPORT_M) || e.survive != SurviveStep::None;
        if warped {
            self.teleport = true;
            self.epoch = self.epoch.wrapping_add(1);
        }
        if self.rewinding && !rewinding {
            self.epoch = self.epoch.wrapping_add(1);
        }
        self.rewinding = rewinding;
        self.prev_root = Some(s.traj);
        match block {
            Ok(b) => {
                self.newest = Some((step, b));
                self.fresh = true;
                Ok(())
            }
            Err(err) => {
                self.failures += 1;
                Err(err)
            }
        }
    }

    pub fn respawned(&mut self) {
        self.respawn = true;
        self.epoch = self.epoch.wrapping_add(1);
        self.prev_root = None;
        self.rumble = LandingRumble::default();
        self.newest = None;
        self.fresh = false;
    }

    pub fn take(&mut self) -> Option<BlockSample> {
        if !self.fresh {
            return None;
        }
        let (step, block) = self.newest.clone()?;
        self.fresh = false;
        Some(BlockSample {
            step,
            block,
            impact: std::mem::take(&mut self.impact),
            teleport: std::mem::take(&mut self.teleport),
            respawn: std::mem::take(&mut self.respawn),
            rewinding: self.rewinding,
            epoch: self.epoch,
        })
    }
}
