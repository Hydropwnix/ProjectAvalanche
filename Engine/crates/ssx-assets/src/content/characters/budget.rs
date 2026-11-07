use std::collections::{BTreeMap, BTreeSet};

use ssx_content::material::TextureRef;
use ssx_content::{ContentId, Kind};

use super::super::GameContent;
use super::super::level::{LevelPackages, budget};
use super::super::resolve::MaterialUse;
use super::super::track_packages::{Outcome, Usage};
use super::dress::LookChoice;

#[derive(Clone, Debug, Default)]
pub struct RiderLooks {
    pub looks: BTreeMap<ContentId, LookChoice>,
    pub items: BTreeSet<ContentId>,
    pub outcomes: Vec<Outcome>,
    pub start: Usage,
    pub used: Usage,
    counted: BTreeSet<(String, String)>,
}

impl RiderLooks {
    pub fn look(&self, id: &ContentId) -> LookChoice {
        self.looks.get(id).cloned().unwrap_or(LookChoice::Disc)
    }

    pub fn summary(&self) -> String {
        let taken = self.outcomes.iter().filter(|o| o.refused.is_none() && o.id.kind() == Kind::Character).count();
        let refused = self.outcomes.iter().filter(|o| o.refused.is_some()).count();
        format!(
            "riders: {taken} package looks and {} package items taken, {refused} not; {} vertices, {} joints, {} bytes of \
             images",
            self.items.len(),
            self.used.vertices,
            self.used.joints,
            self.used.image_bytes
        )
    }
}

impl GameContent {
    pub fn rider_looks(
        &self,
        level: Option<&LevelPackages>,
        characters: &[ContentId],
        items: &[ContentId],
    ) -> RiderLooks {
        let mut d = RiderLooks { start: level.map(|l| l.used).unwrap_or_default(), ..RiderLooks::default() };
        self.decide(&mut d, characters, items);
        d
    }

    pub fn rider_looks_again(&self, before: &RiderLooks, characters: &[ContentId], items: &[ContentId]) -> RiderLooks {
        let mut d = RiderLooks { start: before.start, ..RiderLooks::default() };
        self.decide(&mut d, characters, items);
        d
    }

    pub fn more_rider_looks(&self, before: &RiderLooks, characters: &[ContentId]) -> (RiderLooks, Vec<LookChoice>) {
        let mut d = before.clone();
        let first = d.outcomes.len();
        let looks = self.decide(&mut d, characters, &[]);
        d.outcomes.drain(..first);
        (d, looks)
    }

    fn decide(&self, d: &mut RiderLooks, characters: &[ContentId], items: &[ContentId]) -> Vec<LookChoice> {
        let mut looks = Vec::new();
        if !self.provides_riders() {
            return vec![LookChoice::Disc; characters.len()];
        }
        let start = d.start;
        let first = d.outcomes.len();
        let ids = characters.iter().map(|c| (c, true)).chain(items.iter().map(|i| (i, false)));
        for (id, is_character) in ids {
            if !is_character && d.items.contains(id) {
                continue;
            }
            let Some(at) = self.look_sources(id).into_iter().next() else {
                if is_character {
                    looks.push(LookChoice::Disc);
                }
                continue;
            };
            let cost = match self.look_cost(id, &at) {
                Ok((file, c)) => {
                    let fresh = d.counted.insert((at.source.clone(), file));
                    let mut u = Usage {
                        vertices: c.vertices,
                        triangles: c.triangles,
                        held_bytes: if fresh { c.held_bytes } else { 0 },
                        image_bytes: if fresh { c.drawn_image_bytes } else { 0 },
                        joints: c.joints,
                        collision_triangles: 0,
                    };
                    u.image_bytes += self.material_texture_bytes(id, &mut d.counted);
                    Ok(u)
                }
                Err(e) => Err(e.to_string()),
            };
            let mut o = Outcome {
                id: id.clone(),
                source: at.source.clone(),
                placements: 0,
                cost: Usage::default(),
                refused: None,
            };
            match cost {
                Ok(add) => {
                    o.cost = add;
                    let total = start.plus(&d.used).plus(&add);
                    if let Some((what, amount, cap)) = total.over(&budget()) {
                        o.refused = Some(format!("the level's {what} would be {amount}; at most {cap}"));
                    } else {
                        d.used = d.used.plus(&add);
                    }
                }
                Err(e) => o.refused = Some(e),
            }
            if let Some(why) = &o.refused {
                let instead = if is_character { "the disc's look is drawn" } else { "its slot as the look has it" };
                self.notice(format!("content: {id} from {} is not used ({instead}): {why}", at.source));
                if is_character {
                    looks.push(LookChoice::Disc);
                    d.looks.entry(id.clone()).or_insert(LookChoice::Disc);
                }
            } else if is_character {
                looks.push(LookChoice::Package(at.clone()));
                d.looks.entry(id.clone()).or_insert(LookChoice::Package(at));
            } else {
                d.items.insert(id.clone());
            }
            d.outcomes.push(o);
        }
        if d.outcomes.len() > first {
            self.info(format!("content: {}", d.summary()));
            let mut latest = self.riders.latest.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            *latest = Some((self.generation(), d.clone()));
            self.riders.decisions.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        }
        looks
    }

    pub fn rider_decisions(&self) -> u64 {
        self.riders.decisions.load(std::sync::atomic::Ordering::Acquire)
    }

    pub fn rider_overlay(&self) -> Vec<String> {
        const MAX_SHOWN: usize = 6;
        let generation = self.generation();
        let latest = self.riders.latest.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some((_, d)) = latest.as_ref().filter(|(g, _)| *g == generation) else { return Vec::new() };
        let mut out = vec![d.summary()];
        let refused: Vec<&Outcome> = d.outcomes.iter().filter(|o| o.refused.is_some()).collect();
        for o in refused.iter().take(MAX_SHOWN) {
            out.push(format!("  {} on the disc's: {}", o.id, o.refused.as_deref().unwrap_or("")));
        }
        if refused.len() > MAX_SHOWN {
            out.push(format!("  … {} more (see the log)", refused.len() - MAX_SHOWN));
        }
        out
    }

    fn material_texture_bytes(&self, id: &ContentId, counted: &mut BTreeSet<(String, String)>) -> u64 {
        let materials: Vec<ContentId> = match id.kind() {
            Kind::Character => {
                self.character_def(id).map(|d| d.value.materials.into_values().collect()).unwrap_or_default()
            }
            _ => self.item_def(id).map(|d| d.value.materials.into_values().collect()).unwrap_or_default(),
        };
        let mut bytes = 0u64;
        for m in materials {
            let Ok(found) = self.material(&m, MaterialUse::Riders) else { continue };
            for t in found.value.textures.values() {
                let TextureRef::Id(t) = t else { continue };
                let Some((source, layer, _)) = self.package_providers(t).first().cloned() else { continue };
                if let Ok((probe, file)) = self.texture_probe_from(t, &source, layer)
                    && counted.insert((source, file))
                {
                    bytes = bytes.saturating_add(probe.decoded_bytes());
                }
            }
        }
        bytes
    }
}
