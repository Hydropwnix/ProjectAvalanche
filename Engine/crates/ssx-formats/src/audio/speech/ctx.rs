use super::{Coverage, cstr_at, header, i32_at, offset, u8_at, u16_at, u32_at, zero_pad};
use crate::{Error, Result};

pub const MAGIC: &[u8; 4] = b"CTX0";

#[derive(Clone, Debug)]
pub struct Contexts {
    pub version: (u16, u16),
    pub groups: Vec<u32>,
    pub buckets: u32,
    pub contexts: Vec<Context>,
}

#[derive(Clone, Debug)]
pub struct Context {
    pub id: u32,
    pub priority: u32,
    pub probability: u8,
    pub flags: u8,
    pub unk_0d: u8,
    pub interrupt_group: u16,
    pub unk_10: u16,
    pub unk_12: u16,
    pub trigger: Option<Trigger>,
    pub action: Action,
    pub action2: Option<Action>,
    pub param_conditions: Vec<ParamCondition>,
    pub conditions: Vec<Condition>,
    pub groups: Vec<usize>,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct Trigger {
    pub event: u32,
    pub params: usize,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    TriggerParam(usize),
    Script { script: u32, args: Vec<Value> },
}

#[derive(Clone, Debug)]
pub struct Action {
    pub event: u32,
    pub params: Vec<Value>,
    pub name: String,
}

#[derive(Clone, Copy, Debug)]
pub struct ParamCondition {
    pub source: u32,
    pub index: u32,
    pub op: u32,
    pub value: i32,
}

#[derive(Clone, Debug)]
pub struct Condition {
    pub script: u32,
    pub name: String,
    pub args: Vec<Value>,
}

impl Contexts {
    pub fn parse(d: &[u8]) -> Result<Self> {
        let version = header(d, MAGIC)?;
        let w = (0..8).map(|k| u32_at(d, 0xC + 4 * k)).collect::<Result<Vec<_>>>()?;
        let (strings, rel, n, g, b) = (w[0] as usize, w[1], w[2] as usize, w[3] as usize, w[4]);
        if rel != 0 || w[5..] != [0, 0, 0] || strings > d.len() || n > 0x10000 || g > 0x1000 || b == 0 {
            return Err(Error::invalid(format!("CTX0 header {w:x?}")));
        }
        let mut cov = Coverage::default();
        cov.claim(0, 0x2C);
        let gt = 0x2C;
        let groups = (0..g).map(|i| u32_at(d, gt + 8 * i)).collect::<Result<Vec<_>>>()?;
        if (0..g).map(|i| u32_at(d, gt + 8 * i + 4)).collect::<Result<Vec<_>>>()?.iter().any(|&v| v != 0) {
            return Err(Error::invalid("CTX0 group runtime words"));
        }
        cov.claim(gt, gt + 8 * g);
        let ct = gt + 8 * g;
        cov.claim(ct, ct + 4 * n);
        let ht = ct + 4 * n;
        cov.claim(ht, ht + 8 * b as usize);
        let mut by_id = std::collections::HashMap::new();
        for k in 0..b as usize {
            let (m, off) = (u32_at(d, ht + 8 * k)? as usize, i32_at(d, ht + 8 * k + 4)?);
            let Some(o) = offset(d, off, "CTX0 bucket")? else { continue };
            cov.claim(o, o + 8 * m);
            for j in 0..m {
                let (id, co) = (u32_at(d, o + 8 * j)?, u32_at(d, o + 8 * j + 4)? as usize);
                if id % b != k as u32 {
                    return Err(Error::invalid(format!("CTX0 context id {id} in bucket {k}")));
                }
                by_id.insert(id, co);
            }
        }
        let cx = Cx { d, strings, gt, g };
        let mut contexts = Vec::with_capacity(n);
        for i in 0..n {
            let co = u32_at(d, ct + 4 * i)? as usize;
            let c = cx.context(co, &mut cov)?;
            if by_id.get(&c.id) != Some(&co) {
                return Err(Error::invalid(format!("CTX0 context {} not in its hash bucket", c.id)));
            }
            contexts.push(c);
        }
        if by_id.len() != n {
            return Err(Error::invalid("CTX0 hash table and context table differ"));
        }
        cov.claim(strings, d.len());
        cov.check(d.len(), "CTX0")?;
        Ok(Self { version, groups, buckets: b, contexts })
    }

    pub fn by_name(&self, name: &str) -> Option<&Context> {
        self.contexts.iter().find(|c| c.name == name)
    }
}

struct Cx<'a> {
    d: &'a [u8],
    strings: usize,
    gt: usize,
    g: usize,
}

impl Cx<'_> {
    fn context(&self, co: usize, cov: &mut Coverage) -> Result<Context> {
        let d = self.d;
        cov.claim(co, co + 0x34);
        let (id, priority) = (u32_at(d, co)?, u32_at(d, co + 4)?);
        let (nc, ng, np) = (u8_at(d, co + 8)? as usize, u8_at(d, co + 9)? as usize, u8_at(d, co + 10)? as usize);
        let (probability, flags, unk_0d) = (u8_at(d, co + 11)?, u8_at(d, co + 12)?, u8_at(d, co + 13)?);
        let (interrupt_group, unk_10, unk_12) = (u16_at(d, co + 14)?, u16_at(d, co + 16)?, u16_at(d, co + 18)?);
        let o = |k: usize| i32_at(d, co + 0x18 + 4 * k).and_then(|v| offset(d, v, "CTX0 context field"));
        if u32_at(d, co + 0x14)? != 0 || flags & !1 != 0 {
            return Err(Error::invalid(format!("CTX0 context {id}: runtime word / flags {flags:#x}")));
        }
        let (name, end) = cstr_at(d, u32_at(d, co + 0x30)? as usize)?;
        zero_pad(d, end, "CTX0 context name")?;
        cov.claim_padded(u32_at(d, co + 0x30)? as usize, end);
        let trig = o(0)?;
        let slots = match trig {
            Some(t) => {
                let (z, event, n, z2, z3) = (
                    u32_at(d, t)?,
                    u32_at(d, t + 4)?,
                    u32_at(d, t + 8)? as usize,
                    u32_at(d, t + 12)?,
                    u32_at(d, t + 16)?,
                );
                let ok = (0..n)
                    .map(|k| u32_at(d, t + 20 + 4 * k))
                    .collect::<Result<Vec<_>>>()?
                    .iter()
                    .enumerate()
                    .all(|(k, &v)| v == k as u32 + 1);
                if (z, z2, z3) != (0, 0, 0) || !ok {
                    return Err(Error::invalid(format!("CTX0 context {id}: trigger record at {t:#x}")));
                }
                let (tn, end) = cstr_at(d, t + 20 + 4 * n)?;
                zero_pad(d, end, "CTX0 trigger name")?;
                cov.claim_padded(t, end);
                Some((t + 20, n, Trigger { event, params: n, name: tn }))
            }
            None => None,
        };
        let slot_range = slots.as_ref().map_or((0, 0), |s| (s.0, s.1));
        let action = self.action(
            o(1)?.ok_or_else(|| Error::invalid(format!("CTX0 context {id} without action")))?,
            slot_range,
            cov,
        )?;
        let action2 = o(2)?.map(|a| self.action(a, slot_range, cov)).transpose()?;
        let mut param_conditions = Vec::with_capacity(np);
        if let Some(p) = o(3)? {
            cov.claim(p, p + 16 * np);
            for k in 0..np {
                let q = p + 16 * k;
                param_conditions.push(ParamCondition {
                    source: u32_at(d, q)?,
                    index: u32_at(d, q + 4)?,
                    op: u32_at(d, q + 8)?,
                    value: i32_at(d, q + 12)?,
                });
            }
        }
        let mut conditions = Vec::with_capacity(nc);
        if let Some(c) = o(4)? {
            cov.claim(c, c + 4 * nc);
            for k in 0..nc {
                let r = u32_at(d, c + 4 * k)? as usize;
                let (na, ap, script, nm) =
                    (u32_at(d, r)? as usize, i32_at(d, r + 4)?, u32_at(d, r + 8)?, u32_at(d, r + 12)? as usize);
                if (na == 0) != (ap == -1) || (na > 0 && ap as usize != r + 16) {
                    return Err(Error::invalid(format!("CTX0 condition at {r:#x}: {na} args at {ap:#x}")));
                }
                cov.claim(r, r + 16 + 4 * na);
                let args = (0..na)
                    .map(|j| self.value_ptr(u32_at(d, r + 16 + 4 * j)? as usize, slot_range))
                    .collect::<Result<Vec<_>>>()?;
                let name = cstr_at(d, self.strings + nm)?.0;
                conditions.push(Condition { script, name, args });
            }
        }
        let mut groups = Vec::with_capacity(ng);
        if let Some(gp) = o(5)? {
            cov.claim(gp, gp + 4 * ng);
            for k in 0..ng {
                let at = u32_at(d, gp + 4 * k)? as usize;
                let gi = at.checked_sub(self.gt).filter(|x| x.is_multiple_of(8) && x / 8 < self.g);
                groups.push(
                    gi.map(|x| x / 8)
                        .ok_or_else(|| Error::invalid(format!("CTX0 context {id}: group pointer {at:#x}")))?,
                );
            }
        }
        let trigger = slots.map(|s| s.2);
        Ok(Context {
            id,
            priority,
            probability,
            flags,
            unk_0d,
            interrupt_group,
            unk_10,
            unk_12,
            trigger,
            action,
            action2,
            param_conditions,
            conditions,
            groups,
            name,
        })
    }

    fn value_ptr(&self, at: usize, slots: (usize, usize)) -> Result<Value> {
        if at >= slots.0 && at < slots.0 + 4 * slots.1 && (at - slots.0).is_multiple_of(4) {
            Ok(Value::TriggerParam((at - slots.0) / 4))
        } else {
            Err(Error::invalid(format!("CTX0 value pointer {at:#x} outside the trigger's slots")))
        }
    }

    fn action(&self, a: usize, slots: (usize, usize), cov: &mut Coverage) -> Result<Action> {
        let d = self.d;
        let w = (0..7).map(|k| u32_at(d, a + 4 * k)).collect::<Result<Vec<_>>>()?;
        let (event, n, nm) = (w[1], w[4] as usize, w[6] as usize);
        if w[0] != 0 || w[2] != 0 || w[3] != 0 || w[5] != 0 {
            return Err(Error::invalid(format!("CTX0 action at {a:#x}: runtime words {w:x?}")));
        }
        cov.claim(a, a + 28 + 8 * n);
        let (name, end) = cstr_at(d, nm)?;
        zero_pad(d, end, "CTX0 action name")?;
        cov.claim_padded(nm, end);
        let params = (0..n)
            .map(|k| self.item(u32_at(d, a + 28 + 8 * k)?, u32_at(d, a + 32 + 8 * k)? as usize, slots, cov))
            .collect::<Result<Vec<_>>>()?;
        Ok(Action { event, params, name })
    }

    fn item(&self, kind: u32, at: usize, slots: (usize, usize), cov: &mut Coverage) -> Result<Value> {
        let d = self.d;
        match kind {
            0 => self.value_ptr(at, slots),
            1 => {
                let (z, script, n, ap) =
                    (u32_at(d, at)?, u32_at(d, at + 4)?, u32_at(d, at + 8)? as usize, i32_at(d, at + 12)?);
                if z != 0 || (n == 0) != (ap == -1) {
                    return Err(Error::invalid(format!("CTX0 script call at {at:#x}")));
                }
                cov.claim(at, at + 16);
                let mut args = Vec::with_capacity(n);
                if let Some(p) = offset(d, ap, "CTX0 call args")? {
                    cov.claim(p, p + 8 * n);
                    for k in 0..n {
                        args.push(self.item(u32_at(d, p + 8 * k)?, u32_at(d, p + 8 * k + 4)? as usize, slots, cov)?);
                    }
                }
                Ok(Value::Script { script, args })
            }
            k => Err(Error::invalid(format!("CTX0 item type {k} at {at:#x}"))),
        }
    }
}
