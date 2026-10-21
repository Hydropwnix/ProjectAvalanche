use glam::{Quat, Vec3};
use ssx_assets::track_objects::Placement;

pub const IG_PLAYER: u16 = 1 << 0;
pub const IG_GHOST: u16 = 1 << 1;
pub const IG_TRIGGER: u16 = 1 << 2;
pub const IG_COLLECT: u16 = 1 << 3;
pub const IG_DEATH: u16 = 1 << 4;
pub const IG_LIFE: u16 = 1 << 5;
pub const IG_GAMEFLOW: u16 = 1 << 6;
pub const GROUP_NAMES: [&str; 7] = ["player", "ghost", "trigger", "collect", "death", "life", "gameflow"];
pub const MOVER_GROUPS: u16 = IG_PLAYER | IG_GHOST;
const INTERACTIONS: [(u16, u16); 6] = [(0, 2), (0, 6), (0, 3), (0, 4), (0, 5), (1, 6)];
pub const PLAYER_TOUCH_RADIUS: f32 = 0.35;
pub const SHAPE_BOX: i32 = 1;

pub fn group_bit_from_name(name: &str) -> u16 {
    GROUP_NAMES.iter().position(|&n| n == name).map_or(0, |i| 1 << i)
}

pub fn parse_group_mask(text: &str) -> u16 {
    text.split(' ').filter(|t| !t.is_empty()).fold(0, |m, t| m | group_bit_from_name(t))
}

pub fn interaction_mask(groups: u16) -> u16 {
    INTERACTIONS.iter().filter(|(_, t)| groups & (1 << t) != 0).fold(0, |m, (mv, _)| m | (1 << mv))
}

#[derive(Clone, Debug, PartialEq)]
pub struct TriggerVolumeRecord {
    pub name: String,
    pub type_text: String,
    pub shape: i32,
    pub scale: Vec3,
    pub position: Vec3,
    pub orientation: Quat,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VolumeShape {
    Box { half_extents: Vec3 },
    Sphere { radius: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TriggerScope {
    Track,
    Drop(u32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct TriggerVolume {
    pub name: String,
    pub type_text: String,
    pub groups: u16,
    pub interacts: u16,
    pub shape: VolumeShape,
    pub centre: Vec3,
    pub rotation: Quat,
    pub object: u32,
    pub instance: u32,
    pub placement: usize,
    pub scope: TriggerScope,
}

impl TriggerVolume {
    pub fn place(rec: &TriggerVolumeRecord, pl: &Placement) -> Self {
        let inst_scale = Vec3::from_array(pl.scale);
        let inst_q = Quat::from_xyzw(pl.rotation[0], pl.rotation[1], pl.rotation[2], pl.rotation[3]).normalize();
        let size = rec.scale * inst_scale;
        let shape = if rec.shape == SHAPE_BOX {
            VolumeShape::Box { half_extents: size * 0.5 }
        } else {
            VolumeShape::Sphere { radius: size.x.max(size.y).max(size.z) }
        };
        let groups = parse_group_mask(&rec.type_text);
        TriggerVolume {
            name: rec.name.clone(),
            type_text: rec.type_text.clone(),
            groups,
            interacts: interaction_mask(groups),
            shape,
            centre: Vec3::from_array(pl.position) + inst_q * (rec.position * inst_scale),
            rotation: (inst_q * rec.orientation.normalize()).normalize(),
            object: 0,
            instance: 0,
            placement: 0,
            scope: TriggerScope::Track,
        }
    }

    pub fn overlaps_sphere(&self, centre: Vec3, radius: f32) -> bool {
        match self.shape {
            VolumeShape::Sphere { radius: r } => {
                let rr = r + radius;
                rr * rr >= (centre - self.centre).length_squared()
            }
            VolumeShape::Box { half_extents: h } => {
                let d = self.centre - centre;
                let axes = [self.rotation * Vec3::X, self.rotation * Vec3::Y, self.rotation * Vec3::Z];
                let local = Vec3::new(d.dot(axes[0]), d.dot(axes[1]), d.dot(axes[2]));
                let out = local - local.clamp(-h, h);
                radius * radius >= out.length_squared()
            }
        }
    }
}

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
}

impl TriggerTracker {
    pub fn update(&mut self, set: &TriggerSet, p: Vec3) {
        self.update_count += 1;
        let now = self.update_count;
        self.entered.clear();
        for i in set.touching(p, PLAYER_TOUCH_RADIUS, IG_PLAYER) {
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
    fn group_names_masks_and_interactions() {
        assert_eq!(parse_group_mask("death"), IG_DEATH);
        assert_eq!(parse_group_mask("trigger  collect"), IG_TRIGGER | IG_COLLECT);
        assert_eq!(parse_group_mask("locator"), 0);
        assert_eq!(parse_group_mask("Death"), 0, "exact (case-sensitive) FNV-1 match");
        assert_eq!(interaction_mask(IG_DEATH), IG_PLAYER);
        assert_eq!(interaction_mask(IG_GAMEFLOW), IG_PLAYER | IG_GHOST);
        assert_eq!(interaction_mask(IG_PLAYER | IG_GHOST), 0, "movers do not react to movers");
    }

    #[test]
    fn a_tilted_box_and_the_touch_sphere() {
        let q = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let v = TriggerVolume::place(
            &record("death", 1, Vec3::ONE),
            &placement(Vec3::new(10.0, 2.0, 4.0), q, Vec3::X * 5.0),
        );
        assert_eq!(v.shape, VolumeShape::Box { half_extents: Vec3::new(5.0, 1.0, 2.0) });
        assert!(v.overlaps_sphere(Vec3::new(5.0, 0.0, 4.9), 0.0));
        assert!(!v.overlaps_sphere(Vec3::new(7.5, 0.0, 0.0), 0.35));
        assert!(v.overlaps_sphere(Vec3::new(7.3, 0.0, 0.0), 0.35), "0.3 m outside, within the radius");
        let flat = TriggerVolume::place(
            &record("death", 1, Vec3::splat(4.0)),
            &placement(Vec3::ONE, Quat::IDENTITY, Vec3::ZERO),
        );
        assert!(flat.overlaps_sphere(Vec3::new(2.25, 0.0, 0.0), 0.25), "touching counts (≥)");
        assert!(!flat.overlaps_sphere(Vec3::new(2.25, 0.0, 0.0), 0.24));
        let s = TriggerVolume::place(
            &record("trigger", 0, Vec3::new(1.0, 3.0, 2.0)),
            &placement(Vec3::ONE, Quat::IDENTITY, Vec3::ZERO),
        );
        assert_eq!(s.shape, VolumeShape::Sphere { radius: 3.0 });
        assert!(s.overlaps_sphere(Vec3::Y * 3.35, 0.35) && !s.overlaps_sphere(Vec3::Y * 3.36, 0.35));
    }

    #[test]
    fn the_offset_is_scaled_and_turned_by_the_instance() {
        let mut r = record("trigger", 1, Vec3::ONE);
        r.position = Vec3::new(0.0, 1.5, 2.0);
        let q = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let v = TriggerVolume::place(&r, &placement(Vec3::splat(2.0), q, Vec3::new(10.0, 0.0, 0.0)));
        assert!(v.centre.distance(Vec3::new(14.0, 3.0, 0.0)) < 1e-5, "{:?}", v.centre);
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
