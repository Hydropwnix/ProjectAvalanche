use glam::{Quat, Vec3};

pub use ssx_rules::triggers::*;

#[derive(Clone, Debug, Default)]
pub struct TriggerSet {
    pub volumes: Vec<TriggerVolume>,
    pub entities: Vec<TriggerEntity>,
    pub scripts: std::collections::BTreeMap<u32, (String, std::sync::Arc<str>)>,
    pub bodies: std::collections::BTreeMap<u32, crate::rigid::RigidBodyDef>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TriggerEntity {
    pub object: u32,
    pub instance: u32,
    pub placement: usize,
    pub scope: TriggerScope,
    pub position: Vec3,
    pub rotation: Quat,
}

impl TriggerSet {
    pub fn touching(&self, centre: Vec3, radius: f32, mover_groups: u16) -> impl Iterator<Item = usize> + '_ {
        self.volumes
            .iter()
            .enumerate()
            .filter(move |(_, v)| v.interacts & mover_groups != 0 && v.overlaps_sphere(centre, radius))
            .map(|(i, _)| i)
    }

    pub fn player_groups(&self, p: Vec3) -> u16 {
        self.touching(p, PLAYER_TOUCH_RADIUS, IG_PLAYER).fold(0, |m, i| m | self.volumes[i].groups)
    }

    pub fn group_counts(&self) -> [usize; 7] {
        let mut c = [0; 7];
        for v in &self.volumes {
            for (b, n) in c.iter_mut().enumerate() {
                *n += usize::from(v.groups & (1 << b) != 0);
            }
        }
        c
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TriggerTracker {
    pub update_count: u32,
    pub current: Vec<(usize, u32)>,
    pub entered: Vec<usize>,
    pub exited: Vec<usize>,
    pub closed: Vec<usize>,
}

impl TriggerTracker {
    pub fn update(&mut self, set: &TriggerSet, p: Vec3) {
        self.update_count += 1;
        let now = self.update_count;
        self.entered.clear();
        for i in set.touching(p, PLAYER_TOUCH_RADIUS, IG_PLAYER) {
            if self.closed.contains(&i) {
                continue;
            }
            match self.current.iter_mut().find(|(v, _)| *v == i) {
                Some(pair) => pair.1 = now,
                None => {
                    self.current.push((i, now));
                    self.entered.push(i);
                }
            }
        }
        self.exited.clear();
        let mut k = 0;
        while k < self.current.len() {
            if self.current[k].1 < now {
                self.exited.push(self.current.swap_remove(k).0);
            } else {
                k += 1;
            }
        }
    }

    pub fn touching_group(&self, set: &TriggerSet, mask: u16) -> bool {
        self.current.iter().any(|&(i, _)| set.volumes[i].groups & mask != 0)
    }

    pub fn touching_name(&self, set: &TriggerSet, name: &str) -> bool {
        self.current.iter().any(|&(i, _)| set.volumes[i].name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssx_assets::track_objects::Placement;

    fn record(type_text: &str, shape: i32, scale: Vec3) -> TriggerVolumeRecord {
        TriggerVolumeRecord {
            name: type_text.into(),
            type_text: type_text.into(),
            shape,
            scale,
            position: Vec3::ZERO,
            orientation: Quat::IDENTITY,
        }
    }

    fn placement(scale: Vec3, q: Quat, at: Vec3) -> Placement {
        Placement { scale: scale.to_array(), rotation: q.to_array(), position: at.to_array(), unk: [0.0; 6] }
    }

    #[test]
    fn the_tracker_keeps_current_pairs_and_reports_entries_and_exits() {
        let set = TriggerSet {
            bodies: Default::default(),
            entities: Vec::new(),
            scripts: Default::default(),
            volumes: vec![
                TriggerVolume::place(
                    &record("death", 1, Vec3::splat(4.0)),
                    &placement(Vec3::ONE, Quat::IDENTITY, Vec3::ZERO),
                ),
                TriggerVolume::place(
                    &record("locator", 1, Vec3::splat(100.0)),
                    &placement(Vec3::ONE, Quat::IDENTITY, Vec3::ZERO),
                ),
            ],
        };
        let mut t = TriggerTracker::default();
        t.update(&set, Vec3::new(3.0, 0.0, 0.0));
        assert!(t.current.is_empty() && !t.touching_group(&set, IG_DEATH), "the locator has no groups");
        t.update(&set, Vec3::new(2.2, 0.0, 0.0));
        assert_eq!(t.entered, vec![0]);
        assert!(t.touching_group(&set, IG_DEATH) && t.touching_name(&set, "death"));
        t.update(&set, Vec3::new(1.0, 0.0, 0.0));
        assert!(t.entered.is_empty() && t.touching_group(&set, IG_DEATH), "inside, not an entry");
        t.update(&set, Vec3::new(9.0, 0.0, 0.0));
        assert_eq!(t.exited, vec![0]);
        assert!(!t.touching_group(&set, IG_DEATH));
        assert_eq!(set.player_groups(Vec3::ZERO), IG_DEATH);
    }
}
