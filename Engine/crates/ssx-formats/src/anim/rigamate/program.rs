use super::*;

pub type Slot = usize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MathKind {
    Add,
    Subtract,
    Multiply,
    Divide,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyShape {
    Linear,
    EaseInOut,
    Spline { domain_mid: Slot, range_mid: Slot },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JointAccess {
    LocalBlock { sqt: Slot },
    LocalBlockEuler { sqt: Slot, euler: Slot },
    LocalEuler { euler: Slot },
    WorldTranslation { t: Slot },
    WorldBlock { sqt: Slot },
    SetLocalBlock { sqt: Slot },
    SetLocalEuler { euler: Slot },
    SetLocalTranslation { t: Slot },
    SetLocalEulerTranslation { euler: Slot, t: Slot },
    SetLocalScaleEulerTranslation { s: Slot, euler: Slot, t: Slot },
    SetLocalScaleTranslation { s: Slot, t: Slot },
    SetWorldTranslation { t: Slot },
    VirtualOffset { t: Slot },
    VirtualWorldTranslation { t: Slot },
    VirtualWorldBlock { sqt: Slot },
    SetVirtualOffset { t: Slot },
    SetVirtualWorldTranslation { t: Slot },
    SetVirtualWorldBlock { sqt: Slot },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Math {
        kind: MathKind,
        a: Slot,
        b: Slot,
        result: Slot,
    },
    Magnitude {
        vector: Slot,
        result: Slot,
    },
    Split {
        vector: Slot,
        x: Option<Slot>,
        y: Option<Slot>,
        z: Option<Slot>,
    },
    Build {
        vector: Slot,
        x: Option<Slot>,
        y: Option<Slot>,
        z: Option<Slot>,
    },
    DrivenKey {
        driver: Slot,
        driven: Slot,
        domain: [Slot; 2],
        range: [Slot; 2],
        shape: KeyShape,
    },
    Joint {
        joint: u32,
        access: JointAccess,
    },
    Aim {
        joint: u32,
        target: Slot,
        up: Slot,
    },
    Twist {
        driver: Slot,
        reference: Slot,
        output: Slot,
        amount: Slot,
    },
    PointConstraint {
        targets: [Slot; 3],
        weights: [Slot; 3],
        result: Slot,
    },
    LengthRatio {
        start: Slot,
        p1: Slot,
        p2: Slot,
        result: Slot,
    },
    Psc {
        root: Slot,
        reference: Slot,
        output: Slot,
        gravity: Slot,
        sum_scalar: Slot,
        scalars: Vec<Slot>,
        delays: Vec<u32>,
        history: Slot,
    },
    ReadDof {
        dof: usize,
        value: Slot,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Program {
    pub name: String,
    pub tape: Vec<[f32; 4]>,
    pub ops: Vec<(u32, u16, Op)>,
    pub virtual_joints: Vec<(u32, [f32; 4])>,
    pub dofs: Vec<(String, String)>,
}

pub(super) const JOINT_BASE: [u16; 16] =
    [0, 0x21, 0x1A, 0x22, 0x1E, 0x26, 0x1B, 0x23, 0x20, 0x28, 0x1C, 0x24, 0x1F, 0x27, 0x1D, 0x25];
pub(super) const SPLIT_BASE: [u16; 8] = [0, 6, 7, 9, 8, 0xA, 0xB, 0xC];

impl Rigamate {
    pub fn program(&self) -> Result<Program> {
        let n = self.entry_count as usize;
        let err = |msg: String| Error::invalid(format!("{} ({}): {msg}", Self::CLASS, self.name));
        let slot = |pos: u32, rel: i16| -> Result<Slot> {
            let s = i64::from(pos) + i64::from(rel);
            usize::try_from(s)
                .ok()
                .filter(|&s| s < n)
                .ok_or_else(|| err(format!("operand {pos}{rel:+} outside the tape")))
        };
        let mut tape = vec![[0.0f32; 4]; n];
        for &(p, v) in &self.vectors {
            tape[slot(p, 0)?] = [v[0], v[1], v[2], 0.0];
        }
        for &(p, x) in &self.scalars {
            tape[slot(p, 0)?] = [x; 4];
        }
        let mut ops: std::collections::BTreeMap<u32, (u16, Op)> = std::collections::BTreeMap::new();
        let mut add = |pos: u32, code: u16, op: Op| -> Result<()> {
            match ops.insert(pos, (code, op)) {
                None => Ok(()),
                Some(_) => Err(err(format!("two operations at entry {pos}"))),
            }
        };
        for m in &self.binary_math {
            let kind = match m.kind {
                0 => MathKind::Add,
                1 => MathKind::Subtract,
                2 => MathKind::Multiply,
                3 => MathKind::Divide,
                k => return Err(err(format!("BinaryMath type {k}"))),
            };
            let p = m.position;
            let op = Op::Math { kind, a: slot(p, m.a)?, b: slot(p, m.b)?, result: slot(p, m.result)? };
            add(p, m.kind as u16 + 1, op)?;
        }
        for m in &self.magnitude {
            add(
                m.position,
                5,
                Op::Magnitude { vector: slot(m.position, m.vector)?, result: slot(m.position, m.result)? },
            )?;
        }
        for v in &self.vector_build_split {
            let c = usize::try_from(v.connections).ok().filter(|&c| (1..8).contains(&c));
            let c = c.ok_or_else(|| err(format!("VectorBuildSplit connections {}", v.connections)))?;
            let p = v.position;
            let pick = |bit: usize, rel: i16| -> Result<Option<Slot>> {
                if c & bit != 0 { slot(p, rel).map(Some) } else { Ok(None) }
            };
            let (vector, x, y, z) = (slot(p, v.vector)?, pick(1, v.x)?, pick(2, v.y)?, pick(4, v.z)?);
            let (code, op) = if v.build {
                (SPLIT_BASE[c] + 7, Op::Build { vector, x, y, z })
            } else {
                (SPLIT_BASE[c], Op::Split { vector, x, y, z })
            };
            add(p, code, op)?;
        }
        for k in &self.set_driven_key {
            let p = k.position;
            let (driver, driven) = (slot(p, k.driver)?, slot(p, k.driven)?);
            let (domain, range) =
                ([slot(p, k.domain_a)?, slot(p, k.domain_b)?], [slot(p, k.range_a)?, slot(p, k.range_b)?]);
            let (code, shape) = if k.domain_mid != 0 && k.range_mid != 0 {
                (0x19, KeyShape::Spline { domain_mid: slot(p, k.domain_mid)?, range_mid: slot(p, k.range_mid)? })
            } else {
                match k.power {
                    1 => (0x14, KeyShape::Linear),
                    2 => (0x15, KeyShape::EaseInOut),
                    x => return Err(err(format!("SetDrivenKey power {x} (kernel not decoded)"))),
                }
            };
            add(p, code, Op::DrivenKey { driver, driven, domain, range, shape })?;
        }
        for j in &self.joint_io {
            let (code, access) = self.joint_access(j, &slot).map_err(|e| err(e.to_string()))?;
            add(j.position, code, Op::Joint { joint: j.joint_index, access })?;
        }
        self.more_ops(&slot, &mut tape, &mut add, &err)?;
        drop(add);
        self.write_headers(&mut tape, &ops);
        self.finish(tape, ops, &err)
    }

    fn write_headers(&self, tape: &mut [[f32; 4]], ops: &std::collections::BTreeMap<u32, (u16, Op)>) {
        type Header = ([u16; 8], Option<[u16; 8]>, u16);
        let mut words: std::collections::BTreeMap<u32, Header> = Default::default();
        let w = |x: i16| x as u16;
        let hi = |x: u32| (x >> 16) as u16;
        let lo = |x: u32| x as u16;
        for m in &self.binary_math {
            words.insert(m.position, ([0, 0, w(m.a), w(m.b), w(m.result), 0, 0, 0], None, 1));
        }
        for m in &self.magnitude {
            words.insert(m.position, ([0, 0, w(m.vector), w(m.result), 0, 0, 0, 0], None, 1));
        }
        for v in &self.vector_build_split {
            words.insert(v.position, ([0, 0, w(v.vector), w(v.x), w(v.y), w(v.z), 0, 0], None, 1));
        }
        for k in &self.set_driven_key {
            let head = [0, 0, w(k.driver), w(k.driven), w(k.domain_a), w(k.domain_b), w(k.range_a), w(k.range_b)];
            let second =
                (k.domain_mid != 0 && k.range_mid != 0).then(|| [0, w(k.domain_mid), w(k.range_mid), 0, 0, 0, 0, 0]);
            words.insert(k.position, (head, second, 2));
        }
        for j in &self.joint_io {
            let head =
                [0, 0, hi(j.joint_index), lo(j.joint_index), w(j.sqt), w(j.scale), w(j.quaternion), w(j.translation)];
            words.insert(j.position, (head, None, 1));
        }
        for a in &self.aim {
            let head = [
                0,
                0,
                hi(a.joint_index),
                lo(a.joint_index),
                w(a.target),
                w(a.up_joint),
                w(a.target_offset),
                w(a.twist_amount),
            ];
            words.insert(a.position, (head, None, 1));
        }
        for t in &self.twist {
            words.insert(t.position, ([0, 0, w(t.driver), w(t.reference), w(t.reference), w(t.amount), 0, 0], None, 1));
        }
        for c in &self.point_constraint {
            let [a, b, d] = c.targets;
            words.insert(c.position, ([0, 0, w(a), w(b), w(d), w(c.result), 0, 0], None, 4));
        }
        for l in &self.length_ratio {
            let head = [
                0,
                0,
                w(l.start_position),
                w(l.position1),
                w(l.position2),
                w(l.start_offset),
                w(l.offset1),
                w(l.offset2),
            ];
            words.insert(l.position, (head, Some([w(l.result), 0, 0, 0, 0, 0, 0, 0]), 2));
        }
        for c in &self.psc {
            let nb = c.num_bounces as usize;
            let offset = ((nb + nb / 4 + 7) & !3) as u16;
            let head = [0, 0, w(c.root), w(c.reference), w(c.reference), offset, hi(c.num_bounces), lo(c.num_bounces)];
            words.insert(c.position, (head, None, 0));
        }
        for d in &self.dof_io {
            words.insert(d.position, ([0, 0, 0, 0, hi(d.index), lo(d.index), w(d.value), 0], None, 1));
        }
        let positions: Vec<u32> = ops.keys().copied().collect();
        for (i, &p) in positions.iter().enumerate() {
            let Some((mut head, second, len)) = words.get(&p).copied() else { continue };
            head[0] = ops[&p].0;
            head[1] = positions.get(i + 1).map_or(len, |&n| (n - p) as u16);
            let bits = |h: [u16; 8]| -> [f32; 4] {
                std::array::from_fn(|k| f32::from_bits((u32::from(h[2 * k]) << 16) | u32::from(h[2 * k + 1])))
            };
            tape[p as usize] = bits(head);
            if let Some(s) = second
                && let Some(e) = tape.get_mut(p as usize + 1)
            {
                *e = bits(s);
            }
        }
    }

    fn joint_access(&self, j: &JointIo, slot: &impl Fn(u32, i16) -> Result<Slot>) -> Result<(u16, JointAccess)> {
        let base = JOINT_BASE.get(j.connectivity as usize).copied().filter(|&b| b != 0);
        let base = base.ok_or_else(|| Error::invalid(format!("JointIO connectivity {}", j.connectivity)))?;
        let code = base
            + if j.local_space { 0 } else { 0xF }
            + if j.reader { 0 } else { 0x1E }
            + u16::try_from(j.virtual_joint).ok().filter(|&v| v <= 1).map(|v| v * 0x3C).unwrap_or(u16::MAX / 2);
        let p = j.position;
        let s = |rel: i16| slot(p, rel);
        use JointAccess::*;
        let access = match code {
            0x21 | 0x24 => LocalBlock { sqt: s(j.sqt)? },
            0x23 | 0x25 | 0x26 | 0x27 => LocalBlockEuler { sqt: s(j.sqt)?, euler: s(j.quaternion)? },
            0x1E => LocalEuler { euler: s(j.quaternion)? },
            0x2F => WorldTranslation { t: s(j.translation)? },
            0x30 => WorldBlock { sqt: s(j.sqt)? },
            0x3F => SetLocalBlock { sqt: s(j.sqt)? },
            0x3C => SetLocalEuler { euler: s(j.quaternion)? },
            0x3E => SetLocalTranslation { t: s(j.translation)? },
            0x3D => SetLocalEulerTranslation { euler: s(j.quaternion)?, t: s(j.translation)? },
            0x3B => SetLocalScaleEulerTranslation { s: s(j.scale)?, euler: s(j.quaternion)?, t: s(j.translation)? },
            0x3A => SetLocalScaleTranslation { s: s(j.scale)?, t: s(j.translation)? },
            0x4D => SetWorldTranslation { t: s(j.translation)? },
            0x5C => VirtualOffset { t: s(j.translation)? },
            0x6B => VirtualWorldTranslation { t: s(j.translation)? },
            0x6F => VirtualWorldBlock { sqt: s(j.sqt)? },
            0x7A => SetVirtualOffset { t: s(j.translation)? },
            0x89 => SetVirtualWorldTranslation { t: s(j.translation)? },
            0x8A => SetVirtualWorldBlock { sqt: s(j.sqt)? },
            c => return Err(Error::invalid(format!("JointIO opcode {c:#x} at entry {p}: kernel not decoded"))),
        };
        Ok((code, access))
    }

    fn more_ops(
        &self,
        slot: &impl Fn(u32, i16) -> Result<Slot>,
        tape: &mut [[f32; 4]],
        add: &mut impl FnMut(u32, u16, Op) -> Result<()>,
        err: &impl Fn(String) -> Error,
    ) -> Result<()> {
        for a in &self.aim {
            if a.twist_amount != 0 {
                return Err(err(format!(
                    "Aim at {} with TwistAmount {} (the loader rejects it)",
                    a.position, a.twist_amount
                )));
            }
            let p = a.position;
            add(p, 0xCE, Op::Aim { joint: a.joint_index, target: slot(p, a.target)?, up: slot(p, a.up_joint)? })?;
        }
        for t in &self.twist {
            let p = t.position;
            let (driver, reference, amount) = (slot(p, t.driver)?, slot(p, t.reference)?, slot(p, t.amount)?);
            let (_, _) = (slot(p, t.reference + 2)?, slot(p, t.driver + 2)?);
            let op = Op::Twist { driver, reference, output: reference, amount };
            add(p, if t.gimbal { 0xD1 } else { 0xD0 }, op)?;
        }
        for c in &self.point_constraint {
            let p = c.position;
            let weights = [slot(p, 1)?, slot(p, 2)?, slot(p, 3)?];
            for (w, &x) in weights.iter().zip(&c.weights) {
                tape[*w] = [x; 4];
            }
            let targets = [slot(p, c.targets[0])?, slot(p, c.targets[1])?, slot(p, c.targets[2])?];
            add(p, 0xD2, Op::PointConstraint { targets, weights, result: slot(p, c.result)? })?;
        }
        for l in &self.length_ratio {
            let p = l.position;
            let op = Op::LengthRatio {
                start: slot(p, l.start_position)?,
                p1: slot(p, l.position1)?,
                p2: slot(p, l.position2)?,
                result: slot(p, l.result)?,
            };
            add(p, 0xD3, op)?;
        }
        for c in &self.psc {
            let p = c.position;
            let nb = c.num_bounces as usize;
            if c.bounce_scalars.len() != nb || c.bounce_delays.len() != nb || nb == 0 {
                return Err(err(format!(
                    "PSC at {p}: {nb} bounces, {} scalars, {} delays",
                    c.bounce_scalars.len(),
                    c.bounce_delays.len()
                )));
            }
            let entry =
                |k: usize| i16::try_from(k).map_err(|_| err(format!("PSC at {p}: too large"))).and_then(|k| slot(p, k));
            let (gravity, sum_scalar) = (entry(1)?, entry(2)?);
            tape[gravity] = [c.gravity[0], c.gravity[1], c.gravity[2], 0.0];
            tape[sum_scalar] = [c.sum_scalar; 4];
            let mut scalars = Vec::with_capacity(nb);
            for (i, &x) in c.bounce_scalars.iter().enumerate() {
                let s = entry(3 + i)?;
                tape[s] = [x; 4];
                scalars.push(s);
            }
            let offset = (nb + nb / 4 + 7) & !3;
            let deepest = c.bounce_delays.iter().copied().max().unwrap_or(0) as usize;
            if c.bounce_delays.contains(&0) {
                return Err(err(format!("PSC at {p}: a bounce delay of 0 reads before its history")));
            }
            let history = entry(offset)?;
            entry(offset + deepest + 1)?;
            let (root, reference) = (slot(p, c.root)?, slot(p, c.reference)?);
            let op = Op::Psc {
                root,
                reference,
                output: reference,
                gravity,
                sum_scalar,
                scalars,
                delays: c.bounce_delays.clone(),
                history,
            };
            add(p, 0xDB, op)?;
        }
        for d in &self.dof_io {
            if !d.reader || d.index as usize >= self.dof_values.len() {
                return Err(err(format!("DofIO at {}: reader {} index {}", d.position, d.reader, d.index)));
            }
            add(d.position, 0xE2, Op::ReadDof { dof: d.index as usize, value: slot(d.position, d.value)? })?;
        }
        if let Some(c) = self.conditional.first() {
            return Err(err(format!("Conditional at {} (loader not decoded)", c.position)));
        }
        Ok(())
    }

    fn finish(
        &self,
        tape: Vec<[f32; 4]>,
        mut ops: std::collections::BTreeMap<u32, (u16, Op)>,
        err: &impl Fn(String) -> Error,
    ) -> Result<Program> {
        let constants: std::collections::HashSet<u32> =
            self.vectors.iter().map(|v| v.0).chain(self.scalars.iter().map(|s| s.0)).collect();
        let mut out = Vec::with_capacity(self.operation_positions.len());
        for &p in &self.operation_positions {
            if constants.contains(&p) {
                return Err(err(format!("a constant on the header entry {p}")));
            }
            let (code, op) =
                ops.remove(&p).ok_or_else(|| err(format!("OperationPositions names {p}: no operation")))?;
            out.push((p, code, op));
        }
        if let Some((p, _)) = ops.into_iter().next() {
            return Err(err(format!("operation at {p} missing from OperationPositions")));
        }
        if !out.windows(2).all(|w| w[0].0 < w[1].0) {
            return Err(err("OperationPositions not ascending".into()));
        }
        let mut virtual_joints = vec![None; self.virtual_joints.len()];
        for v in &self.virtual_joints {
            let i = usize::try_from(v.index).ok().filter(|&i| i < virtual_joints.len());
            let i = i.ok_or_else(|| err(format!("virtual joint index {}", v.index)))?;
            let parent =
                u32::try_from(v.parent_index).map_err(|_| err(format!("virtual joint parent {}", v.parent_index)))?;
            if virtual_joints[i].replace((parent, [v.offset[0], v.offset[1], v.offset[2], 0.0])).is_some() {
                return Err(err(format!("virtual joint {i} twice")));
            }
        }
        let virtual_joints: Vec<(u32, [f32; 4])> = virtual_joints
            .into_iter()
            .collect::<Option<_>>()
            .ok_or_else(|| err("virtual joint indices not dense".into()))?;
        for (_, _, op) in &out {
            if let Op::Joint { joint, access } = op
                && is_virtual(access)
                && *joint as usize >= virtual_joints.len()
            {
                return Err(err(format!("virtual joint {joint} of {}", virtual_joints.len())));
            }
        }
        let name = |i: u32| self.strings.get(i as usize).cloned().ok_or_else(|| err(format!("string {i}")));
        let dofs = self
            .dof_values
            .iter()
            .map(|d| Ok((name(d.dof_set_index)?, name(d.dof_index)?)))
            .collect::<Result<Vec<_>>>()?;
        Ok(Program { name: self.name.clone(), tape, ops: out, virtual_joints, dofs })
    }
}

pub fn is_virtual(access: &JointAccess) -> bool {
    use JointAccess::*;
    matches!(
        access,
        VirtualOffset { .. }
            | VirtualWorldTranslation { .. }
            | VirtualWorldBlock { .. }
            | SetVirtualOffset { .. }
            | SetVirtualWorldTranslation { .. }
            | SetVirtualWorldBlock { .. }
    )
}
