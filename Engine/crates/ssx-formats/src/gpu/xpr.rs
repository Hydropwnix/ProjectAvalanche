use crate::reader::Reader;
use crate::xenos::{DataDimension, Endian, LINEAR_ROW_ALIGNMENT, LinearVolume, Surface2D, TextureFormat};
use crate::{Error, Result};

pub const MAGIC: &[u8; 4] = b"XPR2";

pub const TEXTURE_DESCRIPTOR_SIZE: u32 = 52;

pub const D3D_RESOURCE_HEADER: [u32; 7] = [3, 1, 0, 0, 0, 0xffff_0000, 0xffff_0000];

#[derive(Clone, Debug)]
pub struct XprFile {
    pub header_size: u32,
    pub data_size: u32,
    pub resources: Vec<XprResource>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ResourceKind {
    Texture2D,
    CubeTexture,
    VolumeTexture,
}

impl ResourceKind {
    pub fn fourcc(self) -> &'static str {
        match self {
            Self::Texture2D => "TX2D",
            Self::CubeTexture => "TXCM",
            Self::VolumeTexture => "TX3D",
        }
    }

    fn dimension(self) -> DataDimension {
        match self {
            Self::Texture2D => DataDimension::D2,
            Self::CubeTexture => DataDimension::Cube,
            Self::VolumeTexture => DataDimension::D3,
        }
    }
}

#[derive(Clone, Debug)]
pub struct XprResource {
    pub kind: ResourceKind,
    pub name: String,
    pub offset: u32,
    pub fetch: TextureFetch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Swizzle {
    X,
    Y,
    Z,
    W,
    Zero,
    One,
}

impl Swizzle {
    fn from_raw(raw: u32) -> Result<Self> {
        Ok(match raw {
            0 => Self::X,
            1 => Self::Y,
            2 => Self::Z,
            3 => Self::W,
            4 => Self::Zero,
            5 => Self::One,
            _ => return Err(Error::invalid(format!("invalid swizzle selector {raw}"))),
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TextureFetch {
    pub raw: [u32; 6],
    pub tiled: bool,
    pub pitch: u32,
    pub format: TextureFormat,
    pub endian: Endian,
    pub base_address: u32,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub swizzle: [Swizzle; 4],
    pub mip_max_level: u32,
    pub dimension: DataDimension,
    pub packed_mips: bool,
    pub mip_address: u32,
}

fn bits(v: u32, shift: u32, width: u32) -> u32 {
    (v >> shift) & ((1 << width) - 1)
}

impl TextureFetch {
    pub fn parse(raw: [u32; 6]) -> Result<Self> {
        let [d0, d1, d2, d3, d4, d5] = raw;
        let fail = |what: &str| Err(Error::invalid(format!("texture fetch constant {raw:08x?}: {what}")));
        if bits(d0, 0, 2) != 2 {
            return fail("type is not texture (2)");
        }
        if bits(d0, 2, 8) != 0 || bits(d3, 0, 1) != 0 {
            return fail("signed or integer components");
        }
        if bits(d1, 10, 1) != 0 {
            return fail("stacked texture");
        }
        if bits(d4, 2, 4) != 0 {
            return fail("mip_min_level is not 0");
        }
        let Some(format) = TextureFormat::from_raw(bits(d1, 0, 6)) else {
            return fail("unsupported texture format");
        };
        let dimension = DataDimension::from_raw(bits(d5, 9, 2));
        let (width, height, depth) = match dimension {
            DataDimension::D3 => (bits(d2, 0, 11) + 1, bits(d2, 11, 11) + 1, bits(d2, 22, 10) + 1),
            DataDimension::D2 | DataDimension::Cube => (bits(d2, 0, 13) + 1, bits(d2, 13, 13) + 1, bits(d2, 26, 6) + 1),
            DataDimension::D1 => return fail("1D texture"),
        };
        let expected_depth = match dimension {
            DataDimension::D2 => Some(1),
            DataDimension::Cube => Some(6),
            _ => None,
        };
        if expected_depth.is_some_and(|d| d != depth) {
            return fail("unexpected stack depth");
        }
        let sel = |i: u32| Swizzle::from_raw(bits(d3, 1 + 3 * i, 3));
        let fetch = Self {
            raw,
            tiled: bits(d0, 31, 1) == 1,
            pitch: bits(d0, 22, 9) << 5,
            format,
            endian: Endian::from_raw(bits(d1, 6, 2)),
            base_address: bits(d1, 12, 20) << 12,
            width,
            height,
            depth,
            swizzle: [sel(0)?, sel(1)?, sel(2)?, sel(3)?],
            mip_max_level: bits(d4, 6, 4),
            dimension,
            packed_mips: bits(d5, 11, 1) == 1,
            mip_address: bits(d5, 12, 20) << 12,
        };
        if fetch.tiled != (dimension != DataDimension::D3) {
            return fail(if fetch.tiled { "tiled 3D texture" } else { "linear 2D texture" });
        }
        if dimension == DataDimension::D3 && fetch.mip_max_level != 0 {
            return fail("3D texture with mips");
        }
        if let Some(s) = fetch.surface_2d()
            && s.packed_level() == Some(0)
            && fetch.mip_max_level != 0
        {
            return fail("mips of a texture whose level 0 fits the packed tail");
        }
        if !fetch.tiled && !((fetch.pitch * format.bytes_per_block()) as usize).is_multiple_of(LINEAR_ROW_ALIGNMENT) {
            return fail("linear row pitch is not 256-byte aligned");
        }
        if fetch.mip_max_level > fetch.width.max(fetch.height).max(depth).ilog2() {
            return fail("more mip levels than the size allows");
        }
        if (fetch.mip_address == 0) != (fetch.mip_max_level == 0) {
            return fail("mip address does not match the mip count");
        }
        Ok(fetch)
    }

    pub fn linear_volume(&self) -> Option<LinearVolume> {
        (self.dimension == DataDimension::D3).then_some(LinearVolume {
            format: self.format,
            width: self.width,
            height: self.height,
            depth: self.depth,
            pitch: self.pitch,
        })
    }

    pub fn memory_size(&self) -> (usize, usize) {
        match (self.surface_2d(), self.linear_volume()) {
            (Some(s), _) => (s.base_size(), s.mips_size()),
            (None, Some(v)) => (v.size(), 0),
            (None, None) => (0, 0),
        }
    }

    pub fn surface_2d(&self) -> Option<Surface2D> {
        let array_size = match self.dimension {
            DataDimension::D2 => 1,
            DataDimension::Cube => 6,
            _ => return None,
        };
        Some(Surface2D {
            format: self.format,
            width: self.width,
            height: self.height,
            base_pitch: self.pitch,
            packed_mips: self.packed_mips,
            max_level: self.mip_max_level,
            array_size,
        })
    }
}

const TABLE_BASE: usize = 0x0C;

fn dwords<const N: usize>(r: &mut Reader) -> Result<[u32; N]> {
    let mut out = [0; N];
    for v in &mut out {
        *v = r.u32()?;
    }
    Ok(out)
}

impl XprFile {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::be(data);
        r.expect(MAGIC)?;
        let header_size = r.u32()?;
        let data_size = r.u32()?;
        if TABLE_BASE as u64 + header_size as u64 + data_size as u64 != data.len() as u64 {
            return Err(Error::invalid(format!(
                "XPR2 header size {header_size:#x} + data size {data_size:#x} + 12 != file size {:#x}",
                data.len()
            )));
        }
        let header = &data[TABLE_BASE..TABLE_BASE + header_size as usize];
        let mut used = vec![false; header.len()];
        let mut claim = |start: usize, len: usize, what: &str| -> Result<()> {
            let span = used.get_mut(start..start + len).ok_or_else(|| {
                Error::invalid(format!(
                    "XPR2 {what} at {start:#x}+{len:#x} is outside the {header_size:#x}-byte header"
                ))
            })?;
            span.fill(true);
            Ok(())
        };

        let mut hr = Reader::be(header);
        let count = hr.u32()?;
        if count == 0 {
            return Err(Error::invalid("XPR2 without resources"));
        }
        claim(0, 4 + 16 * count as usize + 4, "resource table")?;
        let mut resources = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let fourcc = hr.array::<4>()?;
            let (offset, size, name_offset) = (hr.u32()?, hr.u32()?, hr.u32()?);
            let kind = match &fourcc {
                b"TX2D" => ResourceKind::Texture2D,
                b"TXCM" => ResourceKind::CubeTexture,
                b"TX3D" => ResourceKind::VolumeTexture,
                other => return Err(Error::invalid(format!("unknown XPR2 resource type {other:?}"))),
            };
            if size != TEXTURE_DESCRIPTOR_SIZE {
                return Err(Error::invalid(format!("{} descriptor of {size} bytes", kind.fourcc())));
            }
            let mut nr = Reader::be(header);
            nr.seek(name_offset as usize)?;
            let name = nr.cstr()?;
            claim(name_offset as usize, name.len() + 1, "resource name")?;
            claim(offset as usize, size as usize, "resource descriptor")?;

            let mut dr = Reader::be(header);
            dr.seek(offset as usize)?;
            let d3d: [u32; 7] = dwords(&mut dr)?;
            if d3d != D3D_RESOURCE_HEADER {
                return Err(Error::invalid(format!("{name}: unexpected D3D resource header {d3d:08x?}")));
            }
            let raw: [u32; 6] = dwords(&mut dr)?;
            let fetch = TextureFetch::parse(raw).map_err(|e| Error::invalid(format!("{name}: {e}")))?;
            if fetch.dimension != kind.dimension() {
                return Err(Error::invalid(format!("{name}: {} with dimension {:?}", kind.fourcc(), fetch.dimension)));
            }
            let (base_size, mips_size) = fetch.memory_size();
            let base_end = fetch.base_address as u64 + base_size as u64;
            let mips_end = fetch.mip_address as u64 + mips_size as u64;
            if base_end.max(mips_end) > data_size as u64 {
                return Err(Error::invalid(format!(
                    "{name}: texture memory ends at {:#x}, past the {data_size:#x}-byte data section",
                    base_end.max(mips_end)
                )));
            }
            resources.push(XprResource { kind, name, offset, fetch });
        }
        if hr.u32()? != 0 {
            return Err(Error::invalid("XPR2 resource table is not followed by a zero dword"));
        }
        if let Some(at) = header.iter().zip(&used).position(|(&b, &u)| !u && b != 0) {
            return Err(Error::invalid(format!("XPR2 header byte {:#x} is unaccounted for", TABLE_BASE + at)));
        }
        Ok(Self { header_size, data_size, resources })
    }

    pub fn data_offset(&self) -> usize {
        TABLE_BASE + self.header_size as usize
    }

    pub fn data<'a>(&self, file: &'a [u8]) -> &'a [u8] {
        &file[self.data_offset()..]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(fourcc: &[u8; 4], name: &str, fetch: [u32; 6], data: &[u8]) -> Vec<u8> {
        let name_offset = 4 + 16 + 4;
        let descriptor = (name_offset + name.len() + 1).next_multiple_of(16);
        let mut h = 1u32.to_be_bytes().to_vec();
        h.extend(fourcc);
        for v in [descriptor as u32, TEXTURE_DESCRIPTOR_SIZE, name_offset as u32, 0] {
            h.extend(v.to_be_bytes());
        }
        h.extend(name.as_bytes());
        h.resize(descriptor, 0);
        D3D_RESOURCE_HEADER.iter().chain(&fetch).for_each(|v| h.extend(v.to_be_bytes()));
        h.resize(h.len().next_multiple_of(16), 0);
        let mut f = MAGIC.to_vec();
        f.extend((h.len() as u32).to_be_bytes());
        f.extend((data.len() as u32).to_be_bytes());
        f.extend(h);
        f.extend(data);
        f
    }

    const K8_32: [u32; 6] = [0x8040_0002, 0x0000_0002, 0x0003_e01f, 0x0000_1400, 0, 0x0000_0a00];

    #[test]
    fn fetch_constant_fields() {
        let f = TextureFetch::parse([0x8800_0002, 0x54, 0x007f_e3ff, 0xd10, 0x280, 0x0010_0a00]).unwrap();
        assert_eq!(
            (f.format, f.endian, f.width, f.height, f.pitch),
            (TextureFormat::Dxt45, Endian::Swap8In16, 1024, 1024, 1024)
        );
        assert_eq!((f.tiled, f.packed_mips, f.dimension, f.depth), (true, true, DataDimension::D2, 1));
        assert_eq!((f.base_address, f.mip_address, f.mip_max_level), (0, 0x10_0000, 10));
        assert_eq!(f.swizzle, [Swizzle::X, Swizzle::Y, Swizzle::Z, Swizzle::W]);
        assert!(TextureFetch::parse([0x8800_0002, 0x54, 0x007f_e3ff, 0xd10, 0, 0x0010_0a00]).is_err());
        assert!(TextureFetch::parse([0x8800_0006, 0x54, 0x007f_e3ff, 0xd10, 0x280, 0x0010_0a00]).is_err());
        assert!(TextureFetch::parse([0x8800_0002, 0x55, 0x007f_e3ff, 0xd10, 0x280, 0x0010_0a00]).is_err());
    }

    #[test]
    fn parses_a_minimal_file() {
        let file = build(b"TX2D", "tex", K8_32, &[0; 4096]);
        let xpr = XprFile::parse(&file).unwrap();
        assert_eq!(xpr.resources.len(), 1);
        let r = &xpr.resources[0];
        assert_eq!((r.kind, r.name.as_str()), (ResourceKind::Texture2D, "tex"));
        assert_eq!(r.fetch.swizzle, [Swizzle::X, Swizzle::X, Swizzle::X, Swizzle::One]);
        assert_eq!(xpr.data(&file).len(), 4096);
    }

    #[test]
    fn rejects_inconsistent_files() {
        let good = build(b"TX2D", "tex", K8_32, &[0; 4096]);
        let mut junk = good.clone();
        junk[0x24 + 4] = 1;
        assert!(XprFile::parse(&junk).is_err());
        assert!(XprFile::parse(&good[..good.len() - 1]).is_err());
        assert!(XprFile::parse(&build(b"TXCM", "tex", K8_32, &[0; 4096])).is_err());
        assert!(XprFile::parse(&build(b"TX2D", "tex", K8_32, &[0; 2048])).is_err());
        assert!(XprFile::parse(&build(b"TXXX", "tex", K8_32, &[0; 4096])).is_err());
    }
}
