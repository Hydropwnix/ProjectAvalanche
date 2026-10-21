use crate::reader::Reader;
use crate::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DdsFormat {
    Dxt1,
    Dxt3,
    Dxt5,
}

impl DdsFormat {
    pub fn bytes_per_block(self) -> usize {
        match self {
            DdsFormat::Dxt1 => 8,
            DdsFormat::Dxt3 | DdsFormat::Dxt5 => 16,
        }
    }

    pub fn fourcc(self) -> &'static [u8; 4] {
        match self {
            DdsFormat::Dxt1 => b"DXT1",
            DdsFormat::Dxt3 => b"DXT3",
            DdsFormat::Dxt5 => b"DXT5",
        }
    }

    pub fn level_size(self, width: u32, height: u32) -> usize {
        width.div_ceil(4) as usize * height.div_ceil(4) as usize * self.bytes_per_block()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DdsFile {
    pub flags: u32,
    pub width: u32,
    pub height: u32,
    pub pitch_or_linear_size: u32,
    pub depth: u32,
    pub mip_count: u32,
    pub unk_reserved1: [u32; 11],
    pub pixel_format_flags: u32,
    pub format: DdsFormat,
    pub unk_pixel_format_masks: [u32; 5],
    pub caps: [u32; 4],
    pub unk_reserved2: u32,
    pub unk_trailing: usize,
}

pub const HEADER_SIZE: usize = 128;

impl DdsFile {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::le(data);
        r.expect(b"DDS ")?;
        let size = r.u32()?;
        if size != 124 {
            return Err(Error::invalid(format!("DDS header size {size}")));
        }
        let flags = r.u32()?;
        let height = r.u32()?;
        let width = r.u32()?;
        let pitch_or_linear_size = r.u32()?;
        let depth = r.u32()?;
        let mip_count = r.u32()?;
        let mut unk_reserved1 = [0u32; 11];
        for v in &mut unk_reserved1 {
            *v = r.u32()?;
        }
        let pf_size = r.u32()?;
        if pf_size != 32 {
            return Err(Error::invalid(format!("DDS pixel format size {pf_size}")));
        }
        let pixel_format_flags = r.u32()?;
        let fourcc = r.array::<4>()?;
        let format = match &fourcc {
            b"DXT1" => DdsFormat::Dxt1,
            b"DXT3" => DdsFormat::Dxt3,
            b"DXT5" => DdsFormat::Dxt5,
            other => return Err(Error::invalid(format!("DDS format {:?}", String::from_utf8_lossy(other)))),
        };
        if pixel_format_flags & 0x4 == 0 {
            return Err(Error::invalid(format!("DDS pixel format flags {pixel_format_flags:#x} without FOURCC")));
        }
        let mut unk_pixel_format_masks = [0u32; 5];
        for v in &mut unk_pixel_format_masks {
            *v = r.u32()?;
        }
        let caps = [r.u32()?, r.u32()?, r.u32()?, r.u32()?];
        let unk_reserved2 = r.u32()?;
        if width == 0 || height == 0 {
            return Err(Error::invalid(format!("DDS size {width}×{height}")));
        }
        if mip_count > 1 {
            return Err(Error::invalid(format!("DDS with {mip_count} levels (the disc has one)")));
        }
        let level0 = format.level_size(width, height);
        let available = data.len() - HEADER_SIZE;
        if available < level0 {
            return Err(Error::UnexpectedEof { offset: HEADER_SIZE, wanted: level0, available });
        }
        Ok(DdsFile {
            flags,
            width,
            height,
            pitch_or_linear_size,
            depth,
            mip_count,
            unk_reserved1,
            pixel_format_flags,
            format,
            unk_pixel_format_masks,
            caps,
            unk_reserved2,
            unk_trailing: available - level0,
        })
    }

    pub fn level0<'a>(&self, data: &'a [u8]) -> &'a [u8] {
        &data[HEADER_SIZE..HEADER_SIZE + self.format.level_size(self.width, self.height)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(fourcc: &[u8; 4], w: u32, h: u32, extra: usize) -> Vec<u8> {
        let mut d = b"DDS ".to_vec();
        let mut put = |v: u32| d.extend_from_slice(&v.to_le_bytes());
        for v in [124, 0x1007, h, w, 0, 0, 0] {
            put(v);
        }
        for _ in 0..11 {
            put(0);
        }
        put(32);
        put(4);
        d.extend_from_slice(fourcc);
        for _ in 0..10 {
            d.extend_from_slice(&0u32.to_le_bytes());
        }
        let fmt = if fourcc == b"DXT1" { DdsFormat::Dxt1 } else { DdsFormat::Dxt5 };
        d.resize(HEADER_SIZE + fmt.level_size(w, h) + extra, 0);
        d
    }

    #[test]
    fn parses_npot_dxt_and_trailing_bytes() {
        let d = header(b"DXT1", 6, 5, 8);
        let f = DdsFile::parse(&d).unwrap();
        assert_eq!((f.width, f.height, f.format), (6, 5, DdsFormat::Dxt1));
        assert_eq!(f.level0(&d).len(), 2 * 2 * 8);
        assert_eq!(f.unk_trailing, 8);
        let mut short = header(b"DXT5", 4, 4, 0);
        short.pop();
        assert!(DdsFile::parse(&short).is_err());
        assert!(DdsFile::parse(&header(b"ATI2", 4, 4, 0)).is_err());
    }
}
