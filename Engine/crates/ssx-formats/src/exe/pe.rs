use crate::reader::Reader;
use crate::{Error, Result};

pub const MACHINE_POWERPC_BE: u16 = 0x01F2;
pub const OPTIONAL_MAGIC_PE32: u16 = 0x010B;
pub const SUBSYSTEM_XBOX: u16 = 14;
pub const SCN_CNT_CODE: u32 = 0x0000_0020;
pub const SCN_MEM_DISCARDABLE: u32 = 0x0200_0000;
pub const SCN_MEM_EXECUTE: u32 = 0x2000_0000;

#[derive(Clone, Debug)]
pub struct Section {
    pub name: String,
    pub virtual_address: u32,
    pub virtual_size: u32,
    pub raw_size: u32,
    pub raw_offset: u32,
    pub characteristics: u32,
}

impl Section {
    pub fn is_executable(&self) -> bool {
        self.characteristics & (SCN_MEM_EXECUTE | SCN_CNT_CODE) != 0
    }

    pub fn is_discardable(&self) -> bool {
        self.characteristics & SCN_MEM_DISCARDABLE != 0
    }

    pub fn contains_rva(&self, rva: u32) -> bool {
        rva >= self.virtual_address && rva - self.virtual_address < self.virtual_size
    }
}

#[derive(Clone, Debug)]
pub struct PeImage {
    pub machine: u16,
    pub characteristics: u16,
    pub timestamp: u32,
    pub optional_magic: u16,
    pub entry_rva: u32,
    pub image_base: u32,
    pub section_alignment: u32,
    pub size_of_image: u32,
    pub size_of_headers: u32,
    pub subsystem: u16,
    pub sections: Vec<Section>,
}

impl PeImage {
    pub fn parse(image: &[u8]) -> Result<Self> {
        let mut r = Reader::le(image);
        r.expect(b"MZ")?;
        r.seek(0x3C)?;
        let nt = r.u32()? as usize;
        r.seek(nt)?;
        r.expect(b"PE\0\0")?;
        let machine = r.u16()?;
        let section_count = r.u16()? as usize;
        let timestamp = r.u32()?;
        r.skip(8)?;
        let optional_size = r.u16()? as usize;
        let characteristics = r.u16()?;
        let optional = r.pos();
        let optional_magic = r.u16()?;
        r.seek(optional + 16)?;
        let entry_rva = r.u32()?;
        r.seek(optional + 28)?;
        let image_base = r.u32()?;
        let section_alignment = r.u32()?;
        r.seek(optional + 56)?;
        let size_of_image = r.u32()?;
        let size_of_headers = r.u32()?;
        r.skip(4)?;
        let subsystem = r.u16()?;

        r.seek(optional + optional_size)?;
        let mut sections = Vec::with_capacity(section_count);
        for _ in 0..section_count {
            let name = r.fixed_str(8)?;
            let virtual_size = r.u32()?;
            let virtual_address = r.u32()?;
            let raw_size = r.u32()?;
            let raw_offset = r.u32()?;
            r.skip(12)?;
            let characteristics = r.u32()?;
            sections.push(Section { name, virtual_address, virtual_size, raw_size, raw_offset, characteristics });
        }
        Ok(Self {
            machine,
            characteristics,
            timestamp,
            optional_magic,
            entry_rva,
            image_base,
            section_alignment,
            size_of_image,
            size_of_headers,
            subsystem,
            sections,
        })
    }

    pub fn validate(&self, image_len: usize, entry_point: u32) -> Result<()> {
        let fail = |msg: String| Err(Error::invalid(format!("PE image: {msg}")));
        if self.machine != MACHINE_POWERPC_BE {
            return fail(format!("machine {:#06x} is not PowerPC big-endian", self.machine));
        }
        if self.optional_magic != OPTIONAL_MAGIC_PE32 || self.subsystem != SUBSYSTEM_XBOX {
            return fail(format!("optional magic {:#x} / subsystem {}", self.optional_magic, self.subsystem));
        }
        for s in &self.sections {
            let start = s.virtual_address as u64;
            let end = start + s.virtual_size.max(s.raw_size) as u64;
            if start >= image_len as u64 || (end > image_len as u64 && !s.is_discardable()) {
                return fail(format!("section {} ({start:#x}..{end:#x}) is outside the image", s.name));
            }
        }
        let entry = entry_point.wrapping_sub(self.image_base);
        if !self.sections.iter().any(|s| s.is_executable() && s.contains_rva(entry)) {
            return fail(format!("entry point {entry_point:#x} is not inside an executable section"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(buf: &mut [u8], at: usize, bytes: &[u8]) {
        buf[at..at + bytes.len()].copy_from_slice(bytes);
    }

    fn image(reloc_characteristics: u32) -> Vec<u8> {
        let mut img = vec![0u8; 0x3000];
        put(&mut img, 0, b"MZ");
        put(&mut img, 0x3C, &0x80u32.to_le_bytes());
        put(&mut img, 0x80, b"PE\0\0");
        put(&mut img, 0x84, &MACHINE_POWERPC_BE.to_le_bytes());
        put(&mut img, 0x86, &2u16.to_le_bytes());
        put(&mut img, 0x88, &0x1234_5678u32.to_le_bytes());
        put(&mut img, 0x94, &0xE0u16.to_le_bytes());
        put(&mut img, 0x96, &0x0102u16.to_le_bytes());
        let opt = 0x98;
        put(&mut img, opt, &OPTIONAL_MAGIC_PE32.to_le_bytes());
        put(&mut img, opt + 16, &0x1010u32.to_le_bytes());
        put(&mut img, opt + 28, &0x8200_0000u32.to_le_bytes());
        put(&mut img, opt + 32, &0x1000u32.to_le_bytes());
        put(&mut img, opt + 56, &0x4000u32.to_le_bytes());
        put(&mut img, opt + 60, &0x400u32.to_le_bytes());
        put(&mut img, opt + 68, &SUBSYSTEM_XBOX.to_le_bytes());
        let sections = [
            (b".text\0\0\0", 0x1000u32, 0x800u32, 0x6000_0020u32),
            (b".reloc\0\0", 0x2000, 0x2000, reloc_characteristics),
        ];
        for (i, (name, va, size, characteristics)) in sections.into_iter().enumerate() {
            let at = opt + 0xE0 + 40 * i;
            put(&mut img, at, name);
            put(&mut img, at + 8, &size.to_le_bytes());
            put(&mut img, at + 12, &va.to_le_bytes());
            put(&mut img, at + 36, &characteristics.to_le_bytes());
        }
        img
    }

    #[test]
    fn parses_and_validates() {
        let img = image(0x4200_0040);
        let pe = PeImage::parse(&img).unwrap();
        assert_eq!((pe.machine, pe.timestamp, pe.entry_rva, pe.image_base), (0x01F2, 0x1234_5678, 0x1010, 0x8200_0000));
        assert_eq!((pe.section_alignment, pe.size_of_image, pe.subsystem), (0x1000, 0x4000, SUBSYSTEM_XBOX));
        let names: Vec<_> = pe.sections.iter().map(|s| (s.name.as_str(), s.virtual_address, s.virtual_size)).collect();
        assert_eq!(names, [(".text", 0x1000, 0x800), (".reloc", 0x2000, 0x2000)]);
        pe.validate(img.len(), 0x8200_1010).unwrap();
        assert!(pe.validate(img.len(), 0x8200_2010).is_err());
        assert!(PeImage::parse(&image(0x4000_0040)).unwrap().validate(img.len(), 0x8200_1010).is_err());
        let mut other = img.clone();
        put(&mut other, 0x84, &0x014Cu16.to_le_bytes());
        assert!(PeImage::parse(&other).unwrap().validate(img.len(), 0x8200_1010).is_err());
        assert!(PeImage::parse(&img[..0x100]).is_err());
    }
}
