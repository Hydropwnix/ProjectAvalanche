use anyhow::{Result, bail};
use glam::Vec3;

use crate::tuning::Bundle;

pub const JANKED_SEQUENCE: &str = "SEQ_janked_standing_death.Player0";
pub const JANKED_ANIM_COMPLETE: &str = "BLN_JankedAnimComplete";

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct JankedTuning {
    pub in_soft_collision_ticks: f32,
    pub in_collision_ticks: f32,
    pub out_of_collision_ticks: f32,
    pub test_pos_update_factor: f32,
    pub distance_threshold: f32,
    pub hard_ray_radius: f32,
    pub soft_ray_radius: f32,
    pub anim_complete_tick: f32,
}

impl JankedTuning {
    pub fn load(bundle: &Bundle) -> Result<Self> {
        let q = bundle.view("SnowboarderPhysicsQueriesSceneOp", "SnowboarderPhysicsQueriesSceneOpAsset")?;
        let seq = bundle.view(JANKED_SEQUENCE, "ActorControllerAsset")?;
        let mut anim_complete_tick = None;
        for collection in seq.asset("TagCollectionSet")?.asset_list("TagCollectionKeys")? {
            let Ok(tags) = collection.asset_list("TagKeys") else { continue };
            for tag in tags.iter().filter(|t| t.class() == "BoolGameStateTag") {
                if tag.asset("ValueAsset")?.name() == JANKED_ANIM_COMPLETE {
                    anim_complete_tick = Some(tag.int("Time")? as f32);
                }
            }
        }
        let Some(anim_complete_tick) = anim_complete_tick else {
            bail!("{JANKED_SEQUENCE} has no {JANKED_ANIM_COMPLETE} tag");
        };
        Ok(Self {
            in_soft_collision_ticks: q.f32("JankedInCollisionTimerThreshold")?,
            in_collision_ticks: q.f32("JankedInCollisionTimerThresholdHard")?,
            out_of_collision_ticks: q.f32("UnjankedOutOfCollisionTimerThreshold")?,
            test_pos_update_factor: q.f32("JankedTestPosUpdateFactor")?,
            distance_threshold: q.f32("JankedDistanceThreshold")?,
            hard_ray_radius: q.f32("HardCorrectionRayRadius")?,
            soft_ray_radius: q.f32("SoftCorrectionRayRadius")?,
            anim_complete_tick,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct JankedState {
    pub ticks_in_collision: f32,
    pub ticks_not_in_collision: f32,
    pub ticks_in_soft: f32,
    pub ticks_not_in_soft: f32,
    pub test_pos: Vec3,
    pub colliding: bool,
    pub soft: bool,
    pub janked: bool,
    pub anim_ticks: f32,
    pub anim_complete: bool,
}

impl JankedState {
    pub fn new(traj: Vec3) -> Self {
        Self { test_pos: traj, ..Self::default() }
    }

    pub fn update(&mut self, t: &JankedTuning, colliding: bool, soft: bool, traj: Vec3, dt: f32) {
        (self.colliding, self.soft) = (colliding, soft);
        let far = self.test_pos.distance(traj) > t.distance_threshold;
        if self.ticks_not_in_soft > t.out_of_collision_ticks || far {
            self.test_pos = traj;
        } else {
            let k = (-t.test_pos_update_factor * dt).exp();
            self.test_pos = traj + (self.test_pos - traj) * k;
        }
        if colliding {
            self.ticks_in_collision += dt;
            self.ticks_not_in_collision = 0.0;
        } else {
            self.ticks_not_in_collision += dt;
        }
        if self.ticks_not_in_collision > t.out_of_collision_ticks {
            self.ticks_in_collision = 0.0;
        }
        if soft {
            self.ticks_in_soft += dt;
            self.ticks_not_in_soft = 0.0;
        } else {
            self.ticks_not_in_soft += dt;
        }
        if self.ticks_not_in_soft > t.out_of_collision_ticks {
            self.ticks_in_soft = 0.0;
        }
        if !self.janked
            && self.ticks_in_soft > t.in_soft_collision_ticks
            && self.ticks_in_collision > t.in_collision_ticks
            && !far
        {
            self.janked = true;
        }
    }

    pub fn advance_sequence(&mut self, t: &JankedTuning, dt: f32) {
        self.anim_ticks += dt;
        self.anim_complete |= self.anim_ticks >= t.anim_complete_tick;
    }
}
