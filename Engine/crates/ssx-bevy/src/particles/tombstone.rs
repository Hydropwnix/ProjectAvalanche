use std::path::Path;

use bevy::prelude::*;
use ssx_assets::particles::ParticleNode;
use ssx_physics::session::tombstones::{TOMBSTONE_ENTITIES, Tombstones};

use crate::particles::sim::{EffectInstance, RecordView, SetArena};
use crate::particles::track::{DefCache, textures_of};
use crate::rider::anim::JointRecord;

#[derive(Default)]
pub struct TombstoneSlots {
    node: Option<ParticleNode>,
    pub slots: Vec<EffectInstance>,
}

impl TombstoneSlots {
    pub fn load(
        root: &Path,
        track: &str,
        defs: &mut DefCache,
        names: &mut Vec<String>,
        arena: &mut SetArena,
    ) -> anyhow::Result<(Self, String)> {
        let a = ssx_assets::track_objects::tombstones::load(root, track)?;
        let (def, _) = defs.node(root, &a.node)?;
        textures_of(&def, names);
        let slots = (0..TOMBSTONE_ENTITIES).map(|_| EffectInstance::new(def.clone(), arena)).collect();
        let effect = a.node.lynx.as_ref().map_or("-", |l| l.name.as_str()).to_owned();
        let line = format!(
            "tombstones: {TOMBSTONE_ENTITIES} entities of {} ({effect}; state {} {:?} / {:?}, mesh {})",
            a.game_object, a.state, a.on_start, a.on_end, a.has_mesh
        );
        Ok((TombstoneSlots { node: Some(a.node), slots }, line))
    }

    pub fn records(&self, tombstones: Option<&Tombstones>, blend: &[(u8, f32)]) -> Vec<RecordView> {
        let (Some(node), Some(t)) = (self.node.as_ref(), tombstones) else { return Vec::new() };
        t.entities
            .iter()
            .take(self.slots.len())
            .map(|e| {
                let locator = JointRecord { s: Vec3::ONE, q: Quat::IDENTITY, p: Vec3::from_array(e.position) };
                let (scale, rotation, position) = crate::particles::rider::compose(&locator, node);
                RecordView {
                    scale,
                    rotation,
                    position,
                    start: 0.0,
                    stop: -1.0,
                    instant_off: false,
                    ao: 1.0,
                    blend: blend.to_vec(),
                }
            })
            .collect()
    }
}
