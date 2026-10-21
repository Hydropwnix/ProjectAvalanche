use crate::reader::Reader;
use crate::{Error, Result};

pub const HEADER_SIZE: usize = 0x30;

#[derive(Clone, Debug)]
pub struct SpdtFile {
    pub kind: SpdtKind,
    pub type_name: String,
    pub version: u32,
    pub bbox_min: [f32; 3],
    pub bbox_max: [f32; 3],
    pub width: u16,
    pub height: u16,
    pub cells: SpdtCells,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpdtKind {
    Boundary,
    Height,
}

#[derive(Clone, Debug)]
pub enum SpdtCells {
    Boundary(Vec<u16>),
    Height(Vec<f32>),
}

impl SpdtFile {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::le(data);
        r.expect(b"SDAT")?;
        let kind = match &r.array::<4>()? {
            b"BNDY" => SpdtKind::Boundary,
            b"HEIG" => SpdtKind::Height,
            other => return Err(Error::invalid(format!("unknown .spdt subtype {other:?}"))),
        };
        let type_name = r.fixed_str(8)?;
        let version = r.u32()?;
        let bbox_min = r.vec3()?;
        let bbox_max = r.vec3()?;
        let width = r.u16()?;
        let height = r.u16()?;

        let count = width as usize * height as usize;
        let stride = match kind {
            SpdtKind::Boundary => 2,
            SpdtKind::Height => 4,
        };
        if r.remaining() != count * stride {
            return Err(Error::invalid(format!(
                ".spdt payload is {} bytes, expected {width}×{height}×{stride}",
                r.remaining()
            )));
        }
        let cells = match kind {
            SpdtKind::Boundary => SpdtCells::Boundary((0..count).map(|_| r.u16()).collect::<Result<_>>()?),
            SpdtKind::Height => SpdtCells::Height((0..count).map(|_| r.f32()).collect::<Result<_>>()?),
        };
        Ok(Self { kind, type_name, version, bbox_min, bbox_max, width, height, cells })
    }

    pub fn cell_size(&self) -> [f32; 2] {
        [
            (self.bbox_max[0] - self.bbox_min[0]) / self.width as f32,
            (self.bbox_max[2] - self.bbox_min[2]) / self.height as f32,
        ]
    }
}
