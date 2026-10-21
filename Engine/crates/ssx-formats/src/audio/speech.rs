use crate::{Error, Result};

pub mod ctx;
pub mod evt;
pub mod gas;
pub mod grf;
pub mod kwd;
pub mod rep;
pub mod spc;

pub fn fnv1(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811C_9DC5u32, |h, &b| h.wrapping_mul(0x0100_0193) ^ u32::from(b))
}

pub(crate) fn u32_at(d: &[u8], off: usize) -> Result<u32> {
    d.get(off..off + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]])).ok_or(Error::UnexpectedEof {
        offset: off,
        wanted: 4,
        available: d.len().saturating_sub(off),
    })
}

pub(crate) fn i32_at(d: &[u8], off: usize) -> Result<i32> {
    u32_at(d, off).map(|v| v as i32)
}

pub(crate) fn u16_at(d: &[u8], off: usize) -> Result<u16> {
    d.get(off..off + 2).map(|b| u16::from_be_bytes([b[0], b[1]])).ok_or(Error::UnexpectedEof {
        offset: off,
        wanted: 2,
        available: d.len().saturating_sub(off),
    })
}

pub(crate) fn u8_at(d: &[u8], off: usize) -> Result<u8> {
    d.get(off).copied().ok_or(Error::UnexpectedEof { offset: off, wanted: 1, available: 0 })
}

pub(crate) fn cstr_at(d: &[u8], off: usize) -> Result<(String, usize)> {
    let tail = d.get(off..).ok_or(Error::UnexpectedEof { offset: off, wanted: 1, available: 0 })?;
    let n =
        tail.iter().position(|&b| b == 0).ok_or_else(|| Error::invalid(format!("unterminated string at {off:#x}")))?;
    Ok((tail[..n].iter().map(|&b| char::from(b)).collect(), off + n + 1))
}

pub(crate) fn offset(d: &[u8], v: i32, what: &str) -> Result<Option<usize>> {
    match v {
        -1 => Ok(None),
        v if v >= 0 && (v as usize) < d.len() => Ok(Some(v as usize)),
        v => Err(Error::invalid(format!("{what}: offset {v:#x} outside the file ({:#x})", d.len()))),
    }
}

pub(crate) fn header(d: &[u8], magic: &'static [u8; 4]) -> Result<(u16, u16)> {
    if d.get(..4) != Some(magic.as_slice()) {
        return Err(Error::BadMagic { offset: 0, expected: magic, found: d.iter().take(4).copied().collect() });
    }
    if d.get(4..8) != Some(b"X360".as_slice()) {
        return Err(Error::BadMagic {
            offset: 4,
            expected: b"X360",
            found: d.iter().skip(4).take(4).copied().collect(),
        });
    }
    Ok((u16_at(d, 8)?, u16_at(d, 10)?))
}

#[derive(Default)]
pub(crate) struct Coverage {
    ranges: Vec<(usize, usize)>,
}

impl Coverage {
    pub(crate) fn claim(&mut self, start: usize, end: usize) {
        self.ranges.push((start, end));
    }

    pub(crate) fn claim_padded(&mut self, start: usize, end: usize) {
        self.ranges.push((start, end.next_multiple_of(4)));
    }

    pub(crate) fn check(mut self, end: usize, what: &str) -> Result<()> {
        self.ranges.sort_unstable();
        self.ranges.dedup();
        let mut pos = 0;
        for &(a, b) in &self.ranges {
            if a < pos {
                return Err(Error::invalid(format!("{what}: records overlap at {a:#x} (previous end {pos:#x})")));
            }
            if a > pos {
                return Err(Error::invalid(format!("{what}: unclaimed bytes {pos:#x}..{a:#x}")));
            }
            pos = b;
        }
        if pos != end && !(pos > end && pos - end < 4) {
            return Err(Error::invalid(format!("{what}: records end at {pos:#x}, the data at {end:#x}")));
        }
        Ok(())
    }
}

pub(crate) fn zero_pad(d: &[u8], end: usize, what: &str) -> Result<()> {
    let to = end.next_multiple_of(4).min(d.len());
    if d[end.min(d.len())..to].iter().any(|&b| b != 0) {
        return Err(Error::invalid(format!("{what}: non-zero padding at {end:#x}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn be(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_be_bytes()).collect()
    }

    #[test]
    fn fnv1_matches_the_reference() {
        assert_eq!(fnv1(b""), 0x811C_9DC5);
        assert_eq!(fnv1(b"a"), 0x050C_5D7E);
    }

    fn evt0() -> Vec<u8> {
        let mut d = b"EVT0X360".to_vec();
        d.extend(be(&[0x0003_0002, 0, 0x1234_0000, 1, 1, 0x24, 0x28]));
        d.extend(be(&[0x2C, 0x68]));
        d.extend(be(&[8, 2, 0, 0, 0, 0x1234_0042, 0, 0, 0, 0, 0, 0, 7, 9]));
        d.extend(b"ev\0\0");
        assert_eq!(d.len(), 0x68);
        d.extend(be(&[1, 7, 1, 0, 0x80]));
        d.extend(b"ty\0\0");
        d.extend(be(&[0x84]));
        d.extend(be(&[0, 4]));
        d.extend(b"v\0\0\0");
        d
    }

    #[test]
    fn evt0_parses_to_its_end() {
        let e = evt::EventSystem::parse(&evt0()).unwrap();
        assert_eq!(e.interface, 0x1234);
        assert_eq!(
            (e.events[0].id, e.events[0].params.as_slice(), e.events[0].name.as_str()),
            (0x1234_0042, &[7, 9][..], "ev")
        );
        assert_eq!(e.types[0].values, vec![(4, "v".to_string())]);
    }

    #[test]
    fn evt0_rejects_unclaimed_bytes() {
        let mut d = evt0();
        d.extend([0, 0, 0, 0]);
        assert!(evt::EventSystem::parse(&d).is_err());
    }

    #[test]
    fn rep0_pool() {
        let mut d = b"REP0X360".to_vec();
        d.extend(be(&[0x0003_0002, 0, 1, 0, 5, 0x20]));
        d.extend(b"SHF0");
        d.extend(be(&[5, 200, 0, 0, 0, 0, 0, 0]));
        d.extend(b"Pool\0\0\0\0");
        let r = rep::RepetitionPools::parse(&d).unwrap();
        assert_eq!((r.pools[0].id, r.pools[0].size, r.pools[0].name.as_str()), (5, 200, "Pool"));
    }

    #[test]
    fn graffiti_bucket_is_in_range() {
        for (t, v, n) in [(14, 0x190, 1), (4, 12, 2), (13, 1, 3)] {
            assert!(grf::bucket(t, v, n, 7) < 7);
        }
    }
}
