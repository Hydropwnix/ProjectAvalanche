use crate::placement::Placement;
use glam::{Quat, Vec3};

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
}
