use super::{Coverage, cstr_at, header, i32_at, u32_at, zero_pad};
use crate::{Error, Result};

pub const MAGIC: &[u8; 4] = b"GRF0";

#[derive(Clone, Debug)]
pub struct Graffiti {
    pub version: (u16, u16),
    pub unk_10: u32,
    pub buckets: u32,
    pub entries: Vec<Entry>,
    pub names: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub tag: u32,
    pub value: u32,
    pub count: u32,
    pub samples: Vec<u32>,
}

pub fn bucket(tag: u32, value: u32, count: u32, buckets: u32) -> u32 {
    (((tag ^ 0x050C_5D1F).wrapping_mul(0x0100_0193) ^ value).wrapping_mul(0x0100_0193) ^ count) % buckets.max(1)
}

impl Graffiti {
    pub fn parse(d: &[u8]) -> Result<Self> {
        let version = header(d, MAGIC)?;
        let (rel, unk_10, nb, ptr) = (u32_at(d, 0xC)?, u32_at(d, 0x10)?, u32_at(d, 0x14)?, u32_at(d, 0x18)?);
        if rel != 0 || ptr != 0 || nb == 0 || nb > 0x10000 {
            return Err(Error::invalid(format!("GRF0 header {rel:#x} {nb} {ptr:#x}")));
        }
        const BASE: usize = 0x1C;
        let mut cov = Coverage::default();
        cov.claim(0, BASE + 8 * nb as usize);
        let mut entries = Vec::new();
        let mut end = BASE + 8 * nb as usize;
        for b in 0..nb {
            let (n, off) = (u32_at(d, BASE + 8 * b as usize)? as usize, i32_at(d, BASE + 8 * b as usize + 4)?);
            if n == 0 {
                if off != -1 {
                    return Err(Error::invalid(format!("GRF0 empty bucket {b} with offset {off:#x}")));
                }
                continue;
            }
            let o = BASE + usize::try_from(off).map_err(|_| Error::invalid("GRF0 bucket offset"))?;
            cov.claim(o, o + 16 * n);
            end = end.max(o + 16 * n);
            for k in 0..n {
                let e = o + 16 * k;
                let (tag, value, count, lo) = (u32_at(d, e)?, u32_at(d, e + 4)?, u32_at(d, e + 8)?, u32_at(d, e + 12)?);
                if bucket(tag, value, count, nb) != b {
                    return Err(Error::invalid(format!("GRF0 entry ({tag}, {value}, {count}) in bucket {b}")));
                }
                let l = BASE + lo as usize;
                let m = u32_at(d, l)? as usize;
                cov.claim(l, l + 4 + 4 * m);
                end = end.max(l + 4 + 4 * m);
                let samples = (0..m).map(|j| u32_at(d, l + 4 + 4 * j)).collect::<Result<Vec<_>>>()?;
                entries.push(Entry { tag, value, count, samples });
            }
        }
        let mut names = Vec::new();
        while end < d.len() {
            let (name, e) = cstr_at(d, end)?;
            zero_pad(d, e, "GRF0 name")?;
            cov.claim_padded(end, e);
            names.push(name);
            end = e.next_multiple_of(4);
        }
        if !names.is_empty() && unk_10 != 1 {
            return Err(Error::invalid("GRF0 names without the unk_10 flag"));
        }
        cov.check(d.len(), "GRF0")?;
        Ok(Self { version, unk_10, buckets: nb, entries, names })
    }

    pub fn find(&self, tag: u32, value: u32, count: u32) -> Option<&Entry> {
        self.entries.iter().find(|e| e.tag == tag && e.value == value && e.count == count)
    }
}
