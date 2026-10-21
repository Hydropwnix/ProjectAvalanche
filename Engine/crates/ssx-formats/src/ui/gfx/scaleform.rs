use super::bits::SwfReader;
use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExporterInfo {
    pub version: u16,
    pub flags: u32,
    pub bitmap_format: u16,
    pub prefix: String,
    pub swf_name: String,
    pub code_offsets: Vec<u32>,
}

impl ExporterInfo {
    pub fn read(data: &[u8]) -> Result<Self> {
        let mut r = SwfReader::new(data);
        let version = r.u16()?;
        let flags = if version > 0x109 { r.u32()? } else { 0 };
        let bitmap_format = r.u16()?;
        let prefix = r.string_u8()?;
        let swf_name = r.string_u8()?;
        let mut code_offsets = Vec::new();
        if version > 0x400 {
            let n = r.u16()?;
            for _ in 0..n {
                code_offsets.push(r.u32()?);
            }
        }
        super::expect_end(&r, "ExporterInfo")?;
        Ok(ExporterInfo { version, flags, bitmap_format, prefix, swf_name, code_offsets })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalImage {
    pub id: u32,
    pub format: u16,
    pub target_width: u16,
    pub target_height: u16,
    pub export_name: String,
    pub file_name: String,
}

impl ExternalImage {
    pub const EXPORTER_IMAGE: u32 = 0x9_0000;

    pub fn read(data: &[u8]) -> Result<Self> {
        let mut r = SwfReader::new(data);
        let img = ExternalImage {
            id: r.u32()?,
            format: r.u16()?,
            target_width: r.u16()?,
            target_height: r.u16()?,
            export_name: r.string_u8()?,
            file_name: r.string_u8()?,
        };
        super::expect_end(&r, "DefineExternalImage2")?;
        if img.id & !0x9_FFFF != 0 {
            return Err(Error::invalid(format!("external image id {:#x}", img.id)));
        }
        Ok(img)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SubImage {
    pub id: u16,
    pub image: u16,
    pub x1: u16,
    pub y1: u16,
    pub x2: u16,
    pub y2: u16,
}

impl SubImage {
    pub fn read(data: &[u8]) -> Result<Self> {
        let mut r = SwfReader::new(data);
        let s = SubImage { id: r.u16()?, image: r.u16()?, x1: r.u16()?, y1: r.u16()?, x2: r.u16()?, y2: r.u16()? };
        super::expect_end(&r, "DefineSubImage")?;
        if s.x2 < s.x1 || s.y2 < s.y1 {
            return Err(Error::invalid(format!("sub image {} with an inverted rectangle", s.id)));
        }
        Ok(s)
    }

    pub fn width(&self) -> u16 {
        self.x2 - self.x1
    }

    pub fn height(&self) -> u16 {
        self.y2 - self.y1
    }
}
