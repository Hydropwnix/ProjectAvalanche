use std::collections::VecDeque;
use std::path::Path;

use anyhow::{Context, Result};
use glam::{Mat3, Quat, Vec3};

use crate::collision::{CollisionWorld, SegmentQuery};
use crate::surface::AMBIENT_OCCLUSION;

pub const WASH_RAY_LENGTH: f32 = 60.0;

pub const ID_DOWNWASH_GROUND_AO: u16 = 159;
pub const ID_DOWNWASH_VISUAL_STRENGTH: u16 = 161;

pub fn wash_joints(big_root: &Path) -> Result<(String, String)> {
    use ssx_assets::rig_anim::{ANIM_DIR, RIG_BUNDLE};
    let db = ssx_assets::anim_db::AssetDb::load(&big_root.join(ANIM_DIR), &[RIG_BUNDLE])?;
    let all = &db.streams.first().context("no anim_all.cba")?.1;
    let rig = ssx_assets::nis::Rig::load(all, ssx_assets::nis::HELI_RIG)?;
    let op = db.by_name("HeliPhysicsQueriesSceneOp").context("no HeliPhysicsQueriesSceneOp")?;
    let joint = |field: &str| {
        op.guid(field)
            .and_then(|g| db.get(g))
            .and_then(|id| ssx_assets::nis::template_joint(&db, &rig, &id))
            .with_context(|| format!("HeliPhysicsQueriesSceneOp.{field}: no joint"))
    };
    Ok((joint("WashStartJointAsset")?, joint("WashEndJointAsset")?))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Downwash {
    pub visual_strength: f32,
    pub ground_ao: f32,
    pub wash: Option<(Quat, Vec3)>,
}

pub fn downwash_query(collision: &CollisionWorld, rotor: (Quat, Vec3)) -> Downwash {
    let (q, p) = rotor;
    let end = p + q * Vec3::NEG_Y * WASH_RAY_LENGTH;
    let Some(hit) = collision.segment_query(&SegmentQuery::line(p, end, 0)) else {
        return Downwash { visual_strength: -1.0, ground_ao: -1.0, wash: None };
    };
    let fraction = (hit.t / (end - p).length()).clamp(0.0, 1.0);
    let weights = collision.surface_weights(hit.cluster, hit.tri, hit.bary, hit.normal.y);
    Downwash {
        visual_strength: 1.0 - fraction,
        ground_ao: weights[AMBIENT_OCCLUSION],
        wash: Some((surface_frame(hit.normal), hit.point)),
    }
}

impl super::Helicopter {
    pub fn physics_queries(&self, collision: &CollisionWorld) -> Option<Downwash> {
        let (start, _) = self.flight.as_deref()?.wash.as_ref()?;
        Some(downwash_query(collision, self.joint_world(start)?))
    }
}

pub fn surface_frame(n: Vec3) -> Quat {
    let d = (n - Vec3::Y).abs();
    if d.max_element() <= crate::collision::SEGMENT_MIN_DELTA {
        return Quat::IDENTITY;
    }
    let a = n.cross(Vec3::Y).normalize();
    let b = a.cross(n).normalize();
    Quat::from_mat3(&Mat3::from_cols(a, n, b))
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AoWindow {
    samples: VecDeque<(f32, f32)>,
}

const RING: usize = 128;
const WINDOW: usize = 25;

impl AoWindow {
    pub fn push(&mut self, ground_ao: f32) {
        let sample = if ground_ao < 0.0 { (0.5, 1.0) } else { (ground_ao, 2.0) };
        if self.samples.len() == RING {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }

    pub fn mean(&self) -> f32 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let skip = self.samples.len().saturating_sub(WINDOW);
        let (mut sw, mut swv) = (0.0f32, 0.0f32);
        for &(v, w) in self.samples.iter().skip(skip) {
            sw += w;
            swv = v.mul_add(w, swv);
        }
        let m = swv / sw;
        let m = if -m >= 0.0 { 0.0 } else { m };
        if 1.0 - m >= 0.0 { m } else { 1.0 }
    }
}

pub fn ao_blend(start: f32, mean: f32, elapsed: f32, blend: f32) -> f32 {
    let r = elapsed / blend;
    let t = if -r >= 0.0 { 0.0 } else { r };
    let t = if 1.0 - t >= 0.0 { t } else { 1.0 };
    t.mul_add(mean - start, start)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_name_the_downwash_values() {
        use ssx_lua::game::objectives::game_state_name;
        assert_eq!(game_state_name(ID_DOWNWASH_GROUND_AO), "FLT_DownwashGroundAo");
        assert_eq!(game_state_name(ID_DOWNWASH_VISUAL_STRENGTH), "FLT_DownwashVisualStrength");
    }

    #[test]
    fn surface_frame_aligns_y_with_the_normal() {
        assert_eq!(surface_frame(Vec3::Y), Quat::IDENTITY);
        let n = Vec3::new(0.3, 0.9, -0.2).normalize();
        let q = surface_frame(n);
        assert!((q * Vec3::Y - n).length() < 1e-5);
        assert!((q * Vec3::X - n.cross(Vec3::Y).normalize()).length() < 1e-5);
    }

    #[test]
    fn window_weights_and_length() {
        let mut w = AoWindow::default();
        assert_eq!(w.mean(), 0.0);
        w.push(-1.0);
        w.push(0.8);
        assert!((w.mean() - (0.5 + 1.6) / 3.0).abs() < 1e-6);
        for _ in 0..25 {
            w.push(0.2);
        }
        assert!((w.mean() - 0.2).abs() < 1e-6);
        for _ in 0..200 {
            w.push(-1.0);
        }
        assert_eq!(w.samples.len(), RING);
        assert_eq!(w.mean(), 0.5);
        assert_eq!(ao_blend(1.0, 0.5, 0.25, 0.5), 0.75);
        assert_eq!(ao_blend(1.0, 0.5, 2.0, 0.5), 0.5);
    }
}
