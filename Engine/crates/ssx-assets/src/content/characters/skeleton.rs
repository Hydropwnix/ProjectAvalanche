use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use ssx_content::skeleton::{RowMatrix, Skeleton, SkeletonJoint};
use ssx_formats::crsf::{CrsfFile, SceneGraph};

use crate::character::{Srt, model_matrices};

pub const RIG_FILE: &str = "rig\\geometry\\skeleton_boarder_common.crsf";

#[derive(Default)]
pub struct SkeletonCache(Mutex<BTreeMap<Option<String>, Arc<RiderSkeleton>>>);

pub struct RiderSkeleton {
    pub skeleton: Skeleton,
    pub base_rig: SceneGraph,
    pub rig: SceneGraph,
    pub bind: Vec<Srt>,
    pub face_offsets: HashMap<String, [f32; 3]>,
}

pub fn inverse(m: &RowMatrix) -> Option<RowMatrix> {
    let a = |r: usize, c: usize| f64::from(m[r][c]);
    let det = a(0, 0) * (a(1, 1) * a(2, 2) - a(1, 2) * a(2, 1)) - a(0, 1) * (a(1, 0) * a(2, 2) - a(1, 2) * a(2, 0))
        + a(0, 2) * (a(1, 0) * a(2, 1) - a(1, 1) * a(2, 0));
    if det.abs() < 1e-12 || !det.is_finite() {
        return None;
    }
    let cof = |r0: usize, r1: usize, c0: usize, c1: usize| a(r0, c0) * a(r1, c1) - a(r0, c1) * a(r1, c0);
    let inv3 = [
        [cof(1, 2, 1, 2), -cof(0, 2, 1, 2), cof(0, 1, 1, 2)],
        [-cof(1, 2, 0, 2), cof(0, 2, 0, 2), -cof(0, 1, 0, 2)],
        [cof(1, 2, 0, 1), -cof(0, 2, 0, 1), cof(0, 1, 0, 1)],
    ]
    .map(|row| row.map(|x| x / det));
    let mut out = [[0.0f32; 4]; 4];
    for r in 0..3 {
        for c in 0..3 {
            out[r][c] = inv3[r][c] as f32;
        }
    }
    for c in 0..3 {
        out[3][c] = -(0..3).map(|k| a(3, k) * inv3[k][c]).sum::<f64>() as f32;
    }
    out[3][3] = 1.0;
    Some(out)
}

pub fn face_offsets(big_root: &Path, name: &str) -> Result<HashMap<String, [f32; 3]>> {
    let files = crate::character::outfit_crsf_files(big_root, name)?;
    let mut out = HashMap::new();
    for rel in files {
        let path = crate::track_objects::asset_file(big_root, &rel).with_context(|| format!("{rel} not found"))?;
        let file = CrsfFile::parse(&std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?)
            .with_context(|| format!("parsing {}", path.display()))?;
        for o in file.face_offsets.iter().flatten() {
            out.insert(o.joint.clone(), o.translate);
        }
    }
    Ok(out)
}

impl SkeletonCache {
    pub fn get(&self, big_root: &Path, face: Option<&str>) -> Result<Arc<RiderSkeleton>> {
        let key = face.map(str::to_string);
        if let Some(s) = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&key) {
            return Ok(s.clone());
        }
        let made = Arc::new(build(big_root, face)?);
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(key, made.clone());
        Ok(made)
    }
}

fn build(big_root: &Path, face: Option<&str>) -> Result<RiderSkeleton> {
    let path = crate::track_objects::asset_file(big_root, RIG_FILE).with_context(|| format!("{RIG_FILE} not found"))?;
    let rig = CrsfFile::parse(&std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?)
        .with_context(|| format!("parsing {}", path.display()))?
        .scene
        .context("the rig has no SGRF scene graph")?;
    let offsets = match face {
        Some(name) => face_offsets(big_root, name)?,
        None => HashMap::new(),
    };
    let bind = crate::character::bind_pose_of(&rig, &offsets);
    let base_rig = rig.clone();
    let (rig, bind, _) = crate::anim_rigops::extend_character_rig(big_root, &rig, &bind)?;
    let model = model_matrices(&rig, &bind);
    let mut joints = Vec::with_capacity(rig.nodes.len());
    for (n, m) in rig.nodes.iter().zip(&model) {
        let inverse_bind =
            inverse(m).with_context(|| format!("rig node {:?}: a bind matrix without inverse", n.name))?;
        joints.push(SkeletonJoint { name: n.name.clone(), parent: n.parent, inverse_bind });
    }
    let skeleton = Skeleton::new(joints, face.map(|f| format!("ssx:character/{f}"))).map_err(anyhow::Error::msg)?;
    Ok(RiderSkeleton { skeleton, base_rig, rig, bind, face_offsets: offsets })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_inverse_of_a_bind_matrix() {
        let m: RowMatrix = [[0.0, 2.0, 0.0, 0.0], [-2.0, 0.0, 0.0, 0.0], [0.0, 0.0, 2.0, 0.0], [1.0, 2.0, 3.0, 1.0]];
        let inv = inverse(&m).unwrap();
        let id = ssx_content::skeleton::mul(&m, &inv);
        for (r, row) in id.iter().enumerate() {
            for (c, v) in row.iter().enumerate() {
                assert!((v - if r == c { 1.0 } else { 0.0 }).abs() < 1e-6, "{id:?}");
            }
        }
        assert!(inverse(&[[0.0; 4]; 4]).is_none());
    }
}
