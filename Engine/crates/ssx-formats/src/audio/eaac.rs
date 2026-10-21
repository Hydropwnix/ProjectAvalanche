use crate::reader::Reader;
use crate::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Codec {
    None,
    Reserved,
    Pcm16Be,
    EaXma,
    Xas1,
    EaLayer3V1,
    EaLayer3V2Pcm,
    EaLayer3V2Spike,
    Other(u8),
}

impl Codec {
    pub fn from_id(id: u8) -> Self {
        match id {
            0 => Codec::None,
            1 => Codec::Reserved,
            2 => Codec::Pcm16Be,
            3 => Codec::EaXma,
            4 => Codec::Xas1,
            5 => Codec::EaLayer3V1,
            6 => Codec::EaLayer3V2Pcm,
            7 => Codec::EaLayer3V2Spike,
            n => Codec::Other(n),
        }
    }

    pub fn fourcc(self) -> Option<&'static str> {
        match self {
            Codec::Pcm16Be => Some("P6B0"),
            Codec::EaXma => Some("EXm0"),
            Codec::Xas1 => Some("Xas1"),
            Codec::EaLayer3V1 => Some("EL31"),
            Codec::EaLayer3V2Pcm => Some("L32P"),
            Codec::EaLayer3V2Spike => Some("L32S"),
            Codec::Other(9) => Some("Esp0"),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Storage {
    Ram,
    Stream,
    Gigasample,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SoundHeader {
    pub version: u8,
    pub codec: Codec,
    pub channels: u8,
    pub sample_rate: u32,
    pub storage: Storage,
    pub looped: bool,
    pub num_samples: u32,
    pub loop_start: Option<u32>,
    pub loop_offset: Option<u32>,
    pub prefetch_samples: Option<u32>,
    pub size: usize,
}

impl SoundHeader {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::be(data);
        let w1 = r.u32()?;
        let w2 = r.u32()?;
        let version = (w1 >> 28) as u8;
        if version > 1 {
            return Err(Error::invalid(format!("EAAC header version {version} (word {w1:#010x})")));
        }
        let storage = match w2 >> 30 {
            0 => Storage::Ram,
            1 => Storage::Stream,
            2 => Storage::Gigasample,
            _ => return Err(Error::invalid(format!("EAAC storage type 3 (word {w2:#010x})"))),
        };
        let looped = (w2 >> 29) & 1 != 0;
        let loop_start = if looped { Some(r.u32()?) } else { None };
        let mut loop_offset = None;
        let mut prefetch_samples = None;
        match storage {
            Storage::Ram => {}
            Storage::Stream => {
                if looped {
                    loop_offset = Some(r.u32()?);
                }
            }
            Storage::Gigasample => {
                let prefetch = r.u32()?;
                prefetch_samples = Some(prefetch);
                if looped && loop_start.unwrap_or(0) >= prefetch {
                    loop_offset = Some(r.u32()?);
                }
            }
        }
        Ok(Self {
            version,
            codec: Codec::from_id(((w1 >> 24) & 0xF) as u8),
            channels: ((w1 >> 18) & 0x3F) as u8 + 1,
            sample_rate: w1 & 0x3FFFF,
            storage,
            looped,
            num_samples: w2 & 0x1FFF_FFFF,
            loop_start,
            loop_offset,
            prefetch_samples,
            size: r.pos(),
        })
    }

    pub fn duration_secs(&self) -> f64 {
        self.num_samples as f64 / self.sample_rate.max(1) as f64
    }

    pub fn has_intro_section(&self) -> bool {
        self.loop_start.is_some_and(|s| s > 0)
    }
}

pub const FLAG_DATA: u8 = 0x00;
pub const FLAG_END: u8 = 0x80;
pub const SPS_HEADER: u8 = b'H';
pub const SPS_DATA: u8 = b'D';
pub const SPS_END: u8 = b'E';

#[derive(Clone, Copy, Debug)]
pub struct Block<'a> {
    pub offset: usize,
    pub flag: u8,
    pub samples: u32,
    pub payload: &'a [u8],
}

impl<'a> Block<'a> {
    pub fn size(&self) -> usize {
        self.payload.len() + 8
    }

    pub fn end(&self) -> usize {
        self.offset + self.size()
    }

    pub fn read(data: &'a [u8], offset: usize) -> Result<Self> {
        let mut r = Reader::be(data);
        r.seek(offset)?;
        let word = r.u32()?;
        let flag = (word >> 24) as u8;
        let size = (word & 0x00FF_FFFF) as usize;
        if size < 8 {
            return Err(Error::invalid(format!("EAAC block at {offset:#x} has size {size}")));
        }
        let samples = r.u32()?;
        let payload = r.bytes(size - 8)?;
        Ok(Self { offset, flag, samples, payload })
    }
}

#[derive(Clone, Debug, Default)]
pub struct SoundBlocks<'a> {
    pub sections: Vec<Vec<Block<'a>>>,
}

impl<'a> SoundBlocks<'a> {
    pub fn blocks(&self) -> impl Iterator<Item = &Block<'a>> {
        self.sections.iter().flatten()
    }

    pub fn samples(&self) -> u64 {
        self.blocks().map(|b| b.samples as u64).sum()
    }

    pub fn end(&self) -> usize {
        self.sections.last().and_then(|s| s.last()).map_or(0, Block::end)
    }
}

fn read_v0_section<'a>(data: &'a [u8], mut offset: usize, max_blocks: Option<usize>) -> Result<Vec<Block<'a>>> {
    let mut out = Vec::new();
    loop {
        let b = Block::read(data, offset)?;
        if b.flag != FLAG_DATA && b.flag != FLAG_END {
            return Err(Error::invalid(format!("EAAC v0 block at {offset:#x} has flag {:#04x}", b.flag)));
        }
        offset = b.end();
        out.push(b);
        if b.flag == FLAG_END || max_blocks.is_some_and(|m| out.len() >= m) {
            return Ok(out);
        }
    }
}

pub fn read_v0_blocks<'a>(header: &SoundHeader, body: &'a [u8], offset: usize) -> Result<SoundBlocks<'a>> {
    if header.version != 0 {
        return Err(Error::invalid("read_v0_blocks on a version-1 header"));
    }
    let mut out = SoundBlocks::default();
    match header.storage {
        Storage::Ram => {
            out.sections.push(read_v0_section(body, offset, Some(1))?);
            if header.has_intro_section() {
                let next = out.end();
                out.sections.push(read_v0_section(body, next, Some(1))?);
            }
        }
        Storage::Stream => {
            out.sections.push(read_v0_section(body, offset, None)?);
            if header.has_intro_section() {
                let rel = header.loop_offset.ok_or_else(|| Error::invalid("looped stream without loop offset"))?;
                out.sections.push(read_v0_section(body, offset + rel as usize, None)?);
            }
        }
        Storage::Gigasample => return Err(Error::invalid("gigasample sounds are not used on this disc")),
    }
    Ok(out)
}

#[derive(Clone, Debug)]
pub struct Sps<'a> {
    pub offset: usize,
    pub header: SoundHeader,
    pub blocks: SoundBlocks<'a>,
    pub end_block_size: usize,
    pub end: usize,
}

impl<'a> Sps<'a> {
    pub fn read(data: &'a [u8], offset: usize) -> Result<Self> {
        let mut r = Reader::be(data);
        r.seek(offset)?;
        let word = r.u32()?;
        if (word >> 24) as u8 != SPS_HEADER {
            return Err(Error::invalid(format!("SPS at {offset:#x} does not start with an H block ({word:#010x})")));
        }
        let hsize = (word & 0x00FF_FFFF) as usize;
        let header = SoundHeader::parse(r.bytes(hsize.saturating_sub(4))?)?;
        if header.version != 1 || header.size + 4 != hsize {
            return Err(Error::invalid(format!("SPS H block at {offset:#x}: size {hsize}, header {header:?}")));
        }
        if header.looped || header.storage == Storage::Gigasample {
            return Err(Error::invalid(format!("SPS at {offset:#x}: looped or gigasample SPS not supported")));
        }
        let mut pos = offset + hsize;
        let mut section = Vec::new();
        loop {
            let word = u32::from_be_bytes(
                data.get(pos..pos + 4)
                    .ok_or(Error::UnexpectedEof { offset: pos, wanted: 4, available: 0 })?
                    .try_into()
                    .expect("4 bytes"),
            );
            match (word >> 24) as u8 {
                SPS_DATA => {
                    let b = Block::read(data, pos)?;
                    pos = b.end();
                    section.push(b);
                }
                SPS_END => {
                    let size = (word & 0x00FF_FFFF) as usize;
                    if size < 4 || pos + size > data.len() {
                        return Err(Error::invalid(format!("SPS E block at {pos:#x} has size {size}")));
                    }
                    let blocks = SoundBlocks { sections: vec![section] };
                    return Ok(Self { offset, header, blocks, end_block_size: size, end: pos + size });
                }
                id => return Err(Error::invalid(format!("SPS block at {pos:#x} has id {id:#04x}"))),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(ws: &[u32]) -> Vec<u8> {
        ws.iter().flat_map(|w| w.to_be_bytes()).collect()
    }

    #[test]
    fn looped_stream_header() {
        let h = SoundHeader::parse(&words(&[0x0504_BB80, 0x6019_8562, 0x0003_85F8, 0x0002_3000])).unwrap();
        assert_eq!((h.version, h.codec, h.channels, h.sample_rate), (0, Codec::EaLayer3V1, 2, 48000));
        assert_eq!((h.storage, h.looped, h.num_samples), (Storage::Stream, true, 0x19_8562));
        assert_eq!((h.loop_start, h.loop_offset, h.size), (Some(0x385F8), Some(0x23000), 16));
        assert!(h.has_intro_section());
        assert_eq!(h.codec.fourcc(), Some("EL31"));
    }

    #[test]
    fn ram_header_and_bad_version() {
        let h = SoundHeader::parse(&words(&[0x0300_7D00, 0x0000_0C80])).unwrap();
        assert_eq!((h.codec, h.channels, h.sample_rate, h.storage, h.size), (Codec::EaXma, 1, 32000, Storage::Ram, 8));
        assert!(SoundHeader::parse(&words(&[0x2300_7D00, 0])).is_err());
        assert_eq!(Codec::from_id(9).fourcc(), Some("Esp0"));
        assert_eq!(Codec::from_id(8).fourcc(), None);
    }

    #[test]
    fn v0_stream_blocks() {
        let h = SoundHeader::parse(&words(&[0x0504_AC44, 0x4000_0008])).unwrap();
        let mut body = vec![0xAA; 4];
        body.extend(words(&[0x0000_000C, 5, 0x1111_1111]));
        body.extend(words(&[0x8000_000A, 3]));
        body.extend([0x22, 0x22]);
        let b = read_v0_blocks(&h, &body, 4).unwrap();
        assert_eq!((b.sections.len(), b.blocks().count(), b.samples(), b.end()), (1, 2, 8, body.len()));
        assert_eq!(b.sections[0][1].flag, FLAG_END);
        let ram = SoundHeader::parse(&words(&[0x0504_AC44, 0x0000_0005])).unwrap();
        assert_eq!(read_v0_blocks(&ram, &body, 4).unwrap().blocks().count(), 1);
    }

    #[test]
    fn sps_sound() {
        let mut d = words(&[0x4800_000C, 0x1504_AC44, 0x4000_0002, 0x4400_000A, 2]);
        d.extend([1, 2]);
        d.extend(words(&[0x4500_0006]));
        d.extend([0, 0]);
        let s = Sps::read(&d, 0).unwrap();
        assert_eq!((s.header.version, s.header.num_samples, s.end_block_size, s.end), (1, 2, 6, d.len()));
        assert_eq!(s.blocks.samples(), 2);
    }
}
