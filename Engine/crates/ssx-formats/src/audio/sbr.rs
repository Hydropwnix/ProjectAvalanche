use crate::eaac::SoundHeader;
use crate::reader::Reader;
use crate::{Error, Result};

pub const HEADER_SIZE: usize = 0x30;
pub const REGION_SIZE: usize = 0x80;

#[derive(Clone, Debug)]
pub struct SampleBank {
    pub unk_04: u32,
    pub bank_id: u32,
    pub region_offset: u32,
    pub unk_region_first: u16,
    pub item_types: Vec<ItemType>,
    pub sounds: Vec<BankSound>,
    pub table_offset: u32,
    pub types_offset: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemType {
    pub fourcc: [u8; 4],
    pub unk_a: u8,
    pub unk_b: u8,
}

#[derive(Clone, Debug)]
pub struct BankSound {
    pub id: u32,
    pub items_offset: u32,
    pub items: Vec<Item>,
}

#[derive(Clone, Debug)]
pub struct Item {
    pub kind: u16,
    pub data_offset: u32,
}

#[derive(Clone, Debug)]
pub enum SoundRef {
    SnrSns { header: SoundHeader, header_offset: u32, sns_offset: u32 },
    Sps { sps_offset: u32 },
}

impl SampleBank {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::be(data);
        r.expect(b"SBKR")?;
        let unk_04 = r.u32()?;
        if r.bytes(12)?.iter().any(|&b| b != 0) {
            return Err(Error::invalid("SBKR 0x08..0x14 not zero"));
        }
        let bank_id = r.u32()?;
        let zero_18 = r.u32()?;
        let count = r.u32()? as usize;
        let region_offset = r.u32()?;
        let table_offset = r.u32()?;
        let types_offset = r.u32()?;
        let zero_2c = r.u32()?;
        if zero_18 != 0 || zero_2c != 0 || region_offset as usize != HEADER_SIZE {
            return Err(Error::invalid(format!("SBKR header: {zero_18:#x} {region_offset:#x} {zero_2c:#x}")));
        }
        if types_offset as usize != HEADER_SIZE + REGION_SIZE || table_offset < types_offset {
            return Err(Error::invalid(format!("SBKR tables at {types_offset:#x} / {table_offset:#x}")));
        }
        let unk_region_first = r.u16()?;
        if r.bytes(REGION_SIZE - 2)?.iter().any(|&b| b != 0) {
            return Err(Error::invalid("SBKR region 0x32..0xB0 not zero"));
        }

        let type_bytes = (table_offset - types_offset) as usize;
        if !type_bytes.is_multiple_of(6) {
            return Err(Error::invalid(format!("SBKR type table is {type_bytes} bytes")));
        }
        let mut item_types = Vec::with_capacity(type_bytes / 6);
        for _ in 0..type_bytes / 6 {
            item_types.push(ItemType { fourcc: r.array()?, unk_a: r.u8()?, unk_b: r.u8()? });
        }

        let mut sounds = Vec::with_capacity(count);
        for _ in 0..count {
            let id = r.u32()?;
            let n = r.u16()? as usize;
            let items_offset = r.u32()?;
            let mut ir = Reader::be(data);
            ir.seek(items_offset as usize)?;
            let mut items = Vec::with_capacity(n);
            for _ in 0..n {
                let kind = ir.u16()?;
                if kind as usize >= item_types.len() {
                    return Err(Error::invalid(format!("SBKR sound {id:#x}: item type {kind}")));
                }
                items.push(Item { kind, data_offset: ir.u32()? });
            }
            sounds.push(BankSound { id, items_offset, items });
        }
        Ok(Self { unk_04, bank_id, region_offset, unk_region_first, item_types, sounds, table_offset, types_offset })
    }

    fn item_fourcc(&self, item: &Item) -> &[u8; 4] {
        &self.item_types[item.kind as usize].fourcc
    }

    pub fn resolve(&self, data: &[u8], sound: &BankSound) -> Result<SoundRef> {
        let (mut snr, mut sns) = (None, None);
        for item in &sound.items {
            let at = item.data_offset as usize;
            match self.item_fourcc(item) {
                b"SNR1" => snr = Some((SoundHeader::parse(data.get(at..).unwrap_or_default())?, item.data_offset)),
                b"SNS1" => sns = Some(Reader::be(data.get(at..).unwrap_or_default()).u32()?),
                other => return Err(Error::invalid(format!("SBKR sound {:#x}: item {other:?}", sound.id))),
            }
        }
        match (snr, sns) {
            (Some((header, header_offset)), Some(sns_offset)) => {
                Ok(SoundRef::SnrSns { header, header_offset, sns_offset })
            }
            (None, Some(sps_offset)) => Ok(SoundRef::Sps { sps_offset }),
            _ => Err(Error::invalid(format!("SBKR sound {:#x}: items {:?}", sound.id, sound.items))),
        }
    }
}

pub const SBS_HEADER_SIZE: usize = 0x10;

pub fn check_sbs_header(data: &[u8]) -> Result<()> {
    let mut r = Reader::be(data);
    r.expect(b"SBKS")?;
    if r.bytes(12)?.iter().any(|&b| b != 0) {
        return Err(Error::invalid("SBKS header bytes 0x04..0x10 not zero"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_sps_sound() {
        let mut d = b"SBKR".to_vec();
        d.extend(0x0110_0000u32.to_be_bytes());
        d.extend([0; 12]);
        for w in [0xF33A_DC20u32, 0, 1, 0x30, 0xB6, 0xB0, 0] {
            d.extend(w.to_be_bytes());
        }
        d.extend([0x00, 0x7E]);
        d.resize(0xB0, 0);
        d.extend(b"SNS1\x06\x04");
        d.extend([0, 0, 0, 0x91, 0, 1, 0, 0, 0, 0xC0]);
        d.extend([0, 0, 0, 0, 0, 0xC6]);
        d.extend(0x10u32.to_be_bytes());
        let b = SampleBank::parse(&d).unwrap();
        assert_eq!((b.bank_id, b.unk_region_first, b.sounds.len(), b.sounds[0].id), (0xF33A_DC20, 0x7E, 1, 0x91));
        assert!(matches!(b.resolve(&d, &b.sounds[0]).unwrap(), SoundRef::Sps { sps_offset: 0x10 }));
        assert!(check_sbs_header(b"SBKS\0\0\0\0\0\0\0\0\0\0\0\0").is_ok());
    }
}
