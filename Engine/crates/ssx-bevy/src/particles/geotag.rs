use std::path::Path;

use bevy::prelude::*;
use ssx_assets::particles::ParticleNode;
use ssx_physics::session::geotags::GeotagRun;

use crate::particles::sim::{EffectInstance, RecordView, SetArena};
use crate::particles::track::{DefCache, textures_of};
use crate::render::geotags::GeotagBurst;
use crate::rider::anim::JointRecord;

pub struct BurstSlot {
    pub entity: usize,
    node: ParticleNode,
    pub instance: EffectInstance,
    start: f32,
    at: (Vec3, Quat),
}

#[derive(Default)]
pub struct GeotagSlots {
    pub slots: Vec<BurstSlot>,
}

impl GeotagSlots {
    pub fn start(
        &mut self,
        bursts: Vec<(f32, GeotagBurst)>,
        root: &Path,
        defs: &mut DefCache,
        names: &mut Vec<String>,
        arena: &mut SetArena,
        clock: f32,
    ) -> Vec<String> {
        let mut log = Vec::new();
        for (_, b) in bursts {
            let effect = b.node.lynx.as_ref().map_or("-", |l| l.name.as_str()).to_owned();
            if let Some(s) = self.slots.iter_mut().find(|s| s.entity == b.entity) {
                (s.start, s.at) = (clock, (b.position, b.rotation));
                log.push(format!("geotag entity {} burst {effect} restarted at clock {clock:.3}", b.entity));
                continue;
            }
            match defs.node(root, &b.node) {
                Ok((def, _)) => {
                    textures_of(&def, names);
                    self.slots.push(BurstSlot {
                        entity: b.entity,
                        node: b.node.clone(),
                        instance: EffectInstance::new(def, arena),
                        start: clock,
                        at: (b.position, b.rotation),
                    });
                    log.push(format!(
                        "geotag entity {} burst {effect} ({}) at {:.2} on at clock {clock:.3}",
                        b.entity, b.node.group_name, b.position
                    ));
                }
                Err(e) => log.push(format!("geotag entity {} burst {effect} unavailable: {e:#}", b.entity)),
            }
        }
        log
    }

    pub fn records(&mut self, run: Option<&GeotagRun>, blend: &[(u8, f32)]) -> Vec<RecordView> {
        if let Some(run) = run {
            self.slots.retain(|s| run.entities.get(s.entity).is_some_and(|e| e.collected));
        }
        self.slots
            .iter_mut()
            .map(|s| {
                // TODO(guess): the locator follows the entity every update (0x8295AD98 vs the fixed `WorldLocation` 0x8295B1A0 by the component's +0x20 flag, not traced for a geotag); settle: the `GameObject` component's sub-component creation in 0x82D3FFC0 (the caller of 0x82953E28 / 0x8298FA18).
                if let Some(e) = run.and_then(|r| r.entities.get(s.entity)) {
                    s.at = (e.position, e.rotation);
                }
                let locator = JointRecord { s: Vec3::ONE, q: s.at.1, p: s.at.0 };
                let (scale, rotation, position) = crate::particles::rider::compose(&locator, &s.node);
                RecordView {
                    scale,
                    rotation,
                    position,
                    start: s.start,
                    stop: -1.0,
                    instant_off: false,
                    ao: 1.0,
                    blend: blend.to_vec(),
                }
            })
            .collect()
    }
}
