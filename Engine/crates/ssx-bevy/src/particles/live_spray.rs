use anyhow::Result;
use ssx_assets::ghosts::GhostPose;
use ssx_assets::particles::SnowSpraySet;

use super::Spray;
use crate::particles::sim::SetArena;
use crate::particles::spray::{SnowSprayDriver, SprayInputs};
use crate::rider::anim::JointRecord;

pub(super) struct LiveSpray {
    pub rider: Option<u16>,
    pub board: Option<JointRecord>,
    pub inputs: SprayInputs,
    pub spray: Spray,
}

#[derive(Default)]
pub(super) struct LiveSprays {
    pub set: Option<SnowSpraySet>,
    pub pool: Vec<LiveSpray>,
}

pub(super) fn off() -> SprayInputs {
    SprayInputs { ao: 1.0, ..SprayInputs::default() }
}

impl LiveSprays {
    pub fn add(&mut self, spray: Spray) {
        self.pool.push(LiveSpray { rider: None, board: None, inputs: off(), spray });
    }

    pub fn assign<'a>(
        &mut self,
        racers: impl Iterator<Item = (u16, JointRecord, &'a GhostPose)>,
        arena: &mut SetArena,
        mut build: impl FnMut(&SnowSpraySet, &mut SetArena) -> Result<Spray>,
        inputs_of: impl Fn(&GhostPose) -> SprayInputs,
    ) -> Result<bool> {
        let racers: Vec<(u16, JointRecord, &GhostPose)> = racers.collect();
        for s in &mut self.pool {
            if !s.rider.is_some_and(|id| racers.iter().any(|r| r.0 == id)) {
                s.rider = None;
            }
            s.inputs = off();
        }
        let mut built = false;
        for (id, board, pose) in racers {
            let k = match self.pool.iter().position(|s| s.rider == Some(id)) {
                Some(k) => k,
                None => match self.pool.iter().position(|s| s.rider.is_none()) {
                    Some(k) => {
                        reset(&mut self.pool[k].spray, arena);
                        k
                    }
                    None => {
                        let Some(set) = self.set.as_ref() else { return Ok(built) };
                        let spray = build(set, arena)?;
                        built = true;
                        self.add(spray);
                        self.pool.len() - 1
                    }
                },
            };
            let s = &mut self.pool[k];
            (s.rider, s.board, s.inputs) = (Some(id), Some(board), inputs_of(pose));
        }
        Ok(built)
    }
}

fn reset(spray: &mut Spray, arena: &mut SetArena) {
    for e in &mut spray.effects {
        e.instance.make_dormant(arena);
        (e.start, e.stop, e.instant) = (-1.0, -1.0, false);
    }
    spray.driver = SnowSprayDriver::new(true);
    spray.inputs = Default::default();
}
