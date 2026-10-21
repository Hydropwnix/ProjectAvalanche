use std::collections::HashMap;

use ssx_formats::ant_anim::{Dof, DofType, nlerp, normalize};
use ssx_formats::crsf::SceneGraph;

use crate::anim_graph::{DofCurves, Mask};
use crate::rig_anim::RigPose;

#[derive(Clone, Debug)]
pub struct PoseRig {
    pub mirror_nodes: Vec<usize>,
    pub mirror_floats: Vec<usize>,
    layout_dofs: HashMap<String, Vec<DofTarget>>,
    nodes: usize,
    floats: usize,
    pub trajectory: Option<usize>,
}

#[derive(Clone, Copy, Debug)]
enum DofTarget {
    Node(usize, char),
    Float(usize),
    None,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PoseMask {
    pub nodes: Vec<[f32; 3]>,
    pub floats: Vec<f32>,
}

impl PoseMask {
    pub fn weighted(parts: &[(&PoseMask, f32)]) -> Option<PoseMask> {
        let (first, _) = parts.first()?;
        let mut out = PoseMask { nodes: vec![[0.0; 3]; first.nodes.len()], floats: vec![0.0; first.floats.len()] };
        for (m, w) in parts {
            for (o, n) in out.nodes.iter_mut().zip(&m.nodes) {
                for k in 0..3 {
                    o[k] += w * n[k];
                }
            }
            for (o, f) in out.floats.iter_mut().zip(&m.floats) {
                *o += w * f;
            }
        }
        Some(out)
    }
}

pub fn mirror_name(name: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    for (from, to) in [("right", "left"), ("r", "l"), ("left", "right"), ("l", "r")] {
        if let Some(rest) = lower.strip_prefix(from) {
            return Some(format!("{to}{rest}"));
        }
    }
    None
}

impl PoseRig {
    pub fn new(rig: &SceneGraph, dofs: &[Dof], floats: &[String]) -> Self {
        let by_lower: HashMap<String, usize> =
            rig.nodes.iter().enumerate().map(|(i, n)| (n.name.to_ascii_lowercase(), i)).collect();
        let mirror_nodes = rig
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| mirror_name(&n.name).and_then(|m| by_lower.get(&m).copied()).unwrap_or(i))
            .collect();
        let float_lower: HashMap<String, usize> =
            floats.iter().enumerate().map(|(i, f)| (f.to_ascii_lowercase(), i)).collect();
        let mirror_floats = floats
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let (layout, name) = f.split_once('.').unwrap_or(("", f));
                mirror_name(name)
                    .and_then(|m| float_lower.get(&format!("{}.{m}", layout.to_ascii_lowercase())).copied())
                    .unwrap_or(i)
            })
            .collect();
        let mut layout_dofs: HashMap<String, Vec<DofTarget>> = HashMap::new();
        for d in dofs {
            let target = match (d.ty, d.joint()) {
                (DofType::Float, _) => floats
                    .iter()
                    .position(|f| *f == format!("{}.{}", d.layout, d.name))
                    .map_or(DofTarget::None, DofTarget::Float),
                (_, Some((j, c))) => rig.find(j).map_or(DofTarget::None, |n| DofTarget::Node(n, c)),
                _ => DofTarget::None,
            };
            layout_dofs.entry(d.layout.clone()).or_default().push(target);
        }
        Self {
            mirror_nodes,
            mirror_floats,
            layout_dofs,
            nodes: rig.nodes.len(),
            floats: floats.len(),
            trajectory: rig.find("AITrajectory"),
        }
    }

    pub fn mask(&self, m: &Mask) -> PoseMask {
        let mut out = PoseMask { nodes: vec![[0.0; 3]; self.nodes], floats: vec![0.0; self.floats] };
        let mut put = |t: DofTarget, v: f32| match t {
            DofTarget::Node(n, c) => {
                out.nodes[n][match c {
                    'q' => 0,
                    't' => 1,
                    _ => 2,
                }] = v
            }
            DofTarget::Float(f) => out.floats[f] = v,
            DofTarget::None => {}
        };
        for (layout, v) in &m.defaults {
            for &t in self.layout_dofs.get(layout).map(Vec::as_slice).unwrap_or_default() {
                put(t, *v);
            }
        }
        for (layout, values) in &m.slots {
            for (&t, &v) in self.layout_dofs.get(layout).map(Vec::as_slice).unwrap_or_default().iter().zip(values) {
                put(t, v);
            }
        }
        out
    }

    pub fn dof_weights(&self, curves: &DofCurves, t: f32) -> PoseMask {
        let mut out = PoseMask { nodes: vec![[t; 3]; self.nodes], floats: vec![t; self.floats] };
        for set in &curves.sets {
            for (i, &target) in
                self.layout_dofs.get(&set.layout).map(Vec::as_slice).unwrap_or_default().iter().enumerate()
            {
                let w = set.weight(i, t);
                match target {
                    DofTarget::Node(n, c) => {
                        out.nodes[n][match c {
                            'q' => 0,
                            't' => 1,
                            _ => 2,
                        }] = w
                    }
                    DofTarget::Float(f) => out.floats[f] = w,
                    DofTarget::None => {}
                }
            }
        }
        out
    }

    pub fn mirror(&self, p: &AnimPose) -> AnimPose {
        let mut out = p.clone();
        for i in 0..p.pose.nodes.len() {
            let j = self.mirror_nodes[i];
            let (vi, vj) = (p.valid_nodes[i], p.valid_nodes[j]);
            let b = &p.pose.nodes[j];
            let o = &mut out.pose.nodes[i];
            let swap = |bit: u8| if i == j { vi & bit != 0 } else { (vi | vj) & bit != 0 };
            let [x, y, z, w] = b.rotation;
            let [tx, ty, tz] = b.translation;
            if swap(VALID_Q) {
                o.rotation = [-x, -y, z, w];
            }
            if swap(VALID_T) {
                o.translation = [tx, ty, -tz];
            }
            if swap(VALID_S) && i != j {
                o.scale = b.scale;
            }
            if i != j {
                let swapped = (vi | vj) & (VALID_Q | VALID_T | VALID_S);
                out.valid_nodes[i] = (vi & !swapped) | (vj & swapped);
            }
        }
        for i in 0..p.pose.floats.len() {
            let j = self.mirror_floats[i];
            if i != j && (p.valid_floats[i] || p.valid_floats[j]) {
                out.pose.floats[i] = p.pose.floats[j];
                out.valid_floats[i] = p.valid_floats[j];
            }
        }
        out
    }
}

pub const VALID_Q: u8 = 1;
pub const VALID_T: u8 = 2;
pub const VALID_S: u8 = 4;

#[derive(Clone, Debug, PartialEq)]
pub struct AnimPose {
    pub pose: RigPose,
    pub valid_nodes: Vec<u8>,
    pub valid_floats: Vec<bool>,
}

impl AnimPose {
    pub fn empty(rest: &RigPose) -> Self {
        AnimPose {
            pose: rest.clone(),
            valid_nodes: vec![0; rest.nodes.len()],
            valid_floats: vec![false; rest.floats.len()],
        }
    }

    pub fn full(pose: RigPose) -> Self {
        let (n, f) = (pose.nodes.len(), pose.floats.len());
        AnimPose { pose, valid_nodes: vec![VALID_Q | VALID_T | VALID_S; n], valid_floats: vec![true; f] }
    }

    pub fn blend(&mut self, other: &AnimPose, w: f32) {
        blend_dofs(self, other, |_, _| w);
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Take {
    Old,
    New,
    Blend,
}

fn take(old: bool, new: bool) -> Take {
    match (old, new) {
        (_, false) => Take::Old,
        (false, true) => Take::New,
        (true, true) => Take::Blend,
    }
}

fn blend_dofs(a: &mut AnimPose, b: &AnimPose, weight: impl Fn(usize, usize) -> f32) {
    for (i, (x, y)) in a.pose.nodes.iter_mut().zip(&b.pose.nodes).enumerate() {
        let (va, vb) = (a.valid_nodes[i], b.valid_nodes[i]);
        match take(va & VALID_Q != 0, vb & VALID_Q != 0) {
            Take::Blend => x.rotation = nlerp(x.rotation, y.rotation, weight(i, 0)),
            Take::New => x.rotation = y.rotation,
            Take::Old => {}
        }
        match take(va & VALID_T != 0, vb & VALID_T != 0) {
            Take::Blend => x.translation = lerp3(x.translation, y.translation, weight(i, 1)),
            Take::New => x.translation = y.translation,
            Take::Old => {}
        }
        match take(va & VALID_S != 0, vb & VALID_S != 0) {
            Take::Blend => x.scale = lerp3(x.scale, y.scale, weight(i, 2)),
            Take::New => x.scale = y.scale,
            Take::Old => {}
        }
        a.valid_nodes[i] = va | vb;
    }
    for (i, (x, y)) in a.pose.floats.iter_mut().zip(&b.pose.floats).enumerate() {
        match take(a.valid_floats[i], b.valid_floats[i]) {
            Take::Blend => *x += (y - *x) * weight(i, 3),
            Take::New => *x = *y,
            Take::Old => {}
        }
        a.valid_floats[i] |= b.valid_floats[i];
    }
}

impl AnimPose {
    // TODO(guess): a DOF still invalid after `ControllerUpdate` shows the rig default (the character's bind values); the pose job system's buffer initialisation was not traced (command builder 0x831C0270); settle: the rig-op list's read of an invalid joint DOF (`EffectorPinningRigOp` 0x82D08800, `PoseToGlobalSqt`).
    pub fn fill_invalid(&mut self, rest: &RigPose) {
        for (i, (x, r)) in self.pose.nodes.iter_mut().zip(&rest.nodes).enumerate() {
            let v = self.valid_nodes[i];
            if v & VALID_Q == 0 {
                x.rotation = r.rotation;
            }
            if v & VALID_T == 0 {
                x.translation = r.translation;
            }
            if v & VALID_S == 0 {
                x.scale = r.scale;
            }
        }
        for (i, (x, r)) in self.pose.floats.iter_mut().zip(&rest.floats).enumerate() {
            if !self.valid_floats[i] {
                *x = *r;
            }
        }
    }
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t)
}

pub fn blend_masked(base: &mut AnimPose, layer: &AnimPose, w: f32, mask: Option<&PoseMask>) {
    match mask {
        None => base.blend(layer, w),
        Some(m) => blend_dofs(base, layer, |i, c| w * if c == 3 { m.floats[i] } else { m.nodes[i][c] }),
    }
}

pub fn add_layer(base: &mut AnimPose, layer: &AnimPose, w: f32, mask: Option<&PoseMask>, negate: bool) {
    let w = if negate { -w } else { w };
    for (i, (a, b)) in base.pose.nodes.iter_mut().zip(&layer.pose.nodes).enumerate() {
        let both = base.valid_nodes[i] & layer.valid_nodes[i];
        let [wq, wt, ws] = mask.map_or([w; 3], |m| m.nodes[i].map(|x| x * w));
        if both & VALID_Q != 0 {
            let ql = if negate { [-b.rotation[0], -b.rotation[1], -b.rotation[2], b.rotation[3]] } else { b.rotation };
            let r = nlerp([0.0, 0.0, 0.0, 1.0], ql, wq.abs());
            a.rotation = crate::rigamate::quat_mul(a.rotation, r);
        }
        if both & VALID_T != 0 {
            a.translation = std::array::from_fn(|k| a.translation[k] + wt * b.translation[k]);
        }
        if both & VALID_S != 0 {
            a.scale = std::array::from_fn(|k| a.scale[k] + ws * b.scale[k]);
        }
    }
    for (i, (a, b)) in base.pose.floats.iter_mut().zip(&layer.pose.floats).enumerate() {
        if base.valid_floats[i] && layer.valid_floats[i] {
            *a += w * mask.map_or(1.0, |m| m.floats[i]) * b;
        }
    }
}

pub fn difference(p: &AnimPose, reference: &AnimPose) -> AnimPose {
    let mut out = p.clone();
    for (i, (a, r)) in out.pose.nodes.iter_mut().zip(&reference.pose.nodes).enumerate() {
        let v = p.valid_nodes[i];
        if v & VALID_S != 0 {
            a.scale = std::array::from_fn(|k| a.scale[k] - r.scale[k]);
        }
        if v & VALID_Q != 0 {
            let rq = normalize(r.rotation);
            a.rotation = crate::rigamate::quat_mul([-rq[0], -rq[1], -rq[2], rq[3]], a.rotation);
        }
        if v & VALID_T != 0 {
            a.translation = std::array::from_fn(|k| a.translation[k] - r.translation[k]);
        }
    }
    for (i, (a, r)) in out.pose.floats.iter_mut().zip(&reference.pose.floats).enumerate() {
        if p.valid_floats[i] {
            *a -= r;
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Delta {
    None,
    Physics,
    Value { q: [f32; 4], t: [f32; 3] },
    Untraced,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StepPhysics {
    pub q: [f32; 4],
    pub t: [f32; 3],
    pub controllers: Vec<String>,
}

impl Delta {
    pub const ZERO: Delta = Delta::Value { q: [0.0, 0.0, 0.0, 1.0], t: [0.0; 3] };

    fn value(self, physics: Option<&StepPhysics>) -> Option<([f32; 4], [f32; 3])> {
        match self {
            Delta::Value { q, t } => Some((q, t)),
            Delta::Physics => physics.map(|p| (p.q, p.t)),
            _ => None,
        }
    }

    pub fn blend(old: Delta, new: Delta, w: f32, physics: Option<&StepPhysics>) -> Delta {
        match (old, new) {
            (Delta::None, x) | (x, Delta::None) => x,
            _ if w <= 0.0 => old,
            _ if w >= 1.0 => new,
            (Delta::Untraced, _) | (_, Delta::Untraced) => Delta::Untraced,
            (Delta::Physics, Delta::Physics) => Delta::Physics,
            _ => match (old.value(physics), new.value(physics)) {
                (Some((qa, ta)), Some((qb, tb))) => Delta::Value { q: nlerp(qa, qb, w), t: lerp3(ta, tb, w) },
                _ => Delta::Untraced,
            },
        }
    }

    pub fn accumulate(partial: Delta, last: Delta, physics: Option<&StepPhysics>) -> Delta {
        match (partial, last) {
            (Delta::None, x) | (x, Delta::None) => x,
            (Delta::Untraced, _) | (_, Delta::Untraced) => Delta::Untraced,
            (Delta::Physics, Delta::Physics) => Delta::Physics,
            (Delta::Physics, _) => Delta::Untraced,
            _ => match (partial.value(physics), last.value(physics)) {
                (Some((qp, tp)), Some((ql, tl))) => {
                    let r = crate::rigamate::rotate(qp, [tl[0], tl[1], tl[2], 0.0]);
                    Delta::Value { q: crate::rigamate::quat_mul(ql, qp), t: [tp[0] + r[0], tp[1] + r[1], tp[2] + r[2]] }
                }
                _ => Delta::Untraced,
            },
        }
    }

    pub fn mirror(self) -> Delta {
        match self {
            Delta::Value { q, t } => Delta::Value { q: [-q[0], -q[1], q[2], q[3]], t: [t[0], t[1], -t[2]] },
            Delta::None => Delta::None,
            Delta::Physics | Delta::Untraced => Delta::Untraced,
        }
    }

    pub fn zeroed(self) -> Delta {
        match self {
            Delta::Value { .. } | Delta::Physics => Delta::ZERO,
            x => x,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character::Srt;

    fn rig(names: &[&str]) -> SceneGraph {
        let node = |n: &str| ssx_formats::crsf::SceneNode {
            parent: None,
            child_count: 0,
            name: n.to_string(),
            unk_a: 0,
            flags: 0,
            attributes: None,
            unk_group_flag: 0,
            kind: ssx_formats::crsf::NodeKind::Group,
        };
        SceneGraph { nodes: names.iter().map(|n| node(n)).collect() }
    }

    fn srt(q: [f32; 4], t: [f32; 3]) -> Srt {
        Srt { scale: [1.0; 3], rotation: q, translation: t }
    }

    #[test]
    fn blend_takes_the_valid_side() {
        let half = std::f32::consts::FRAC_1_SQRT_2;
        let rest = RigPose { nodes: vec![srt([0.0, 0.0, 0.0, 1.0], [0.0; 3]); 3], floats: vec![0.0] };
        let mut old = AnimPose::empty(&rest);
        let mut new = AnimPose::empty(&rest);
        old.pose.nodes[0] = srt([0.0, 0.0, 0.0, 1.0], [1.0, 0.0, 0.0]);
        old.valid_nodes[0] = VALID_Q | VALID_T;
        old.pose.nodes[2].translation = [5.0, 0.0, 0.0];
        old.valid_nodes[2] = VALID_T;
        for n in 0..2 {
            new.pose.nodes[n] = srt([0.0, half, 0.0, half], [3.0, 0.0, 0.0]);
            new.valid_nodes[n] = VALID_Q | VALID_T;
        }
        new.pose.floats[0] = 0.9;
        new.valid_floats[0] = true;
        old.blend(&new, 0.25);
        assert_eq!(old.pose.nodes[0].translation, [1.5, 0.0, 0.0]);
        assert_eq!(old.pose.nodes[1].translation, [3.0, 0.0, 0.0]);
        assert_eq!(old.pose.nodes[1].rotation, [0.0, half, 0.0, half]);
        assert_eq!(old.pose.nodes[2].translation, [5.0, 0.0, 0.0]);
        assert_eq!(old.pose.floats[0], 0.9);
        assert_eq!(old.valid_nodes, vec![VALID_Q | VALID_T, VALID_Q | VALID_T, VALID_T]);
        let mut nothing = AnimPose::empty(&rest);
        nothing.blend(&new, 0.01);
        assert_eq!(nothing.pose.nodes[1], new.pose.nodes[1]);
    }

    #[test]
    fn mirror_touches_valid_dofs_only() {
        let r = rig(&["LeftLeg", "RightLeg", "Hips", "RM_LeftBoardFlex1"]);
        let pr = PoseRig::new(&r, &[], &[]);
        let q = [0.1, 0.2, 0.3, 0.927];
        let rest = RigPose { nodes: vec![srt(q, [0.0, 0.0, 0.5]); 4], floats: Vec::new() };
        let mut p = AnimPose::empty(&rest);
        p.pose.nodes[0] = srt([0.5, 0.0, 0.0, 0.866], [0.0, 0.0, 0.2]);
        p.valid_nodes[0] = VALID_Q | VALID_T;
        p.valid_nodes[2] = VALID_Q;
        let m = pr.mirror(&p);
        assert_eq!(m.pose.nodes[1].rotation, [-0.5, 0.0, 0.0, 0.866]);
        assert_eq!(m.pose.nodes[1].translation, [0.0, 0.0, -0.2]);
        assert_eq!((m.valid_nodes[0], m.valid_nodes[1]), (0, VALID_Q | VALID_T));
        assert_eq!(m.pose.nodes[2].rotation, [-0.1, -0.2, 0.3, 0.927]);
        assert_eq!(m.pose.nodes[2].translation, [0.0, 0.0, 0.5]);
        assert_eq!(m.pose.nodes[3], rest.nodes[3]);
    }

    #[test]
    fn additive_needs_both_sides() {
        let rest = RigPose { nodes: vec![srt([0.0, 0.0, 0.0, 1.0], [1.0, 0.0, 0.0]); 2], floats: Vec::new() };
        let mut base = AnimPose::empty(&rest);
        base.valid_nodes[0] = VALID_T;
        let mut layer = AnimPose::empty(&rest);
        layer.pose.nodes[0].translation = [0.0, 2.0, 0.0];
        layer.pose.nodes[1].translation = [0.0, 2.0, 0.0];
        layer.valid_nodes = vec![VALID_T, VALID_T];
        add_layer(&mut base, &layer, 0.5, None, false);
        assert_eq!(base.pose.nodes[0].translation, [1.0, 1.0, 0.0]);
        assert_eq!(base.pose.nodes[1].translation, [1.0, 0.0, 0.0]);
        assert_eq!(base.valid_nodes, vec![VALID_T, 0]);
    }

    #[test]
    fn delta_ops() {
        let a = Delta::Value { q: [0.0, 0.0, 0.0, 1.0], t: [1.0, 0.0, 0.0] };
        let half = std::f32::consts::FRAC_1_SQRT_2;
        let b = Delta::Value { q: [0.0, half, 0.0, half], t: [0.0, 0.0, 2.0] };
        assert_eq!(Delta::blend(Delta::None, b, 0.3, None), b);
        assert_eq!(Delta::blend(a, Delta::None, 0.3, None), a);
        let Delta::Value { t, .. } = Delta::blend(a, b, 0.5, None) else { panic!() };
        assert_eq!(t, [0.5, 0.0, 1.0]);
        assert_eq!(Delta::blend(a, Delta::Physics, 0.5, None), Delta::Untraced);
        let p = StepPhysics { q: [0.0, 0.0, 0.0, 1.0], t: [0.0, 1.0, 0.0], controllers: Vec::new() };
        let Delta::Value { t, .. } = Delta::blend(a, Delta::Physics, 0.5, Some(&p)) else { panic!() };
        assert_eq!(t, [0.5, 0.5, 0.0]);
        let Delta::Value { q, t } = Delta::accumulate(b, a, None) else { panic!() };
        assert!(t[0].abs() < 1e-5 && t[1].abs() < 1e-5 && (t[2] - 1.0).abs() < 1e-5, "{t:?}");
        assert!((q[1] - half).abs() < 1e-6 && (q[3] - half).abs() < 1e-6);
        assert_eq!(Delta::accumulate(Delta::None, a, None), a);
        assert_eq!(b.mirror(), Delta::Value { q: [0.0, -half, 0.0, half], t: [0.0, 0.0, -2.0] });
    }

    #[test]
    fn mirror_names() {
        assert_eq!(mirror_name("RightArm").as_deref(), Some("leftarm"));
        assert_eq!(mirror_name("LeftAnkleEffectorAux").as_deref(), Some("rightankleeffectoraux"));
        assert_eq!(mirror_name("r_middle_root").as_deref(), Some("l_middle_root"));
        assert_eq!(mirror_name("RM_RightArmTwist1").as_deref(), Some("lm_rightarmtwist1"));
        assert_eq!(mirror_name("Hips"), None);
    }
}
