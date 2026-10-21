use super::{Coverage, cstr_at, header, u32_at, zero_pad};
use crate::{Error, Result};

pub const MAGIC: &[u8; 4] = b"EVT0";

#[derive(Clone, Debug)]
pub struct EventSystem {
    pub version: (u16, u16),
    pub interface: u16,
    pub events: Vec<Event>,
    pub types: Vec<ParamType>,
}

#[derive(Clone, Debug)]
pub struct Event {
    pub id: u32,
    pub params: Vec<u32>,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct ParamType {
    pub kind: u32,
    pub id: u32,
    pub name: String,
    pub values: Vec<(u32, String)>,
}

impl EventSystem {
    pub fn parse(d: &[u8]) -> Result<Self> {
        let version = header(d, MAGIC)?;
        let (z, iface, ne, nt, oe, ot) =
            (u32_at(d, 0xC)?, u32_at(d, 0x10)?, u32_at(d, 0x14)?, u32_at(d, 0x18)?, u32_at(d, 0x1C)?, u32_at(d, 0x20)?);
        if z != 0 || iface & 0xFFFF != 0 || ne > 0x10000 || nt > 0x10000 {
            return Err(Error::invalid(format!("EVT0 header {z:#x} {iface:#x} {ne} {nt}")));
        }
        let (oe, ot) = (oe as usize, ot as usize);
        let mut cov = Coverage::default();
        cov.claim(0, 0x24);
        cov.claim(oe, oe + 4 * ne as usize);
        cov.claim(ot, ot + 4 * nt as usize);
        let mut events = Vec::with_capacity(ne as usize);
        for i in 0..ne as usize {
            let o = u32_at(d, oe + 4 * i)? as usize;
            let (size, n) = (u32_at(d, o)?, u32_at(d, o + 4)? as usize);
            let id = u32_at(d, o + 0x14)?;
            let zeros = (0..3).map(|k| u32_at(d, o + 8 + 4 * k)).chain((0..6).map(|k| u32_at(d, o + 0x18 + 4 * k)));
            if size as usize != 4 * n
                || id >> 16 != iface >> 16
                || zeros.collect::<Result<Vec<_>>>()?.iter().any(|&w| w != 0)
            {
                return Err(Error::invalid(format!("EVT0 event at {o:#x}: size {size} / {n}, id {id:#x}")));
            }
            let params = (0..n).map(|k| u32_at(d, o + 0x30 + 4 * k)).collect::<Result<Vec<_>>>()?;
            let (name, end) = cstr_at(d, o + 0x30 + 4 * n)?;
            zero_pad(d, end, "EVT0 event name")?;
            cov.claim_padded(o, end);
            events.push(Event { id, params, name });
        }
        let mut types = Vec::with_capacity(nt as usize);
        for i in 0..nt as usize {
            let o = u32_at(d, ot + 4 * i)? as usize;
            let (kind, id, n, z, vo) =
                (u32_at(d, o)?, u32_at(d, o + 4)?, u32_at(d, o + 8)?, u32_at(d, o + 12)?, u32_at(d, o + 16)?);
            if z != 0 || !matches!(kind, 1 | 3) || (n == 0) != (vo == 0) {
                return Err(Error::invalid(format!("EVT0 type at {o:#x}: kind {kind}, {n} values at {vo:#x}, {z:#x}")));
            }
            let (name, end) = cstr_at(d, o + 20)?;
            zero_pad(d, end, "EVT0 type name")?;
            cov.claim_padded(o, end);
            let vo = vo as usize;
            if n > 0 {
                cov.claim(vo, vo + 4 * n as usize);
            }
            let mut values = Vec::with_capacity(n as usize);
            for k in 0..n as usize {
                let v = u32_at(d, vo + 4 * k)? as usize;
                if u32_at(d, v)? != 0 {
                    return Err(Error::invalid(format!("EVT0 value at {v:#x}: first word not 0")));
                }
                let (vn, end) = cstr_at(d, v + 8)?;
                zero_pad(d, end, "EVT0 value name")?;
                cov.claim_padded(v, end);
                values.push((u32_at(d, v + 4)?, vn));
            }
            types.push(ParamType { kind, id, name, values });
        }
        cov.check(d.len(), "EVT0")?;
        Ok(Self { version, interface: (iface >> 16) as u16, events, types })
    }

    pub fn event(&self, name: &str) -> Option<&Event> {
        self.events.iter().find(|e| e.name == name)
    }

    pub fn param_type(&self, id: u32) -> Option<&ParamType> {
        self.types.iter().find(|t| t.id == id)
    }
}
