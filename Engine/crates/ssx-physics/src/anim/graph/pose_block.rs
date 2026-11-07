use super::*;
use ssx_formats::ghost::{PoseBlock, PoseSource};
use ssx_formats::vmx;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockPose {
    pub trajectory_rotation: [f32; 4],
    pub rotations: [[f32; 4]; 29],
    pub translations: [[f32; 3]; 12],
}

impl RiderGraph {
    pub fn block_pose(&self, pose: &RigPose) -> Result<BlockPose> {
        let node = |i: Option<usize>, name: &str| {
            i.and_then(|i| pose.nodes.get(i)).with_context(|| format!("the rider's pose has no {name}"))
        };
        let trajectory_rotation = node(self.trajectory, "AITrajectory")?.rotation;
        let mut rotations = [[0.0; 4]; 29];
        for ((r, &i), name) in rotations.iter_mut().zip(&self.block_joints.0).zip(REWIND_ROTATIONS) {
            *r = node(i, name)?.rotation;
        }
        let mut translations = [[0.0; 3]; 12];
        for ((t, &i), name) in translations.iter_mut().zip(&self.block_joints.1).zip(REWIND_TRANSLATIONS) {
            *t = node(i, name)?.translation;
        }
        Ok(BlockPose { trajectory_rotation, rotations, translations })
    }

    pub fn in_air_tag(&self) -> bool {
        self.producers.in_air_tag
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LandingRumble {
    prev_in_air: bool,
    velocities: [Vec3; 2],
}

impl LandingRumble {
    pub fn step(&mut self, s: &RiderState) -> f32 {
        let in_air = s.in_air();
        let prev = self.velocities[1].to_array();
        let moving = prev.iter().any(|c| vmx::gt(c.abs(), f32::EPSILON));
        let magnitude = if !in_air && self.prev_in_air && s.contact.is_some() && moving {
            let d = vmx::dot3(prev, s.ground_normal.to_array());
            vmx::max(f32::from_bits(d.to_bits() ^ 0x8000_0000), 0.0)
        } else {
            0.0
        };
        self.prev_in_air = in_air;
        self.velocities = [s.traj_velocity, self.velocities[0]];
        magnitude
    }
}

pub fn pose_source(s: &RiderState, g: &RiderGraph, posed: &RigPose, landing_impact: f32) -> Result<PoseSource> {
    let b = g.block_pose(posed)?;
    Ok(PoseSource {
        trajectory_rotation: b.trajectory_rotation,
        trajectory_position: s.traj.to_array(),
        rotations: b.rotations,
        translations: b.translations,
        flags: [
            s.contact.is_some(),
            s.score.is_awesome,
            s.score.is_super_awesome,
            g.in_air_tag(),
            s.goofy,
            s.board_particle.enabled,
            s.wingsuit.signal,
            s.impact.impact_this_frame,
        ],
        snow_coverage: s.weights[crate::surface::SNOW_COVERAGE],
        depth_below_snow: s.depth_below_snow,
        ground_contact: s.contact.map_or(Vec3::ZERO, |c| c.contact).to_array(),
        ground_normal: s.ground_normal.to_array(),
        velocity: s.traj_velocity.to_array(),
        grinding: s.grind.is_grinding(),
        boosting: s.boost.is_boosting,
        snow_spray_velocity: s.spray.velocity.to_array(),
        snow_spray_carving: s.spray.carving.to_array(),
        landing_impact,
        // TODO(guess): the entity ids are the port's numbering (1 + the placement's load order, `CollisionWorld::add_entity`, as `INT_GroundContactEntityId` uses it); the console's come from its instance spawner (0x82EA5160 → `PhysicsMgr_AddPartWithEntityId` 0x82AB2608, u16 at the part object's +0x5A) and are not compared; settle: the spawner's id assignment.
        collision_entity: s.collision_entity as i32,
        impact_velocity: s.impact.impact_velocity.to_array(),
        ticks_to_landing: s.ballistic.ticks,
    })
}

pub struct PoseRecorder {
    rumble: LandingRumble,
    rig_ops: super::RiderRigOps,
}

impl PoseRecorder {
    pub fn new(rig_ops: super::RiderRigOps) -> Self {
        Self { rumble: LandingRumble::default(), rig_ops }
    }

    pub fn reset(&mut self) {
        self.rig_ops.reset();
        self.rumble = LandingRumble::default();
    }

    pub fn record(&mut self, s: &RiderState, g: &RiderGraph) -> Result<(RigPose, PoseBlock)> {
        let landing = self.rumble.step(s);
        let posed = self.rig_ops.apply(s, g);
        let block = PoseBlock::record(&pose_source(s, g, &posed, landing)?);
        Ok((posed, block))
    }
}
