use super::{Coverage, cstr_at, header, u32_at, zero_pad};
use crate::{Error, Result};

pub const MAGIC: &[u8; 4] = b"REP0";

#[derive(Clone, Debug)]
pub struct RepetitionPools {
    pub version: (u16, u16),
    pub pools: Vec<Pool>,
}

#[derive(Clone, Debug)]
pub struct Pool {
    pub kind: [u8; 4],
    pub id: u32,
    pub size: u32,
    pub name: String,
}

impl RepetitionPools {
    pub fn parse(d: &[u8]) -> Result<Self> {
        let version = header(d, MAGIC)?;
        let (rel, n, ptr) = (u32_at(d, 0xC)?, u32_at(d, 0x10)? as usize, u32_at(d, 0x14)?);
        if rel != 0 || ptr != 0 || n > 0x1000 {
            return Err(Error::invalid(format!("REP0 header {rel:#x} {n} {ptr:#x}")));
        }
        let mut cov = Coverage::default();
        cov.claim(0, 0x18 + 8 * n);
        let mut pools = Vec::with_capacity(n);
        for i in 0..n {
            let (id, o) = (u32_at(d, 0x18 + 8 * i)?, u32_at(d, 0x1C + 8 * i)? as usize);
            let kind: [u8; 4] =
                d.get(o..o + 4).and_then(|b| b.try_into().ok()).ok_or(Error::invalid("REP0 pool offset"))?;
            if &kind != b"SHF0" || u32_at(d, o + 4)? != id {
                return Err(Error::invalid(format!("REP0 pool {i} at {o:#x}: {kind:?}, id {}", u32_at(d, o + 4)?)));
            }
            if (0..6).map(|k| u32_at(d, o + 0xC + 4 * k)).collect::<Result<Vec<_>>>()?.iter().any(|&w| w != 0) {
                return Err(Error::invalid(format!("REP0 pool {i}: non-zero runtime words")));
            }
            let (name, end) = cstr_at(d, o + 0x24)?;
            zero_pad(d, end, "REP0 pool name")?;
            cov.claim_padded(o, end);
            pools.push(Pool { kind, id, size: u32_at(d, o + 8)?, name });
        }
        cov.check(d.len(), "REP0")?;
        Ok(Self { version, pools })
    }

    pub fn pool(&self, id: u32) -> Option<&Pool> {
        self.pools.iter().find(|p| p.id == id)
    }
}
