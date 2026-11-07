use anyhow::{Context, Result, bail};
use ssx_formats::ant_anim::Skeleton;
use ssx_formats::rigamate::{JointAccess, KeyShape, MathKind, Op, Program};

pub type V4 = [f32; 4];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sqt {
    pub s: V4,
    pub q: V4,
    pub t: V4,
}

impl Sqt {
    pub const IDENTITY: Sqt = Sqt { s: [1.0; 4], q: [0.0, 0.0, 0.0, 1.0], t: [0.0; 4] };
}

fn add(a: V4, b: V4) -> V4 {
    std::array::from_fn(|i| a[i] + b[i])
}
fn sub(a: V4, b: V4) -> V4 {
    std::array::from_fn(|i| a[i] - b[i])
}
fn mul(a: V4, b: V4) -> V4 {
    std::array::from_fn(|i| a[i] * b[i])
}
fn scale(a: V4, k: f32) -> V4 {
    a.map(|x| x * k)
}
fn splat(x: f32) -> V4 {
    [x; 4]
}
fn dot3(a: V4, b: V4) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross3(a: V4, b: V4) -> V4 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0], 0.0]
}
fn normalize3(v: V4) -> V4 {
    let l = dot3(v, v).sqrt();
    if l > 0.0 { scale(v, 1.0 / l) } else { [0.0; 4] }
}
fn conj(q: V4) -> V4 {
    [-q[0], -q[1], -q[2], q[3]]
}

pub fn quat_mul(a: V4, b: V4) -> V4 {
    let c = cross3(a, b);
    [
        a[3] * b[0] + b[3] * a[0] + c[0],
        a[3] * b[1] + b[3] * a[1] + c[1],
        a[3] * b[2] + b[3] * a[2] + c[2],
        a[3] * b[3] - dot3(a, b),
    ]
}

pub fn rotate(q: V4, v: V4) -> V4 {
    let t = add(scale(v, q[3]), cross3(q, v));
    let r = add(v, scale(cross3(q, t), 2.0));
    [r[0], r[1], r[2], v[3]]
}

fn normalize4(q: V4) -> V4 {
    let l = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if l > 0.0 { scale(q, 1.0 / l) } else { q }
}

pub fn compose(parent: &Sqt, child: &Sqt) -> Sqt {
    Sqt {
        s: mul(parent.s, child.s),
        q: quat_mul(parent.q, child.q),
        t: add(parent.t, rotate(parent.q, mul(parent.s, child.t))),
    }
}

pub fn world(skeleton: &Skeleton, pose: &[Sqt], j: usize) -> Sqt {
    let mut out = pose[j];
    let mut p = skeleton.joints[j].parent;
    while let Some(i) = p {
        out = compose(&pose[i], &out);
        p = skeleton.joints[i].parent;
    }
    out.q = normalize4(out.q);
    out
}

pub fn local_from_world(parent: &Sqt, w: &Sqt) -> Sqt {
    let inv_s = parent.s.map(|x| 1.0 / x);
    let cq = conj(parent.q);
    Sqt { s: mul(w.s, inv_s), q: quat_mul(cq, w.q), t: rotate(cq, mul(sub(w.t, parent.t), inv_s)) }
}

fn rows_of(q: V4) -> [[f32; 3]; 3] {
    let [x, y, z, w] = q;
    [
        [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y + z * w), 2.0 * (x * z - y * w)],
        [2.0 * (x * y - z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z + x * w)],
        [2.0 * (x * z + y * w), 2.0 * (y * z - x * w), 1.0 - 2.0 * (x * x + y * y)],
    ]
}

pub fn euler_of_quat(q: V4) -> V4 {
    let m = rows_of(q);
    let cy = (m[0][0] * m[0][0] + m[0][1] * m[0][1]).sqrt();
    if cy > 0.001 {
        [m[1][2].atan2(m[2][2]), (-m[0][2]).atan2(cy), m[0][1].atan2(m[0][0]), 0.0]
    } else {
        [(-m[2][1]).atan2(m[1][1]), (-m[0][2]).atan2(cy), 0.0, 0.0]
    }
}

pub fn quat_of_euler(e: V4) -> V4 {
    let (sx, cx) = (e[0] * 0.5).sin_cos();
    let (sy, cy) = (e[1] * 0.5).sin_cos();
    let (sz, cz) = (e[2] * 0.5).sin_cos();
    [sx * cy * cz - cx * sy * sz, cx * sy * cz + sx * cy * sz, cx * cy * sz - sx * sy * cz, cx * cy * cz + sx * sy * sz]
}

pub fn quat_of_rows(m: [[f32; 3]; 3]) -> V4 {
    let trace = m[0][0] + m[1][1] + m[2][2];
    let q = if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        [(m[1][2] - m[2][1]) / s, (m[2][0] - m[0][2]) / s, (m[0][1] - m[1][0]) / s, 0.25 * s]
    } else if m[0][0] > m[1][1] && m[0][0] > m[2][2] {
        let s = (1.0 + m[0][0] - m[1][1] - m[2][2]).sqrt() * 2.0;
        [0.25 * s, (m[0][1] + m[1][0]) / s, (m[2][0] + m[0][2]) / s, (m[1][2] - m[2][1]) / s]
    } else if m[1][1] > m[2][2] {
        let s = (1.0 + m[1][1] - m[0][0] - m[2][2]).sqrt() * 2.0;
        [(m[0][1] + m[1][0]) / s, 0.25 * s, (m[1][2] + m[2][1]) / s, (m[2][0] - m[0][2]) / s]
    } else {
        let s = (1.0 + m[2][2] - m[0][0] - m[1][1]).sqrt() * 2.0;
        [(m[2][0] + m[0][2]) / s, (m[1][2] + m[2][1]) / s, 0.25 * s, (m[0][1] - m[1][0]) / s]
    };
    normalize4(q)
}

#[derive(Clone, Debug)]
pub struct RigamateInstance {
    pub program: Program,
    tape: Vec<V4>,
    offsets: Vec<V4>,
    parents: Vec<usize>,
    dofs: Vec<Option<usize>>,
    fresh: bool,
}

impl RigamateInstance {
    pub fn new(program: Program, skeleton: &Skeleton, dof_index: impl Fn(&str, &str) -> Option<usize>) -> Result<Self> {
        let n = skeleton.joints.len();
        for (pos, _, op) in &program.ops {
            let joint = match op {
                Op::Joint { joint, access } if !ssx_formats::rigamate::is_virtual(access) => Some(*joint),
                Op::Aim { joint, .. } => Some(*joint),
                _ => None,
            };
            if let Some(j) = joint
                && j as usize >= n
            {
                bail!("{}: operation at {pos} uses joint {j}, the skeleton has {n}", program.name);
            }
        }
        let parents = program.virtual_joints.iter().map(|v| v.0 as usize).collect::<Vec<_>>();
        if let Some(p) = parents.iter().find(|&&p| p >= n) {
            bail!("{}: a virtual joint hangs below joint {p} of {n}", program.name);
        }
        let dofs = program.dofs.iter().map(|(s, d)| dof_index(s, d)).collect();
        Ok(Self {
            tape: program.tape.clone(),
            offsets: program.virtual_joints.iter().map(|v| v.1).collect(),
            parents,
            dofs,
            program,
            fresh: true,
        })
    }

    pub fn missing_dofs(&self) -> Vec<String> {
        self.program
            .dofs
            .iter()
            .zip(&self.dofs)
            .filter(|(_, i)| i.is_none())
            .map(|((s, d), _)| format!("{s}.{d}"))
            .collect()
    }

    pub fn reset_history(&mut self) {
        self.fresh = true;
    }

    fn virtual_world(&self, skeleton: &Skeleton, pose: &[Sqt], v: usize) -> Sqt {
        let p = world(skeleton, pose, self.parents[v]);
        compose(&p, &Sqt { s: [1.0; 4], q: [0.0, 0.0, 0.0, 1.0], t: self.offsets[v] })
    }

    fn parent_world(skeleton: &Skeleton, pose: &[Sqt], j: usize) -> Sqt {
        skeleton.joints[j].parent.map_or(Sqt::IDENTITY, |p| world(skeleton, pose, p))
    }

    pub fn run(&mut self, skeleton: &Skeleton, pose: &mut [Sqt], dofs: &[f32]) {
        let reset = std::mem::replace(&mut self.fresh, false);
        for i in 0..self.program.ops.len() {
            let op = &self.program.ops[i].2;
            match op {
                Op::Joint { joint, access } => {
                    let (j, a) = (*joint as usize, *access);
                    self.joint(skeleton, pose, j, a);
                }
                Op::Aim { joint, target, up } => {
                    let (j, target, up) = (*joint as usize, self.tape[*target], self.tape[*up]);
                    let p = world(skeleton, pose, j).t;
                    let a = normalize3(sub(target, p));
                    let u = normalize3(sub(up, p));
                    let n = normalize3(cross3(a, u));
                    let b = normalize3(cross3(n, a));
                    let rows = [[a[0], a[1], a[2]], [b[0], b[1], b[2]], [n[0], n[1], n[2]]];
                    let parent = Self::parent_world(skeleton, pose, j);
                    pose[j].q = quat_mul(conj(parent.q), quat_of_rows(rows));
                }
                Op::Psc { .. } => self.psc(i, reset),
                Op::ReadDof { dof, value } => {
                    let v = self.dofs[*dof].and_then(|k| dofs.get(k)).copied().unwrap_or(0.0);
                    self.tape[*value] = splat(v);
                }
                _ => self.arith(i),
            }
        }
    }

    fn block(&self, s: usize) -> Sqt {
        Sqt { s: self.tape[s], q: self.tape[s + 1], t: self.tape[s + 2] }
    }

    fn set_block(&mut self, s: usize, v: &Sqt) {
        self.tape[s] = v.s;
        self.tape[s + 1] = v.q;
        self.tape[s + 2] = v.t;
    }

    fn joint(&mut self, skeleton: &Skeleton, pose: &mut [Sqt], j: usize, access: JointAccess) {
        use JointAccess::*;
        match access {
            LocalBlock { sqt } => self.set_block(sqt, &pose[j]),
            LocalBlockEuler { sqt, euler } => {
                self.set_block(sqt, &pose[j]);
                self.tape[euler] = euler_of_quat(pose[j].q);
            }
            LocalEuler { euler } => self.tape[euler] = euler_of_quat(pose[j].q),
            WorldTranslation { t } => self.tape[t] = world(skeleton, pose, j).t,
            WorldBlock { sqt } => {
                let w = world(skeleton, pose, j);
                self.set_block(sqt, &w);
            }
            SetLocalBlock { sqt } => pose[j] = self.block(sqt),
            SetLocalEuler { euler } => pose[j].q = quat_of_euler(self.tape[euler]),
            SetLocalTranslation { t } => pose[j].t = self.tape[t],
            SetLocalEulerTranslation { euler, t } => {
                pose[j].q = quat_of_euler(self.tape[euler]);
                pose[j].t = self.tape[t];
            }
            SetLocalScaleEulerTranslation { s, euler, t } => {
                pose[j].s = self.tape[s];
                pose[j].q = quat_of_euler(self.tape[euler]);
                pose[j].t = self.tape[t];
            }
            SetLocalScaleTranslation { s, t } => {
                pose[j].s = self.tape[s];
                pose[j].t = self.tape[t];
            }
            SetWorldTranslation { t } => {
                let mut w = world(skeleton, pose, j);
                w.t = self.tape[t];
                let parent = Self::parent_world(skeleton, pose, j);
                pose[j].t = local_from_world(&parent, &w).t;
            }
            VirtualOffset { t } => self.tape[t] = self.offsets[j],
            VirtualWorldTranslation { t } => self.tape[t] = self.virtual_world(skeleton, pose, j).t,
            VirtualWorldBlock { sqt } => {
                let w = self.virtual_world(skeleton, pose, j);
                self.set_block(sqt, &w);
            }
            SetVirtualOffset { t } => self.offsets[j] = self.tape[t],
            SetVirtualWorldTranslation { t } | SetVirtualWorldBlock { sqt: t } => {
                let target =
                    if matches!(access, SetVirtualWorldBlock { .. }) { self.tape[t + 2] } else { self.tape[t] };
                let parent = world(skeleton, pose, self.parents[j]);
                let w = Sqt { s: [1.0; 4], q: [0.0, 0.0, 0.0, 1.0], t: target };
                self.offsets[j] = local_from_world(&parent, &w).t;
            }
        }
    }

    fn arith(&mut self, i: usize) {
        let tape = &mut self.tape;
        match &self.program.ops[i].2 {
            Op::Math { kind, a, b, result } => {
                let (a, b) = (tape[*a], tape[*b]);
                tape[*result] = match kind {
                    MathKind::Add => add(a, b),
                    MathKind::Subtract => sub(a, b),
                    MathKind::Multiply => mul(a, b),
                    MathKind::Divide => std::array::from_fn(|k| a[k] / b[k]),
                };
            }
            Op::Magnitude { vector, result } => tape[*result] = splat(dot3(tape[*vector], tape[*vector]).sqrt()),
            Op::Split { vector, x, y, z } => {
                let v = tape[*vector];
                for (lane, s) in [x, y, z].into_iter().enumerate() {
                    if let Some(s) = s {
                        tape[*s] = splat(v[lane]);
                    }
                }
            }
            Op::Build { vector, x, y, z } => {
                let l = |s: usize, k: usize| tape[s][k];
                tape[*vector] = match (*x, *y, *z) {
                    (Some(x), None, None) => [l(x, 0), 0.0, 0.0, 0.0],
                    (None, Some(y), None) => [0.0, l(y, 0), 0.0, 0.0],
                    (None, None, Some(z)) => [0.0, 0.0, l(z, 0), 0.0],
                    (Some(x), Some(y), None) => [l(x, 0), l(y, 1), 0.0, l(x, 0)],
                    (Some(x), None, Some(z)) => [l(x, 0), 0.0, l(z, 2), l(x, 0)],
                    (None, Some(y), Some(z)) => [0.0, l(y, 1), l(z, 2), 0.0],
                    (Some(x), Some(y), Some(z)) => [l(x, 0), l(y, 1), l(z, 2), l(x, 0)],
                    (None, None, None) => [0.0; 4],
                };
            }
            Op::DrivenKey { driver, driven, domain, range, shape } => {
                tape[*driven] = driven_key(tape, *driver, *domain, *range, *shape);
            }
            Op::Twist { driver, reference, output, amount } => {
                let q = tape[driver + 1];
                let half = if q[3].abs() > 1.192_092_9e-7 { (q[0] / q[3]).atan() } else { 0.0 };
                let k = tape[*amount][0] * half;
                let (s, c) = k.sin_cos();
                let (sv, tv) = (tape[*reference], tape[reference + 2]);
                tape[*output] = sv;
                tape[output + 1] = [s, 0.0, 0.0, c];
                tape[output + 2] = tv;
            }
            Op::PointConstraint { targets, weights, result } => {
                let mut acc = [0.0; 4];
                for (t, w) in targets.iter().zip(weights) {
                    acc = add(acc, mul(tape[*t], tape[*w]));
                }
                tape[*result] = acc;
            }
            Op::LengthRatio { start, p1, p2, result } => {
                let len = |a: usize, b: usize| {
                    let d = sub(tape[a], tape[b]);
                    dot3(d, d).sqrt()
                };
                let (l1, l2, l3) = (len(*p1, *start), len(*start, *p2), len(*p1, *p2));
                tape[*result] = splat(l3 / (l1 + l2));
            }
            _ => {}
        }
    }

    fn psc(&mut self, i: usize, reset: bool) {
        let Op::Psc { root, reference, output, gravity, sum_scalar, scalars, delays, history } = &self.program.ops[i].2
        else {
            return;
        };
        let tape = &mut self.tape;
        let h = *history;
        tape[h] = tape[root + 2];
        let count = delays.last().map_or(0, |&d| d as usize + 1);
        if reset {
            for k in 1..=count {
                tape[h + k] = tape[h];
            }
        }
        let current = tape[h];
        let mut acc = [0.0f32; 4];
        let mut sign = 1.0f32;
        for (s, &d) in scalars.iter().zip(delays) {
            let d = d as usize;
            let mean = scale(add(add(tape[h + d], tape[h + d - 1]), tape[h + d + 1]), 1.0 / 3.0);
            acc = add(acc, mul(sub(mean, current), scale(tape[*s], sign)));
            sign = -sign;
        }
        let t = add(add(mul(acc, tape[*sum_scalar]), current), tape[*gravity]);
        let (sv, qv) = (tape[*reference], tape[reference + 1]);
        tape[*output] = sv;
        tape[output + 1] = qv;
        tape[output + 2] = t;
        for k in (1..=count).rev() {
            tape[h + k] = tape[h + k - 1];
        }
    }
}

fn driven_key(tape: &[V4], driver: usize, domain: [usize; 2], range: [usize; 2], shape: KeyShape) -> V4 {
    let (a, b, x) = (tape[domain[0]], tape[domain[1]], tape[driver]);
    let (ra, rb) = (tape[range[0]], tape[range[1]]);
    let clamped: V4 = std::array::from_fn(|k| x[k].max(a[k].min(b[k])).min(a[k].max(b[k])));
    match shape {
        KeyShape::Linear => std::array::from_fn(|k| ra[k] + (rb[k] - ra[k]) * (clamped[k] - a[k]) / (b[k] - a[k])),
        KeyShape::EaseInOut => std::array::from_fn(|k| {
            let t = (clamped[k] - a[k]) / (b[k] - a[k]);
            let e = if t > 0.5 { 1.0 - 2.0 * (1.0 - t) * (1.0 - t) } else { 2.0 * t * t };
            ra[k] + (rb[k] - ra[k]) * e
        }),
        KeyShape::Spline { domain_mid, range_mid } => {
            let (m, rm) = (tape[domain_mid], tape[range_mid]);
            let w0 = |v: V4| [v[0], v[1], v[2], 0.0];
            let (p0, p3) = (w0(ra), w0(rb));
            let first: [bool; 4] = std::array::from_fn(|k| m[k] > clamped[k]);
            let pick = |k: usize, f: V4, s: V4| if first[k] { f[k] } else { s[k] };
            let p1: V4 = std::array::from_fn(|k| pick(k, w0(ra), w0(rm)));
            let p2: V4 = std::array::from_fn(|k| pick(k, w0(rm), w0(rb)));
            let t: V4 = std::array::from_fn(|k| {
                if first[k] { (clamped[k] - a[k]) / (m[k] - a[k]) } else { (clamped[k] - m[k]) / (b[k] - m[k]) }
            });
            let (t1, t2, t3) = (t[2], t[1] * t[1], t[0] * t[1] * t[1]);
            std::array::from_fn(|k| {
                let ca = -0.5 * p0[k] + 1.5 * p1[k] - 1.5 * p2[k] + 0.5 * p3[k];
                let cb = p0[k] - 2.5 * p1[k] + 2.0 * p2[k] - 0.5 * p3[k];
                let cc = -0.5 * p0[k] + 0.5 * p2[k];
                ca * t3 + cb * t2 + cc * t1 + p1[k]
            })
        }
    }
}

pub fn load_program(stream: &ssx_formats::ant::Stream, name: &str) -> Result<Program> {
    let a = stream
        .assets_of_class(ssx_formats::rigamate::Rigamate::CLASS)
        .find(|a| a.object.name(&stream.schema) == Some(name))
        .with_context(|| format!("no RigamateAsset {name}"))?;
    let r = ssx_formats::rigamate::Rigamate::parse(&stream.schema, &a.object)?;
    Ok(r.program()?)
}

use std::collections::HashMap;
use std::path::Path;

use ssx_formats::ant::{Guid, Stream, Value};
use ssx_formats::crsf::SceneGraph;

use crate::character::Srt;

#[derive(Clone, Debug)]
pub struct RigamateRig {
    pub skeleton: Skeleton,
    node: Vec<Option<usize>>,
    base: Vec<Sqt>,
    pub instances: Vec<RigamateInstance>,
    pose: Vec<Sqt>,
    pub float_names: Vec<String>,
}

fn config_overrides(streams: &[&Stream], config: &str) -> Result<HashMap<String, Guid>> {
    let mut chain = Vec::new();
    let mut next = Some(config.to_string());
    while let Some(name) = next.take() {
        let (s, a) = streams
            .iter()
            .find_map(|s| {
                s.assets_of_class("AnimatableConfigAsset")
                    .find(|a| a.object.name(&s.schema) == Some(&name))
                    .map(|a| (*s, a))
            })
            .with_context(|| format!("no AnimatableConfigAsset {name}"))?;
        let guids = |f: &str| -> Vec<Guid> {
            a.object
                .get(&s.schema, f)
                .and_then(Value::as_array)
                .map(|v| v.iter().filter_map(Value::as_guid).collect())
                .unwrap_or_default()
        };
        chain.push((guids("OverrideGameStateTarget"), guids("OverrideGameStateData")));
        if let Some(p) = a.object.get(&s.schema, "Parent").and_then(Value::as_guid).filter(|g| !g.is_null()) {
            next = streams.iter().find_map(|s| {
                let i = *s.guid_index().get(&p)?;
                s.assets[i].object.name(&s.schema).map(str::to_string)
            });
        }
    }
    let name_of = |g: &Guid| {
        streams
            .iter()
            .find_map(|s| s.guid_index().get(g).and_then(|&i| s.assets[i].object.name(&s.schema).map(str::to_string)))
    };
    let mut out = HashMap::new();
    for (targets, data) in chain.iter().rev() {
        for (t, d) in targets.iter().zip(data) {
            if let Some(n) = name_of(t) {
                out.insert(n, *d);
            }
        }
    }
    Ok(out)
}

fn rigamate_of_value(streams: &[&Stream], value: Guid) -> Result<Program> {
    let find = |g: &Guid| streams.iter().find_map(|s| s.guid_index().get(g).map(|&i| (*s, &s.assets[i].object)));
    let (s, v) = find(&value).with_context(|| format!("RigamateValueAsset {value} not loaded"))?;
    let g = v.get(&s.schema, "RigamateAsset").and_then(Value::as_guid).context("RigamateValueAsset.RigamateAsset")?;
    let (s, r) = find(&g).with_context(|| format!("RigamateAsset {g} not loaded"))?;
    Ok(ssx_formats::rigamate::Rigamate::parse(&s.schema, r)?.program()?)
}

fn sqt_of(s: &Srt) -> Sqt {
    let [sx, sy, sz] = s.scale;
    let [tx, ty, tz] = s.translation;
    Sqt { s: [sx, sy, sz, 1.0], q: s.rotation, t: [tx, ty, tz, 0.0] }
}

fn srt_of(s: &Sqt) -> Srt {
    Srt { scale: [s.s[0], s.s[1], s.s[2]], rotation: s.q, translation: [s.t[0], s.t[1], s.t[2]] }
}

pub fn rig_defaults(stream: &Stream, rig: &str) -> Result<HashMap<String, Sqt>> {
    let schema = &stream.schema;
    let obj = &stream
        .assets_of_class("RigAsset")
        .find(|a| a.object.name(schema) == Some(rig))
        .with_context(|| format!("no RigAsset {rig}"))?
        .object;
    let mut out: HashMap<String, Sqt> = HashMap::new();
    for field in ["DefaultVector3Values", "DefaultVector4Values"] {
        for v in obj.get(schema, field).and_then(Value::as_array).unwrap_or_default() {
            let Some(s) = v.as_struct() else { continue };
            let name = s.own(schema, "DofName").and_then(Value::as_str).unwrap_or_default();
            let Some((rest, comp)) = name.rsplit_once('.') else { continue };
            let joint = rest.split_once('.').map_or(rest, |x| x.1);
            let value = match s.own(schema, "Value") {
                Some(Value::Vector3(v)) => [v[0], v[1], v[2], 0.0],
                Some(Value::Vector4(v) | Value::Quaternion(v)) => *v,
                _ => continue,
            };
            let e = out.entry(joint.to_string()).or_insert(Sqt::IDENTITY);
            match comp {
                "q" => e.q = value,
                "t" => e.t = [value[0], value[1], value[2], 0.0],
                "s" => e.s = [value[0], value[1], value[2], 1.0],
                _ => {}
            }
        }
    }
    Ok(out)
}

impl RigamateRig {
    pub fn load(big_root: &Path, rig: &SceneGraph, bind: &[Srt], character: &str) -> Result<(Self, Vec<String>)> {
        let dir = big_root.join(crate::rig_anim::ANIM_DIR);
        let read = |f: &str| -> Result<Stream> {
            let p = dir.join(f);
            let data = std::fs::read(&p).with_context(|| format!("reading {}", p.display()))?;
            Stream::parse(&data).with_context(|| format!("parsing {}", p.display()))
        };
        let all = crate::content::data::tuning::anim_all(big_root)?;
        let mut problems = Vec::new();
        let own = read(&format!("{character}.cba"));
        let streams: Vec<&Stream> = match &own {
            Ok(s) => vec![s, &all],
            Err(e) => {
                problems.push(format!("rigamate: {e:#} (only rigamate_boarder runs)"));
                vec![&all]
            }
        };
        let config = if own.is_ok() { format!("{character}_config") } else { "Snowboarder_config".to_string() };
        let overrides = config_overrides(&streams, &config)?;
        let skeleton = Skeleton::of_rig(&all, "genchar_rig")?;
        let defaults = rig_defaults(&all, "genchar_rig")?;
        let node: Vec<Option<usize>> = skeleton.joints.iter().map(|j| rig.find(&j.name)).collect();
        let base = skeleton
            .joints
            .iter()
            .zip(&node)
            .map(|(j, n)| {
                defaults.get(&j.name).copied().or_else(|| n.map(|n| sqt_of(&bind[n]))).unwrap_or(Sqt::IDENTITY)
            })
            .collect();
        let unmapped = node.iter().filter(|n| n.is_none()).count();
        if unmapped > 0 {
            problems.push(format!("rigamate: {unmapped} of {} skeleton joints have no rig node", node.len()));
        }
        let mut float_names: Vec<String> = Vec::new();
        let mut instances = Vec::new();
        for target in ["PTR_Rigamate_bodytype", "PTR_Rigamate_character"] {
            let Some(&value) = overrides.get(target) else {
                problems.push(format!("rigamate: {config} sets no {target}"));
                continue;
            };
            let program = rigamate_of_value(&streams, value)?;
            for (s, d) in &program.dofs {
                let n = format!("{s}.{d}");
                if !float_names.contains(&n) {
                    float_names.push(n);
                }
            }
            let names = float_names.clone();
            instances.push(RigamateInstance::new(program, &skeleton, |s, d| {
                names.iter().position(|n| *n == format!("{s}.{d}"))
            })?);
        }
        let pose = vec![Sqt::IDENTITY; skeleton.joints.len()];
        Ok((Self { skeleton, node, base, instances, pose, float_names }, problems))
    }

    pub fn summary(&self) -> String {
        let p: Vec<String> =
            self.instances.iter().map(|i| format!("{} ({} ops)", i.program.name, i.program.ops.len())).collect();
        p.join(" + ")
    }

    pub fn reset_history(&mut self) {
        self.instances.iter_mut().for_each(RigamateInstance::reset_history);
    }

    pub fn apply(&mut self, pose: &mut [Srt], floats: &[f32]) {
        for (j, n) in self.node.iter().enumerate() {
            self.pose[j] = n.map_or(self.base[j], |n| sqt_of(&pose[n]));
        }
        for inst in &mut self.instances {
            inst.run(&self.skeleton, &mut self.pose, floats);
        }
        for (j, n) in self.node.iter().enumerate() {
            if let Some(n) = n {
                pose[*n] = srt_of(&self.pose[j]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: V4, b: V4, eps: f32) -> bool {
        a.iter().zip(&b).all(|(x, y)| (x - y).abs() < eps)
    }

    #[test]
    fn euler_round_trip() {
        for e in [[0.3, -0.7, 1.1, 0.0], [-2.0, 0.4, -0.2, 0.0], [0.0, 0.0, 0.0, 0.0]] {
            let q = quat_of_euler(e);
            assert!(close(euler_of_quat(q), e, 1e-5), "{e:?} -> {q:?} -> {:?}", euler_of_quat(q));
        }
        let a = quat_of_euler([0.2, 0.5, -0.4, 0.0]);
        let b = ssx_formats::ant_anim::euler_xyz_to_quat([0.2, 0.5, -0.4]);
        assert!(close(a, b, 1e-6));
    }

    #[test]
    fn rows_and_quaternions_agree() {
        let q = normalize4([0.3, -0.5, 0.1, 0.8]);
        let r = quat_of_rows(rows_of(q));
        let d: f32 = (0..4).map(|i| q[i] * r[i]).sum();
        assert!((d.abs() - 1.0).abs() < 1e-5);
        let m = rows_of(q);
        let v = [0.2, -1.0, 0.7, 0.0];
        let rv: V4 = [0, 1, 2]
            .map(|j| (0..3).map(|i| v[i] * m[i][j]).sum::<f32>())
            .into_iter()
            .chain([0.0])
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
        assert!(close(rotate(q, v), rv, 1e-5));
    }

    #[test]
    fn world_and_local_are_inverse() {
        let parent = Sqt { s: [1.0; 4], q: normalize4([0.1, 0.2, -0.3, 0.9]), t: [1.0, 2.0, 3.0, 0.0] };
        let child = Sqt { s: [1.0; 4], q: normalize4([-0.4, 0.1, 0.2, 0.8]), t: [0.5, -0.2, 0.1, 0.0] };
        let w = compose(&parent, &child);
        let l = local_from_world(&parent, &w);
        assert!(close(l.t, child.t, 1e-5) && close(l.q, child.q, 1e-5));
    }

    #[test]
    fn driven_key_shapes() {
        let tape = vec![[0.25; 4], [0.0; 4], [1.0; 4], [10.0; 4], [20.0; 4], [0.5; 4], [12.0; 4]];
        assert!(close(driven_key(&tape, 0, [1, 2], [3, 4], KeyShape::Linear), [12.5; 4], 1e-5));
        assert!(close(driven_key(&tape, 0, [1, 2], [3, 4], KeyShape::EaseInOut), [11.25; 4], 1e-5));
        let at = |x: f32| {
            let mut t = tape.clone();
            t[0] = [x; 4];
            driven_key(&t, 0, [1, 2], [3, 4], KeyShape::Spline { domain_mid: 5, range_mid: 6 })[0]
        };
        assert!((at(0.0) - 10.0).abs() < 1e-5 && (at(0.5) - 12.0).abs() < 1e-5 && (at(1.0) - 20.0).abs() < 1e-5);
        assert!((at(-3.0) - 10.0).abs() < 1e-5 && (at(7.0) - 20.0).abs() < 1e-5);
    }
}
