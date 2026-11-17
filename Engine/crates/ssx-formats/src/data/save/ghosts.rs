use super::{Header, LIBRARY_HEADER_LEN, crc32_bzip2};
use crate::reader::Reader;
use crate::{Error, Result};

pub const GHOSTS_HEADER_NAME: &[u8; 16] = b"TXT_CONSOLEFILE\0";
pub const GHOSTS_GROUP_WORD: u32 = 0xC;
pub const REGION_WORD: u32 = 0xC;
pub const COMMON_REGION_SIZE: usize = 0x3C08;
pub const SLOT_REGION_SIZE: usize = 0x3E808;
pub const SLOTS: usize = 256;
pub const SLOT_TABLE_BYTES: usize = SLOTS * 4;
pub const MAX_SLOT_CONTAINER: usize = SLOT_REGION_SIZE - 20;

pub const fn common_offset() -> usize {
    LIBRARY_HEADER_LEN
}

pub const fn slot_offset(slot: usize) -> usize {
    LIBRARY_HEADER_LEN + COMMON_REGION_SIZE + slot * SLOT_REGION_SIZE
}

pub const MAX_FILE_LEN: usize = slot_offset(SLOTS);

pub fn header_bytes() -> [u8; LIBRARY_HEADER_LEN] {
    let mut h = [0u8; LIBRARY_HEADER_LEN];
    h[..16].copy_from_slice(GHOSTS_HEADER_NAME);
    h[0x14..0x18].copy_from_slice(&GHOSTS_GROUP_WORD.to_be_bytes());
    h
}

pub fn check_header(data: &[u8]) -> Result<Header> {
    let h = Header::parse(data.get(..LIBRARY_HEADER_LEN).ok_or_else(|| Error::invalid("ssxvghosts: no header"))?)?;
    if h.name != *GHOSTS_HEADER_NAME || h.group != GHOSTS_GROUP_WORD {
        return Err(Error::invalid(format!(
            "ssxvghosts: header {:?} group {:#x}, the group's is TXT_CONSOLEFILE / {GHOSTS_GROUP_WORD:#x}",
            String::from_utf8_lossy(&h.name),
            h.group
        )));
    }
    // PORT(choice): a header checksum other than 0 is refused. The console's writer of this group leaves it 0 (its
    // regions are written in place, each with its own CRC: the user's level-15 file and a new one, both 0); its reader
    // checks the name, group, size and a CRC-32/IEEE of the file only when it is not 0 (0x8315F870) — a file the
    // console never writes.
    if h.checksum != 0 {
        return Err(Error::invalid(format!("ssxvghosts: header checksum {:#010x}, the group writes 0", h.checksum)));
    }
    Ok(h)
}

pub fn parse_region(region: &[u8], chunks: usize) -> Result<Vec<&[u8]>> {
    let mut r = Reader::be(region);
    let crc = r.u32()?;
    let computed = crc32_bzip2(&region[4..]);
    if crc != computed {
        return Err(Error::invalid(format!("ssxvghosts: region CRC {crc:#010x}, computed {computed:#010x}")));
    }
    let word = r.u32()?;
    if word != REGION_WORD {
        return Err(Error::invalid(format!("ssxvghosts: region word {word:#x}, the group writes {REGION_WORD:#x}")));
    }
    let count = r.u32()? as usize;
    if count != chunks {
        return Err(Error::invalid(format!("ssxvghosts: {count} chunks, the region registers {chunks}")));
    }
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let n = r.u32()? as usize;
        out.push(r.bytes(n)?);
    }
    if region[r.pos()..].iter().any(|&b| b != 0) {
        return Err(Error::invalid("ssxvghosts: bytes after the region's chunks"));
    }
    Ok(out)
}

pub fn write_region(chunks: &[&[u8]], size: usize) -> Result<Vec<u8>> {
    let mut out = vec![0u8; 4];
    out.extend_from_slice(&REGION_WORD.to_be_bytes());
    out.extend_from_slice(&(chunks.len() as u32).to_be_bytes());
    for c in chunks {
        let n = u32::try_from(c.len()).map_err(|_| Error::invalid("ssxvghosts: chunk above 4 GB"))?;
        out.extend_from_slice(&n.to_be_bytes());
        out.extend_from_slice(c);
    }
    if out.len() > size {
        return Err(Error::invalid(format!("ssxvghosts: {} bytes of chunks in a {size}-byte region", out.len())));
    }
    out.resize(size, 0);
    let crc = crc32_bzip2(&out[4..]);
    out[..4].copy_from_slice(&crc.to_be_bytes());
    Ok(out)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommonChunk {
    pub slots: Vec<u32>,
    pub database: Vec<u8>,
}

impl CommonChunk {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::be(data);
        let slots = (0..SLOTS).map(|_| r.u32()).collect::<Result<Vec<_>>>()?;
        let n = r.u32()? as usize;
        let database = r.bytes(n)?.to_vec();
        if r.pos() != data.len() {
            return Err(Error::invalid(format!("ssxvghosts: {} bytes after the database", data.len() - r.pos())));
        }
        Ok(CommonChunk { slots, database })
    }

    pub fn write(&self) -> Result<Vec<u8>> {
        if self.slots.len() != SLOTS {
            return Err(Error::invalid(format!(
                "ssxvghosts: {} slot entries, the table has {SLOTS}",
                self.slots.len()
            )));
        }
        let n = u32::try_from(self.database.len()).map_err(|_| Error::invalid("ssxvghosts: database above 4 GB"))?;
        let mut out = Vec::with_capacity(SLOT_TABLE_BYTES + 4 + self.database.len());
        for s in &self.slots {
            out.extend_from_slice(&s.to_be_bytes());
        }
        out.extend_from_slice(&n.to_be_bytes());
        out.extend_from_slice(&self.database);
        Ok(out)
    }

    pub fn region(&self) -> Result<Vec<u8>> {
        write_region(&[&self.write()?], COMMON_REGION_SIZE)
    }

    pub fn from_region(region: &[u8]) -> Result<Self> {
        if region.len() != COMMON_REGION_SIZE {
            return Err(Error::invalid(format!("ssxvghosts: common region of {} bytes", region.len())));
        }
        let chunks = parse_region(region, 1)?;
        Self::parse(chunks[0])
    }
}

pub fn slot_region(container: &[u8]) -> Result<Vec<u8>> {
    if container.len() > MAX_SLOT_CONTAINER {
        return Err(Error::invalid(format!(
            "ssxvghosts: a ghost of {} bytes, a slot holds {MAX_SLOT_CONTAINER}",
            container.len()
        )));
    }
    let mut chunk = Vec::with_capacity(4 + container.len());
    chunk.extend_from_slice(&(container.len() as u32).to_be_bytes());
    chunk.extend_from_slice(container);
    write_region(&[&chunk], SLOT_REGION_SIZE)
}

pub fn slot_container(region: &[u8]) -> Result<&[u8]> {
    if region.len() != SLOT_REGION_SIZE {
        return Err(Error::invalid(format!("ssxvghosts: slot region of {} bytes", region.len())));
    }
    let chunks = parse_region(region, 1)?;
    let mut r = Reader::be(chunks[0]);
    let n = r.u32()? as usize;
    let container = r.bytes(n)?;
    if r.pos() != chunks[0].len() {
        return Err(Error::invalid("ssxvghosts: bytes after the slot's container"));
    }
    Ok(container)
}

pub fn check_len(len: usize) -> Result<usize> {
    let rest =
        len.checked_sub(slot_offset(0)).ok_or_else(|| Error::invalid("ssxvghosts: shorter than the common region"))?;
    if rest % SLOT_REGION_SIZE != 0 || rest / SLOT_REGION_SIZE > SLOTS {
        return Err(Error::invalid(format!(
            "ssxvghosts: {len} bytes is not the header, the common region and whole slots"
        )));
    }
    Ok(rest / SLOT_REGION_SIZE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_offsets_are_the_groups() {
        assert_eq!(common_offset(), 0x1C);
        assert_eq!(slot_offset(0), 0x1C + 0x3C08);
        assert_eq!(slot_offset(16), 4_111_524);
        assert_eq!(check_len(4_111_524).unwrap(), 16);
        assert_eq!(check_len(15_396).unwrap(), 0);
        assert!(check_len(15_395).is_err());
        assert!(check_len(4_111_523).is_err());
        assert!(check_len(MAX_FILE_LEN + SLOT_REGION_SIZE).is_err());
    }

    #[test]
    fn regions_round_trip_and_damage_is_refused() {
        let mut slots = vec![0u32; SLOTS];
        slots[3] = 0x643361E7;
        let common = CommonChunk { slots, database: vec![0x44, 0x42, 0, 8, 1, 2, 3] };
        let region = common.region().unwrap();
        assert_eq!(region.len(), COMMON_REGION_SIZE);
        assert_eq!(region[4..12], [0, 0, 0, 0xC, 0, 0, 0, 1]);
        assert_eq!(CommonChunk::from_region(&region).unwrap(), common);
        let mut bad = region.clone();
        bad[0x500] ^= 0x80;
        assert!(CommonChunk::from_region(&bad).is_err(), "the CRC");
        let container = vec![7u8; 1000];
        let slot = slot_region(&container).unwrap();
        assert_eq!(slot.len(), SLOT_REGION_SIZE);
        assert_eq!(slot_container(&slot).unwrap(), &container[..]);
        assert!(slot_container(&vec![0u8; SLOT_REGION_SIZE]).is_err(), "an unwritten slot");
        assert!(slot_region(&vec![0u8; MAX_SLOT_CONTAINER + 1]).is_err());
        assert!(slot_region(&vec![0u8; MAX_SLOT_CONTAINER]).is_ok());
        let mut word = slot.clone();
        word[7] = 0xD;
        let crc = crc32_bzip2(&word[4..]);
        word[..4].copy_from_slice(&crc.to_be_bytes());
        assert!(slot_container(&word).is_err(), "the region word");
    }

    #[test]
    fn the_header_is_the_groups() {
        let h = header_bytes().to_vec();
        assert_eq!(h[..0x1C], *b"TXT_CONSOLEFILE\0\0\0\0\0\0\0\0\x0c\0\0\0\0");
        assert!(check_header(&h).is_ok());
        let mut sum = h.clone();
        sum[0x13] = 1;
        assert!(check_header(&sum).is_err());
        let mut prof = h.clone();
        prof[..16].copy_from_slice(super::super::PROFILE_HEADER_NAME);
        assert!(check_header(&prof).is_err());
    }
}
