use super::{header, u32_at};
use crate::{Error, Result};

pub const MAGIC: &[u8; 4] = b"KWD0";

#[derive(Clone, Debug)]
pub struct Keywords {
    pub version: (u16, u16),
    pub pairs: Vec<(u32, u32)>,
}

impl Keywords {
    pub fn parse(d: &[u8]) -> Result<Self> {
        let version = header(d, MAGIC)?;
        let (z, n) = (u32_at(d, 0xC)?, u32_at(d, 0x10)? as usize);
        if z != 0 || d.len() != 0x14 + 8 * n {
            return Err(Error::invalid(format!("KWD0 header {z:#x}, {n} pairs in {:#x} bytes", d.len())));
        }
        let pairs =
            (0..n).map(|i| Ok((u32_at(d, 0x14 + 8 * i)?, u32_at(d, 0x18 + 8 * i)?))).collect::<Result<Vec<_>>>()?;
        if pairs.windows(2).any(|w| w[0].0 > w[1].0) {
            return Err(Error::invalid("KWD0 pairs not sorted by sample id"));
        }
        Ok(Self { version, pairs })
    }

    pub fn of(&self, sample: u32) -> impl Iterator<Item = u32> + '_ {
        let i = self.pairs.partition_point(|p| p.0 < sample);
        self.pairs[i..].iter().take_while(move |p| p.0 == sample).map(|p| p.1)
    }
}
