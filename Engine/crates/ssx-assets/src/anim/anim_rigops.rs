use ssx_formats::ant_anim::Skeleton;
use ssx_formats::crsf::{NodeKind, SceneGraph, SceneNode};

use crate::character::{Srt, model_matrices};
use crate::rigamate::{Sqt, V4, compose, rotate};

pub mod humanik;
use humanik::HumanIk;

pub fn extend_rig(
    rig: &SceneGraph,
    bind: &[Srt],
    skeleton: &Skeleton,
    defaults: &std::collections::HashMap<String, Sqt>,
) -> (SceneGraph, Vec<Srt>, Vec<String>) {
    let mut rig = rig.clone();
    let mut bind = bind.to_vec();
    let mut added = Vec::new();
    for j in &skeleton.joints {
        if rig.find(&j.name).is_some() {
            continue;
        }
        let parent = j.parent.and_then(|p| rig.find(&skeleton.joints[p].name));
        let Some(parent) = parent else { continue };
        rig.nodes.push(SceneNode {
            parent: Some(parent),
            child_count: 0,
            name: j.name.clone(),
            unk_a: 0,
            flags: 0,
            attributes: None,
            unk_group_flag: 0,
            kind: NodeKind::Group,
        });
        let d = defaults.get(&j.name).copied().unwrap_or(Sqt::IDENTITY);
        bind.push(Srt { scale: [d.s[0], d.s[1], d.s[2]], rotation: d.q, translation: [d.t[0], d.t[1], d.t[2]] });
        added.push(j.name.clone());
    }
    (rig, bind, added)
}

fn sqt(s: &Srt) -> Sqt {
    Sqt {
        s: [s.scale[0], s.scale[1], s.scale[2], 1.0],
        q: s.rotation,
        t: [s.translation[0], s.translation[1], s.translation[2], 0.0],
    }
}

fn world(rig: &SceneGraph, pose: &[Srt], node: usize) -> Sqt {
    let mut out = sqt(&pose[node]);
    let mut p = rig.nodes[node].parent;
    while let Some(i) = p {
        out = compose(&sqt(&pose[i]), &out);
        p = rig.nodes[i].parent;
    }
    out
}

fn parent_world(rig: &SceneGraph, pose: &[Srt], n: usize) -> Sqt {
    rig.nodes[n].parent.map_or(Sqt::IDENTITY, |p| world(rig, pose, p))
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Effector {
    pub q: V4,
    pub t: V4,
    pub reach: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Compression {
    pub riding_height: f32,
    pub lean_modifier: f32,
    pub chatter: f32,
    pub reach_value: f32,
    pub reach: f32,
    pub ground_up: [f32; 3],
}

#[derive(Clone, Debug)]
pub struct LegRigOps {
    sides: [[usize; 4]; 2],
    board: usize,
    trajectory: usize,
    height: Option<usize>,
    hik: HumanIk,
    pub effectors: [Effector; 2],
}

const NEAR: f32 = 0.2;
const FAR: f32 = 0.4;
const GROUND_RADIUS: f32 = 0.1;
const GROUND_ELEVATION: f32 = -0.0125;
const REACH: f32 = 1.0;

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn len(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}

impl LegRigOps {
    pub fn new(rig: &SceneGraph, floats: &[String]) -> Option<Self> {
        let f = |n: &str| rig.find(n);
        let side = |s: &str| -> Option<[usize; 4]> {
            Some([
                f(&format!("{s}AnkleEffectorAux"))?,
                f(&format!("{s}UpLeg"))?,
                f(&format!("{s}Leg"))?,
                f(&format!("{s}Foot"))?,
            ])
        };
        Some(Self {
            sides: [side("Left")?, side("Right")?],
            board: f("Board")?,
            trajectory: f("AITrajectory")?,
            height: floats.iter().position(|n| n == "TrajectoryVelocityRig.AITrajectory.height"),
            hik: HumanIk::new(rig)?,
            effectors: [Effector::default(); 2],
        })
    }

    pub fn clip_height(&self, floats: &[f32]) -> f32 {
        self.height.and_then(|i| floats.get(i)).copied().unwrap_or(0.0)
    }
}

impl LegRigOps {
    pub fn apply(&mut self, rig: &SceneGraph, pose: &mut [Srt], floats: &[f32], compression: Option<Compression>) {
        self.apply_with(rig, pose, floats, compression, false);
    }

    pub fn apply_with(
        &mut self,
        rig: &SceneGraph,
        pose: &mut [Srt],
        floats: &[f32],
        compression: Option<Compression>,
        invalid_dofs: bool,
    ) {
        for (k, side) in self.sides.iter().enumerate() {
            self.effectors[k] = Self::pin(rig, pose, side[0]);
            if invalid_dofs {
                self.effectors[k].reach = 0.0;
            }
        }
        if let Some(c) = compression {
            self.compress(rig, pose, floats, &c);
        }
        self.hik.solve(rig, pose, &self.effectors);
    }

    fn pin(rig: &SceneGraph, pose: &[Srt], joint: usize) -> Effector {
        let mut l = sqt(&pose[joint]);
        if l.t[0] * l.t[0] + l.t[2] * l.t[2] < GROUND_RADIUS * GROUND_RADIUS {
            l.t[1] = l.t[1].max(GROUND_ELEVATION);
        }
        let parent = parent_world(rig, pose, joint);
        let w = compose(&parent, &l);
        let d = len([l.t[0], l.t[1], l.t[2]]);
        let fade = if d <= NEAR {
            1.0
        } else if d >= FAR {
            0.0
        } else {
            (FAR - d) / (FAR - NEAR)
        };
        Effector { q: ssx_formats::ant_anim::normalize(w.q), t: w.t, reach: REACH.min(fade) }
    }

    fn compress(&mut self, rig: &SceneGraph, pose: &mut [Srt], floats: &[f32], c: &Compression) {
        let h = self.clip_height(floats) * c.lean_modifier + c.riding_height + c.chatter;
        let k = h.max(0.0) * c.reach_value * c.reach;
        if k == 0.0 {
            return;
        }
        let off = [c.ground_up[0] * k, c.ground_up[1] * k, c.ground_up[2] * k, 0.0];
        for e in &mut self.effectors {
            e.t = [e.t[0] + off[0], e.t[1] + off[1], e.t[2] + off[2], e.t[3]];
        }
        let traj = world(rig, pose, self.trajectory);
        let local = rotate([-traj.q[0], -traj.q[1], -traj.q[2], traj.q[3]], off);
        let b = &mut pose[self.board].translation;
        *b = [b[0] + local[0], b[1] + local[1], b[2] + local[2]];
    }
}

impl LegRigOps {
    pub fn place_board(&self, rig: &SceneGraph, pose: &mut [Srt], q: V4, t: V4) {
        place_joint(rig, pose, self.board, q, t);
    }
}

pub fn place_joint(rig: &SceneGraph, pose: &mut [Srt], j: usize, q: V4, t: V4) {
    pose[j].rotation = [0.0, 0.0, 0.0, 1.0];
    pose[j].translation = [0.0; 3];
    let g = world(rig, pose, j);
    let inv_s = [1.0 / g.s[0], 1.0 / g.s[1], 1.0 / g.s[2], 1.0];
    let cq = [-g.q[0], -g.q[1], -g.q[2], g.q[3]];
    let ts = rotate(cq, [g.t[0] * inv_s[0], g.t[1] * inv_s[1], g.t[2] * inv_s[2], 0.0]);
    let inv = Sqt { s: inv_s, q: cq, t: [-ts[0], -ts[1], -ts[2], 0.0] };
    let local = compose(&inv, &Sqt { s: [1.0; 4], q, t });
    pose[j] = Srt {
        scale: [local.s[0], local.s[1], local.s[2]],
        rotation: local.q,
        translation: [local.t[0], local.t[1], local.t[2]],
    };
}

pub fn joint_positions(rig: &SceneGraph, pose: &[Srt]) -> Vec<[f32; 3]> {
    model_matrices(rig, pose).iter().map(|m| [m[3][0], m[3][1], m[3][2]]).collect()
}

pub fn extend_character_rig(
    big_root: &std::path::Path,
    rig: &SceneGraph,
    bind: &[Srt],
) -> anyhow::Result<(SceneGraph, Vec<Srt>, Vec<String>)> {
    let all = crate::content::data::tuning::anim_all(big_root)?;
    let skeleton = Skeleton::of_rig(&all, "genchar_rig")?;
    let defaults = crate::rigamate::rig_defaults(&all, "genchar_rig")?;
    Ok(extend_rig(rig, bind, &skeleton, &defaults))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssx_formats::crsf::SceneNode;

    fn node(parent: Option<usize>) -> SceneNode {
        SceneNode {
            parent,
            child_count: 0,
            name: String::new(),
            unk_a: 0,
            flags: 0,
            attributes: None,
            unk_group_flag: 0,
            kind: NodeKind::Group,
        }
    }

    #[test]
    fn the_board_lands_on_the_particle() {
        let rig = SceneGraph { nodes: vec![node(None), node(Some(0)), node(Some(1)), node(Some(2))] };
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let mut pose = vec![
            Srt { scale: [1.0; 3], rotation: [0.0, h, 0.0, h], translation: [1.0, 2.0, 3.0] },
            Srt { scale: [1.0; 3], rotation: [h, 0.0, 0.0, h], translation: [0.0, 0.9, 0.1] },
            Srt { scale: [1.0; 3], rotation: [0.0, 0.0, 0.6, 0.8], translation: [0.3, -0.2, 0.5] },
            Srt { scale: [1.0; 3], rotation: [0.0, 0.0, 0.0, 1.0], translation: [0.24, 0.115, 0.0] },
        ];
        let (q, t) = ([0.0, 0.0, 1.0, 0.0], [-4.0, 0.5, 7.0, 0.0]);
        place_joint(&rig, &mut pose, 2, q, t);
        let w = world(&rig, &pose, 2);
        assert!(w.t.iter().zip(&t).take(3).all(|(a, b)| (a - b).abs() < 1e-5), "{w:?}");
        assert!(w.s.iter().take(3).all(|s| (s - 1.0).abs() < 1e-6));
        let d: f32 = w.q.iter().zip(&q).map(|(a, b)| a * b).sum();
        assert!(d.abs() > 1.0 - 1e-6);
        let c = world(&rig, &pose, 3);
        let off = rotate(q, [0.24, 0.115, 0.0, 0.0]);
        assert!((0..3).all(|i| (c.t[i] - (t[i] + off[i])).abs() < 1e-5));
    }
}
