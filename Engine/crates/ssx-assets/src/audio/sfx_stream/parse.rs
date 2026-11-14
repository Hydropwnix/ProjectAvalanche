use anyhow::{Result, bail};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    V0,
    Sps,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockType {
    Header,
    Data,
    End,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Head {
    pub size: usize,
    pub ty: BlockType,
    pub last: bool,
}

pub const HEAD_BYTES: usize = 8;

const SPS_HEADER: u8 = b'H';
const SPS_DATA: u8 = b'D';
const SPS_END: u8 = b'E';
const SPS_U: u8 = b'U';

impl BlockKind {
    pub fn head_bytes(self) -> usize {
        match self {
            BlockKind::V0 => HEAD_BYTES,
            BlockKind::Sps => 4,
        }
    }

    pub fn head(self, bytes: &[u8], max: usize) -> Result<Head> {
        let Some(word) = bytes.get(..4).map(|w| u32::from_be_bytes([w[0], w[1], w[2], w[3]])) else {
            bail!("a block head of {} bytes", bytes.len());
        };
        let head = match self {
            BlockKind::V0 => {
                let size = (word & 0x7FFF_FFFF) as usize;
                if size < 8 {
                    bail!("a version-0 block of {size} bytes");
                }
                Head { size, ty: BlockType::Data, last: word & 0x8000_0000 != 0 }
            }
            BlockKind::Sps => {
                let size = (word & 0x00FF_FFFF) as usize;
                match (word >> 24) as u8 {
                    SPS_HEADER if size >= 4 + 8 => Head { size, ty: BlockType::Header, last: false },
                    SPS_DATA if size >= 8 => Head { size, ty: BlockType::Data, last: false },
                    SPS_U if size >= 4 => Head { size, ty: BlockType::Other, last: false },
                    SPS_HEADER | SPS_DATA | SPS_U => bail!("an SPS block {word:#010x} too short"),
                    SPS_END => Head { size: size.max(4), ty: BlockType::End, last: true },
                    _ => Head { size: 0, ty: BlockType::End, last: true },
                }
            }
        };
        if head.size > max {
            bail!("a block of {} bytes in a {max}-byte ring", head.size);
        }
        Ok(head)
    }
}
