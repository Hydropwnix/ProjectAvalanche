use crate::reader::Reader;
use crate::{Error, Result};

pub const MAGIC: &[u8; 4] = b"irf1";

#[derive(Clone, Debug)]
pub struct Irf {
    pub version: u32,
    pub channels: u32,
    pub samples: u32,
    pub rate: u32,
    pub block: u32,
    pub bins: u32,
    pub unk_flag: u32,
    pub records: Vec<IrfRecord>,
}

#[derive(Clone, Debug)]
pub struct IrfRecord {
    pub scale: u16,
    pub unk_2: u16,
    pub unk_4: f32,
    pub unk_8: u16,
    pub bins: Vec<(i16, i16)>,
}

impl Irf {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::be(data);
        r.expect(MAGIC)?;
        let (version, channels, samples, rate, block, bins, values, unk_flag) =
            (r.u32()?, r.u32()?, r.u32()?, r.u32()?, r.u32()?, r.u32()?, r.u32()?, r.u32()?);
        let zero = (r.u32()?, r.u32()?, r.u32()?);
        if version != 1 || values != 2 * bins || zero != (0, 0, 0) || block == 0 || channels == 0 || channels > 8 {
            return Err(Error::invalid(format!("irf1 header {version} {channels} {block} {bins} {values} {zero:?}")));
        }
        let parts = samples.div_ceil(block) as usize;
        let mut records = Vec::with_capacity(parts * channels as usize);
        for _ in 0..parts * channels as usize {
            let (scale, unk_2, unk_4, unk_8) = (r.u16()?, r.u16()?, r.f32()?, r.u16()?);
            if r.bytes(6)?.iter().any(|&b| b != 0) || (u32::from(scale) + 1).count_ones() != 1 {
                return Err(Error::invalid(format!("irf1 record scale {scale:#x} / padding")));
            }
            let bins = (0..bins).map(|_| Ok((r.i16()?, r.i16()?))).collect::<Result<Vec<_>>>()?;
            records.push(IrfRecord { scale, unk_2, unk_4, unk_8, bins });
        }
        if r.remaining() != 0 {
            return Err(Error::invalid(format!("{} bytes after the irf1 records", r.remaining())));
        }
        Ok(Self { version, channels, samples, rate, block, bins, unk_flag, records })
    }
}
