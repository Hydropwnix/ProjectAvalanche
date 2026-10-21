use ssx_formats::crsf::SceneGraph;

use super::Effector;
use crate::character::Srt;
use crate::rigamate::{Sqt, V4, compose, local_from_world, quat_mul};

type V3 = [f32; 3];

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale(a: V3, k: f32) -> V3 {
    a.map(|x| x * k)
}
fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn len(a: V3) -> f32 {
    dot(a, a).sqrt()
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn conj(q: V4) -> V4 {
    [-q[0], -q[1], -q[2], q[3]]
}
const IDENTITY: V4 = [0.0, 0.0, 0.0, 1.0];

fn normalize(q: V4) -> V4 {
    let l = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if l > 0.0 { q.map(|x| x / l) } else { IDENTITY }
}
fn mul(a: V4, b: V4) -> V4 {
    normalize(quat_mul(a, b))
}
fn rot(q: V4, v: V3) -> V3 {
    let r = crate::rigamate::rotate(q, [v[0], v[1], v[2], 0.0]);
    [r[0], r[1], r[2]]
}
fn axis_angle(a: V3, angle: f32) -> V4 {
    let (s, c) = (angle * 0.5).sin_cos();
    [a[0] * s, a[1] * s, a[2] * s, c]
}

fn arc(a: V3, b: V3) -> V4 {
    let c = cross(a, b);
    let q = [c[0], c[1], c[2], (dot(a, a) * dot(b, b)).sqrt() + dot(a, b)];
    // TODO(guess): a zero-length or opposite `a`, `b` gives a zero quaternion the game normalises to NaN (vrsqrtefp of 0); the port keeps the joint; settle: what the NaN pose shows (0x8321BF60…0x8321C054, and the pose validation after HumanIK — op 6 of `RigOpList_Character` — in op 10 `ValidatePose_Player_PrePhysics`; op 0's validation has no kernel in retail, 0x82CE8958).
    normalize(q)
}

fn twist(q: V4, axis: V3) -> V4 {
    mul(arc(rot(q, axis), axis), q)
}

fn interior(l1: f32, l2: f32, d: f32) -> f32 {
    let c = (l1 * l1 + l2 * l2 - d * d) / (2.0 * l1 * l2);
    if c.is_finite() { c.clamp(-1.0, 1.0).acos() } else { std::f32::consts::PI }
}

fn smoothstep(c: f32, lo: f32, hi: f32) -> f32 {
    let f = ((c - lo) / (hi - lo)).clamp(0.0, 1.0);
    f * f * (3.0 - 2.0 * f)
}

fn partial(q: V4, k: f32) -> V4 {
    if k <= 0.0 { IDENTITY } else { normalize([q[0] * k, q[1] * k, q[2] * k, q[3]]) }
}

#[derive(Clone, Debug)]
pub struct HikLeg {
    nodes: [usize; 4],
    off: [V4; 4],
    shin: V3,
    hinge: V3,
}

#[derive(Clone, Debug)]
pub struct HumanIk {
    pub legs: [HikLeg; 2],
}

fn bind_of(rig: &SceneGraph) -> Vec<Sqt> {
    rig.nodes
        .iter()
        .map(|n| {
            let t = n.attribute("translate").unwrap_or([0.0; 3]);
            let s = n.attribute("scale").unwrap_or([1.0; 3]);
            let q = ssx_formats::ant_anim::euler_xyz_to_quat(n.attribute("rotate").unwrap_or([0.0; 3]));
            Sqt { s: [s[0], s[1], s[2], 1.0], q, t: [t[0], t[1], t[2], 0.0] }
        })
        .collect()
}

fn globals(rig: &SceneGraph, local: &[Sqt]) -> Vec<Sqt> {
    let mut out: Vec<Sqt> = Vec::with_capacity(local.len());
    for (i, l) in local.iter().enumerate() {
        let g = match rig.nodes[i].parent {
            Some(p) if p < i => compose(&out[p], l),
            Some(p) => {
                let mut g = *l;
                let mut q = Some(p);
                while let Some(j) = q {
                    g = compose(&local[j], &g);
                    q = rig.nodes[j].parent;
                }
                g
            }
            None => *l,
        };
        out.push(g);
    }
    out
}

fn facing(rig: &SceneGraph, g: &[Sqt]) -> V4 {
    let (Some(h), Some(l), Some(r)) = (rig.find("Hips"), rig.find("LeftHand"), rig.find("RightHand")) else {
        return IDENTITY;
    };
    let p = |i: usize| [g[i].t[0], g[i].t[1], g[i].t[2]];
    let (a0, b0) = (sub(p(l), p(h)), sub(p(r), p(h)));
    for k in 0..4 {
        let q = axis_angle([0.0, 1.0, 0.0], k as f32 * std::f32::consts::FRAC_PI_2);
        let (a, b) = (rot(q, a0), rot(q, b0));
        let sideways = a[2].abs() <= 0.5 * a[0].abs() && b[2].abs() <= 0.5 * b[0].abs();
        if sideways && a[0] > 0.0 && b[0] < 0.0 && 0.25 * (a[0] - b[0]) > a[0] + b[0] {
            return q;
        }
    }
    IDENTITY
}

impl HumanIk {
    pub fn new(rig: &SceneGraph) -> Option<Self> {
        let g = globals(rig, &bind_of(rig));
        Self::characterise(rig, &g)
    }

    // TODO(guess): the formulas reproduce the emulated characterisation of `genchar_rig` (every rider's legs) — the characterisation itself (0x83203538 → 0x83209E60, ~3 900 instructions) is not traced, so for other legs they are a hypothesis; settle: the knee frame's set-up in 0x83209E60 / 0x83209A30.
    pub fn characterise(rig: &SceneGraph, g: &[Sqt]) -> Option<Self> {
        let yaw = facing(rig, g);
        let leg = |s: &str| -> Option<HikLeg> {
            let f = |n: &str| rig.find(&format!("{s}{n}"));
            let nodes = [f("UpLeg")?, f("Leg")?, f("Foot")?, f("ToeBase")?];
            let off = nodes.map(|n| mul(yaw, g[n].q));
            let p = |i: usize| rot(yaw, [g[nodes[i]].t[0], g[nodes[i]].t[1], g[nodes[i]].t[2]]);
            let d = sub(p(2), p(1));
            let shin = scale(d, 1.0 / len(d));
            let x = sub([1.0, 0.0, 0.0], scale(shin, shin[0]));
            let hinge = scale(x, 1.0 / len(x));
            (len(d) > 0.0 && len(x) > 0.0).then_some(HikLeg { nodes, off, shin, hinge })
        };
        Some(Self { legs: [leg("Left")?, leg("Right")?] })
    }
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

struct Chain {
    t: [V3; 4],
    q: [V4; 4],
}

impl Chain {
    fn turn(&mut self, k: usize, r: V4) {
        self.q[k] = mul(r, self.q[k]);
        for c in k + 1..4 {
            self.t[c] = add(self.t[k], rot(r, sub(self.t[c], self.t[k])));
            self.q[c] = mul(r, self.q[c]);
        }
    }
    fn thigh(&self) -> V3 {
        sub(self.t[1], self.t[0])
    }
    fn shin(&self) -> V3 {
        sub(self.t[2], self.t[1])
    }
}

fn reject(v: V3, a: V3) -> V3 {
    sub(v, scale(a, dot(v, a)))
}

impl HumanIk {
    // TODO(guess): the other solving steps of 0x61F9 — body pull (bit 0x1, 0x83212DC8), leg passes 0x8 / 0x10 (0x83214C88, 0x832196E0, 0x83219F30), arm passes (the limb pass 0x8321FBD0 on bits 0x20 / 0x40), bits 0x2000 (0x832160C8) / 0x4000 — are not ported:
    // the game's HumanIK code, run in a PPC + VMX128 interpreter, gives the same `LowerBody` output from the leg limb passes 0x80 / 0x100 alone (≤ 11 µm on 205 poses; the rig op's pull and resist are 0); settle: trace their code (no-ops for every input is inferred).
    pub fn solve(&self, rig: &SceneGraph, pose: &mut [Srt], effectors: &[Effector; 2]) {
        for (leg, e) in self.legs.iter().zip(effectors).rev() {
            leg.solve(rig, pose, e);
        }
    }
}

impl HikLeg {
    fn solve(&self, rig: &SceneGraph, pose: &mut [Srt], e: &Effector) {
        let w = self.nodes.map(|n| world(rig, pose, n));
        let mut c = Chain {
            t: w.map(|s| [s.t[0], s.t[1], s.t[2]]),
            q: std::array::from_fn(|k| mul(w[k].q, conj(self.off[k]))),
        };
        let a = rot(c.q[0], self.hinge);
        let (u, s) = (reject(c.thigh(), a), reject(c.shin(), a));
        if dot(cross(u, s), a) < 0.0 {
            c.turn(1, arc(s, u));
        }
        let (u, s) = (c.thigh(), c.shin());
        let cos_us = dot(u, s) / (len(u) * len(s));
        let fade = if cos_us.is_finite() {
            smoothstep(cos_us, 30f32.to_radians().cos(), 15f32.to_radians().cos())
        } else {
            1.0
        };
        let (p, n) = (cross(u, cross(a, u)), cross(u, s));
        let r =
            if dot(n, n) > 0.0 && dot(p, p) > 0.0 && fade < 1.0 { partial(arc(p, n), 1.0 - fade) } else { IDENTITY };
        let roll = mul(conj(c.q[0]), mul(r, c.q[0]));
        c.q[0] = mul(r, c.q[0]);
        let twist0 = twist(mul(conj(c.q[1]), c.q[2]), self.shin);
        if e.reach > 1e-20 {
            self.reach(&mut c, e);
        }
        let twist1 = twist(mul(conj(c.q[1]), c.q[2]), self.shin);
        c.q[0] = mul(c.q[0], conj(roll));
        c.q[1] = mul(c.q[1], mul(conj(twist0), twist1));
        for k in 0..4 {
            let n = self.nodes[k];
            let parent = rig.nodes[n].parent.map_or(Sqt::IDENTITY, |p| world(rig, pose, p));
            let q = mul(c.q[k], self.off[k]);
            pose[n].rotation = local_from_world(&parent, &Sqt { q, ..parent }).q;
        }
    }
}

impl HikLeg {
    fn reach(&self, c: &mut Chain, e: &Effector) {
        let reach = e.reach;
        let a = rot(c.q[0], self.hinge);
        let (u, s) = (c.thigh(), c.shin());
        let (up, sp) = (reject(u, a), reject(s, a));
        let (l1, l2, d) = (len(up), len(sp), len(add(up, sp)));
        let open = axis_angle(a, interior(l1, l2, d.min(l1 + l2)) - std::f32::consts::PI);
        let max_len = len(sub(add(c.t[1], rot(open, s)), c.t[0]));
        let target = [e.t[0], e.t[1], e.t[2]];
        let to = sub(target, c.t[0]);
        let dist = len(to);
        let target = if dist > 0.0 && max_len > 0.0 {
            add(c.t[0], scale(to, (dist / max_len).min(1.0) * max_len / dist))
        } else {
            target
        };
        let aim = |c: &mut Chain| {
            let v = sub(c.t[2], c.t[0]);
            c.turn(0, arc(v, add(v, scale(sub(target, c.t[2]), reach))));
        };
        aim(c);
        let a = rot(c.q[0], self.hinge);
        let (u, s) = (c.thigh(), c.shin());
        let (up, sp) = (reject(u, a), reject(s, a));
        let (l1, l2, now) = (len(up), len(sp), len(add(up, sp)));
        let along = dot(u, a) + dot(s, a);
        let to2 = dot(sub(target, c.t[0]), sub(target, c.t[0]));
        let want = if to2 > along * along { (to2 - along * along).sqrt() } else { 0.0 };
        let want = (now + reach * (want - now)).min(l1 + l2);
        c.turn(1, axis_angle(a, interior(l1, l2, now.min(l1 + l2)) - interior(l1, l2, want)));
        aim(c);
        let qe = mul(e.q, conj(self.off[2]));
        let q = c.q[2];
        let sign = if (0..4).map(|i| q[i] * qe[i]).sum::<f32>().is_sign_negative() { -1.0 } else { 1.0 };
        let target_q = normalize(std::array::from_fn(|i| q[i] + reach * (qe[i] * sign - q[i])));
        c.turn(2, mul(target_q, conj(q)));
        c.q[2] = target_q;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssx_formats::crsf::{Attribute, NodeKind, SceneNode};

    fn rig(knee_z: f32) -> (SceneGraph, Vec<Srt>) {
        let mut nodes = Vec::new();
        let mut node = |name: &str, parent: Option<usize>, t: [f32; 3]| {
            let attributes = Some(vec![Attribute { name: "translate".into(), value: t }]);
            nodes.push(SceneNode {
                parent,
                child_count: 0,
                name: name.into(),
                unk_a: 0,
                flags: 0,
                attributes,
                unk_group_flag: 1,
                kind: NodeKind::Group,
            });
            nodes.len() - 1
        };
        let hips = node("Hips", None, [0.0, 1.0, 0.0]);
        node("LeftHand", Some(hips), [0.7, 0.5, 0.0]);
        node("RightHand", Some(hips), [-0.7, 0.5, 0.0]);
        for (side, x) in [("Left", 0.1), ("Right", -0.1)] {
            let up = node(&format!("{side}UpLeg"), Some(hips), [x, 0.0, 0.0]);
            let knee = node(&format!("{side}Leg"), Some(up), [0.0, -0.45, knee_z]);
            let foot = node(&format!("{side}Foot"), Some(knee), [0.0, -0.45, -knee_z]);
            node(&format!("{side}ToeBase"), Some(foot), [0.0, -0.05, 0.12]);
        }
        let rig = SceneGraph { nodes };
        let bind = rig
            .nodes
            .iter()
            .map(|n| Srt { scale: [1.0; 3], rotation: IDENTITY, translation: n.attribute("translate").unwrap() })
            .collect();
        (rig, bind)
    }

    fn pos(rig: &SceneGraph, pose: &[Srt], name: &str) -> V3 {
        let w = world(rig, pose, rig.find(name).unwrap());
        [w.t[0], w.t[1], w.t[2]]
    }

    fn effector(t: V3, reach: f32) -> Effector {
        Effector { q: IDENTITY, t: [t[0], t[1], t[2], 0.0], reach }
    }

    #[test]
    fn reaches_a_target_bending_the_knee_forward() {
        let (rig, mut pose) = rig(0.03);
        let hik = HumanIk::new(&rig).unwrap();
        let target = add(pos(&rig, &pose, "LeftFoot"), [0.0, 0.25, 0.05]);
        let knee0 = pos(&rig, &pose, "LeftLeg");
        hik.solve(&rig, &mut pose, &[effector(target, 1.0), effector([0.0; 3], 0.0)]);
        assert!(len(sub(pos(&rig, &pose, "LeftFoot"), target)) < 1e-4);
        assert!(pos(&rig, &pose, "LeftLeg")[2] > knee0[2] + 0.05, "the knee bends forward (its hinge side)");
        let foot = pose[rig.find("LeftFoot").unwrap()].rotation;
        let w = world(&rig, &pose, rig.find("LeftFoot").unwrap()).q;
        assert!((w[3].abs() - 1.0).abs() < 1e-5, "the foot takes the effector's rotation: {foot:?}");
    }

    #[test]
    fn target_out_of_reach_straightens_the_leg_towards_it() {
        let (rig, mut pose) = rig(0.03);
        let hik = HumanIk::new(&rig).unwrap();
        let hip = pos(&rig, &pose, "LeftUpLeg");
        let target = add(hip, [0.3, -2.0, 0.4]);
        hik.solve(&rig, &mut pose, &[effector(target, 1.0), effector([0.0; 3], 0.0)]);
        let foot = sub(pos(&rig, &pose, "LeftFoot"), hip);
        let full = 2.0 * (0.45f32 * 0.45 + 0.03 * 0.03).sqrt();
        assert!((len(foot) - full).abs() < 1e-3, "at full extension: {} vs {full}", len(foot));
        let dir = sub(target, hip);
        assert!(dot(foot, dir) / (len(foot) * len(dir)) > 0.9999, "towards the target");
    }

    #[test]
    fn zero_reach_keeps_a_bent_leg() {
        let (rig, mut pose) = rig(0.03);
        let before = pose.clone();
        let hik = HumanIk::new(&rig).unwrap();
        hik.solve(&rig, &mut pose, &[effector([5.0; 3], 0.0), effector([5.0; 3], 0.0)]);
        for (a, b) in pose.iter().zip(&before) {
            let d: f32 = (0..4).map(|k| (a.rotation[k] - b.rotation[k]).abs()).sum();
            assert!(d < 1e-5, "{:?} vs {:?}", a.rotation, b.rotation);
        }
    }

    #[test]
    fn hyperextended_knee_is_straightened() {
        let (rig, mut pose) = rig(-0.05);
        let hik = HumanIk::new(&rig).unwrap();
        hik.solve(&rig, &mut pose, &[effector([0.0; 3], 0.0), effector([0.0; 3], 0.0)]);
        let (h, k, a) = (pos(&rig, &pose, "LeftUpLeg"), pos(&rig, &pose, "LeftLeg"), pos(&rig, &pose, "LeftFoot"));
        let side = cross(sub(k, h), sub(a, k))[0];
        assert!(side.abs() < 1e-5, "thigh and shin in line: {side}");
    }

    #[test]
    fn degenerate_targets_stay_finite() {
        for (knee_z, offset) in [(0.03, [0.0; 3]), (0.0, [0.0, -0.9, 0.0]), (0.0, [0.0, -0.5, 0.0])] {
            let (rig, mut pose) = rig(knee_z);
            let hik = HumanIk::new(&rig).unwrap();
            let target = add(pos(&rig, &pose, "LeftUpLeg"), offset);
            hik.solve(&rig, &mut pose, &[effector(target, 1.0), effector(target, 0.5)]);
            assert!(pose.iter().all(|s| s.rotation.iter().all(|x| x.is_finite())), "knee {knee_z}, {offset:?}");
        }
    }
}
