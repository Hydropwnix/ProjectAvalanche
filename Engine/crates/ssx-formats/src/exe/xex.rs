use sha1::{Digest, Sha1};

use crate::lzx::LzxDecoder;
use crate::reader::Reader;
use crate::{Error, Result};

pub const MAGIC: &[u8; 4] = b"XEX2";

pub const RETAIL_KEY: [u8; 16] =
    [0x20, 0xB1, 0x85, 0xA5, 0x9D, 0x28, 0xFD, 0xC3, 0x40, 0x58, 0x3F, 0xBB, 0x08, 0x96, 0xBF, 0x91];

pub const KEY_RESOURCE_INFO: u32 = 0x0000_02FF;
pub const KEY_FILE_FORMAT_INFO: u32 = 0x0000_03FF;
pub const KEY_ENTRY_POINT: u32 = 0x0001_0100;
pub const KEY_IMAGE_BASE_ADDRESS: u32 = 0x0001_0201;
pub const KEY_IMPORT_LIBRARIES: u32 = 0x0001_03FF;
pub const KEY_ORIGINAL_PE_NAME: u32 = 0x0001_83FF;

pub const ENCRYPTION_NONE: u16 = 0;
pub const ENCRYPTION_NORMAL: u16 = 1;
pub const COMPRESSION_NORMAL: u16 = 2;

pub const IMAGE_FLAG_PAGE_SIZE_4KB: u32 = 0x1000_0000;

pub const PAGE_DESCRIPTOR_SIZE: usize = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OptHeader {
    pub key: u32,
    pub value: u32,
}

#[derive(Clone, Debug)]
pub struct XexHeader {
    pub module_flags: u32,
    pub header_size: u32,
    pub unk_reserved: u32,
    pub security_offset: u32,
    pub opt_headers: Vec<OptHeader>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockInfo {
    pub size: u32,
    pub sha1: [u8; 20],
}

#[derive(Clone, Debug)]
pub enum Compression {
    Normal { window_size: u32, first_block: BlockInfo },
    Other { kind: u16, raw: Vec<u8> },
}

#[derive(Clone, Debug)]
pub struct FileFormatInfo {
    pub encryption: u16,
    pub compression: Compression,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageDescriptor {
    pub section: u8,
    pub page_count: u32,
    pub digest: [u8; 20],
}

#[derive(Clone, Debug)]
pub struct SecurityInfo {
    pub header_size: u32,
    pub image_size: u32,
    pub rsa_signature: [u8; 0x100],
    pub info_size: u32,
    pub image_flags: u32,
    pub load_address: u32,
    pub section_digest: [u8; 20],
    pub import_table_count: u32,
    pub import_table_digest: [u8; 20],
    pub xgd2_media_id: [u8; 16],
    pub aes_key: [u8; 16],
    pub export_table: u32,
    pub header_digest: [u8; 20],
    pub region: u32,
    pub allowed_media_types: u32,
    pub page_descriptors: Vec<PageDescriptor>,
    pub page_descriptors_offset: usize,
}

impl SecurityInfo {
    pub fn page_size(&self) -> u32 {
        if self.image_flags & IMAGE_FLAG_PAGE_SIZE_4KB != 0 { 0x1000 } else { 0x1_0000 }
    }
}

#[derive(Clone, Debug)]
pub struct ImportLibrary {
    pub name: String,
    pub id: u32,
    pub version: u32,
    pub version_min: u32,
    pub next_import_digest: [u8; 20],
    pub record_addresses: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resource {
    pub name: String,
    pub address: u32,
    pub size: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImportRecord {
    pub address: u32,
    pub kind: u8,
    pub library_index: u8,
    pub ordinal: u16,
}

#[derive(Clone, Debug)]
pub struct Xex<'a> {
    data: &'a [u8],
    pub header: XexHeader,
    pub security: SecurityInfo,
}

impl<'a> Xex<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        let mut r = Reader::be(data);
        r.expect(MAGIC)?;
        let module_flags = r.u32()?;
        let header_size = r.u32()?;
        let unk_reserved = r.u32()?;
        let security_offset = r.u32()?;
        let count = r.u32()? as usize;
        if count > (data.len() - 0x18) / 8 {
            return Err(Error::invalid(format!("XEX optional header count {count} exceeds the file")));
        }
        let opt_headers =
            (0..count).map(|_| Ok(OptHeader { key: r.u32()?, value: r.u32()? })).collect::<Result<_>>()?;
        if header_size as usize > data.len() {
            return Err(Error::invalid(format!("XEX header size {header_size:#x} exceeds the file")));
        }
        let header = XexHeader { module_flags, header_size, unk_reserved, security_offset, opt_headers };
        let security = Self::parse_security(data, security_offset as usize)?;
        Ok(Self { data, header, security })
    }

    fn parse_security(data: &[u8], offset: usize) -> Result<SecurityInfo> {
        let mut r = Reader::be(data);
        r.seek(offset)?;
        let mut s = SecurityInfo {
            header_size: r.u32()?,
            image_size: r.u32()?,
            rsa_signature: r.array()?,
            info_size: r.u32()?,
            image_flags: r.u32()?,
            load_address: r.u32()?,
            section_digest: r.array()?,
            import_table_count: r.u32()?,
            import_table_digest: r.array()?,
            xgd2_media_id: r.array()?,
            aes_key: r.array()?,
            export_table: r.u32()?,
            header_digest: r.array()?,
            region: r.u32()?,
            allowed_media_types: r.u32()?,
            page_descriptors: Vec::new(),
            page_descriptors_offset: 0,
        };
        let count = r.u32()? as usize;
        s.page_descriptors_offset = r.pos();
        if count > r.remaining() / PAGE_DESCRIPTOR_SIZE {
            return Err(Error::invalid(format!("XEX page descriptor count {count} exceeds the file")));
        }
        for _ in 0..count {
            let value = r.u32()?;
            let digest = r.array()?;
            s.page_descriptors.push(PageDescriptor { section: (value & 0xF) as u8, page_count: value >> 4, digest });
        }
        Ok(s)
    }

    pub fn data(&self) -> &'a [u8] {
        self.data
    }

    pub fn payload(&self) -> &'a [u8] {
        &self.data[self.header.header_size as usize..]
    }

    pub fn opt_header(&self, key: u32) -> Option<OptHeader> {
        self.header.opt_headers.iter().copied().find(|h| h.key == key)
    }

    pub fn opt_data(&self, key: u32) -> Result<Option<&'a [u8]>> {
        let Some(index) = self.header.opt_headers.iter().position(|h| h.key == key) else {
            return Ok(None);
        };
        let value = self.header.opt_headers[index].value as usize;
        let mut r = Reader::be(self.data);
        let bytes = match key & 0xFF {
            0x00 | 0x01 => {
                r.seek(0x18 + index * 8 + 4)?;
                r.bytes(4)?
            }
            0xFF => {
                r.seek(value)?;
                let size = r.u32()? as usize;
                r.seek(value)?;
                r.bytes(size)?
            }
            n => {
                r.seek(value)?;
                r.bytes(n as usize * 4)?
            }
        };
        Ok(Some(bytes))
    }

    pub fn opt_value(&self, key: u32) -> Option<u32> {
        self.opt_header(key).filter(|h| h.key & 0xFF <= 1).map(|h| h.value)
    }

    pub fn entry_point(&self) -> Option<u32> {
        self.opt_value(KEY_ENTRY_POINT)
    }

    pub fn image_base(&self) -> Option<u32> {
        self.opt_value(KEY_IMAGE_BASE_ADDRESS)
    }

    pub fn original_pe_name(&self) -> Result<Option<String>> {
        let Some(data) = self.opt_data(KEY_ORIGINAL_PE_NAME)? else { return Ok(None) };
        let mut r = Reader::be(data);
        r.skip(4)?;
        Ok(Some(r.fixed_str(r.remaining())?))
    }
}

impl<'a> Xex<'a> {
    pub fn file_format_info(&self) -> Result<FileFormatInfo> {
        let data = self.opt_data(KEY_FILE_FORMAT_INFO)?.ok_or_else(|| Error::invalid("XEX has no file-format info"))?;
        let mut r = Reader::be(data);
        r.skip(4)?;
        let encryption = r.u16()?;
        let kind = r.u16()?;
        let compression = match kind {
            COMPRESSION_NORMAL => Compression::Normal {
                window_size: r.u32()?,
                first_block: BlockInfo { size: r.u32()?, sha1: r.array()? },
            },
            _ => Compression::Other { kind, raw: r.bytes(r.remaining())?.to_vec() },
        };
        Ok(FileFormatInfo { encryption, compression })
    }

    pub fn import_libraries(&self) -> Result<Vec<ImportLibrary>> {
        let Some(data) = self.opt_data(KEY_IMPORT_LIBRARIES)? else { return Ok(Vec::new()) };
        let mut r = Reader::be(data);
        r.skip(4)?;
        let table_size = r.u32()? as usize;
        let count = r.u32()? as usize;
        let mut table = Reader::be(r.bytes(table_size)?);
        let mut names = Vec::with_capacity(count.min(64));
        for _ in 0..count {
            names.push(table.cstr()?);
            table.align(4)?;
        }
        let mut libraries = Vec::new();
        while r.remaining() > 0 {
            let start = r.pos();
            let size = r.u32()? as usize;
            if size < 0x28 {
                return Err(Error::invalid(format!("XEX import library at {start:#x} has size {size:#x}")));
            }
            let next_import_digest = r.array()?;
            let (id, version, version_min) = (r.u32()?, r.u32()?, r.u32()?);
            let name_index = r.u16()? as usize;
            let records = r.u16()? as usize;
            let record_addresses = (0..records).map(|_| r.u32()).collect::<Result<_>>()?;
            if r.pos() != start + size {
                return Err(Error::invalid(format!("XEX import library at {start:#x}: size {size:#x} != contents")));
            }
            let name = names
                .get(name_index)
                .cloned()
                .ok_or_else(|| Error::invalid(format!("XEX import library name index {name_index} out of range")))?;
            libraries.push(ImportLibrary { name, id, version, version_min, next_import_digest, record_addresses });
        }
        Ok(libraries)
    }

    pub fn resources(&self) -> Result<Vec<Resource>> {
        let Some(data) = self.opt_data(KEY_RESOURCE_INFO)? else { return Ok(Vec::new()) };
        if data.len() < 4 || !(data.len() - 4).is_multiple_of(16) {
            return Err(Error::invalid(format!("XEX resource info of {} bytes is not 4 + 16n", data.len())));
        }
        let mut r = Reader::be(&data[4..]);
        (0..(data.len() - 4) / 16)
            .map(|_| Ok(Resource { name: r.fixed_str(8)?, address: r.u32()?, size: r.u32()? }))
            .collect()
    }

    pub fn resource_data<'i>(&self, resource: &Resource, image: &'i [u8]) -> Result<&'i [u8]> {
        let mut r = Reader::be(image);
        r.seek(resource.address.wrapping_sub(self.security.load_address) as usize)?;
        r.bytes(resource.size as usize)
    }

    pub fn session_key(&self, master: &[u8; 16]) -> [u8; 16] {
        let mut key = self.security.aes_key;
        aes_cbc_decrypt(master, &mut key);
        key
    }
}

fn aes_cbc_decrypt(key: &[u8; 16], data: &mut [u8]) {
    use aes::cipher::{BlockDecrypt, KeyInit};
    let cipher = aes::Aes128::new(key.into());
    let mut prev = [0u8; 16];
    for block in data.as_chunks_mut::<16>().0 {
        let ct = *block;
        cipher.decrypt_block(block.into());
        block.iter_mut().zip(&prev).for_each(|(b, p)| *b ^= p);
        prev = ct;
    }
}

#[derive(Clone, Debug)]
pub struct Image {
    pub data: Vec<u8>,
    pub blocks: usize,
    pub frames: usize,
}

pub fn deblock(payload: &[u8], first: BlockInfo) -> Result<(Vec<&[u8]>, usize)> {
    let mut chunks = Vec::new();
    let mut r = Reader::be(payload);
    let mut info = first;
    let mut blocks = 0;
    while info.size != 0 {
        let start = r.pos();
        let block = r.bytes(info.size as usize)?;
        let digest: [u8; 20] = Sha1::digest(block).into();
        if digest != info.sha1 {
            return Err(Error::invalid(format!(
                "XEX payload block {blocks} at {start:#x}: SHA-1 mismatch (wrong key or corrupt file)"
            )));
        }
        let mut b = Reader::be(block);
        let next = BlockInfo { size: b.u32()?, sha1: b.array()? };
        loop {
            let size = b.u16()? as usize;
            if size == 0 {
                break;
            }
            chunks.push(b.bytes(size)?);
        }
        info = next;
        blocks += 1;
    }
    Ok((chunks, blocks))
}

const FRAME_SIZE: usize = 0x8000;

impl<'a> Xex<'a> {
    pub fn image(&self, master: &[u8; 16]) -> Result<Image> {
        let info = self.file_format_info()?;
        let mut payload = self.payload().to_vec();
        match info.encryption {
            ENCRYPTION_NONE => {}
            ENCRYPTION_NORMAL => {
                if !payload.len().is_multiple_of(16) {
                    return Err(Error::invalid(format!(
                        "XEX payload size {:#x} is not a multiple of 16",
                        payload.len()
                    )));
                }
                aes_cbc_decrypt(&self.session_key(master), &mut payload);
            }
            other => return Err(Error::invalid(format!("unsupported XEX encryption type {other}"))),
        }
        let Compression::Normal { window_size, first_block } = info.compression else {
            return Err(Error::invalid("unsupported XEX compression type (only normal/LZX is on the disc)"));
        };
        if !window_size.is_power_of_two() {
            return Err(Error::invalid(format!("XEX LZX window size {window_size:#x} is not a power of two")));
        }
        let (chunks, blocks) = deblock(&payload, first_block)?;
        let size = self.security.image_size as usize;
        let mut data = Vec::with_capacity(size);
        let mut lzx = LzxDecoder::new(window_size.trailing_zeros())?;
        for (i, chunk) in chunks.iter().enumerate() {
            let out_len = (size - data.len()).min(FRAME_SIZE);
            if out_len == 0 {
                return Err(Error::invalid(format!(
                    "XEX payload has {} chunks, image is full after {i}",
                    chunks.len()
                )));
            }
            lzx.decode_frame(chunk, out_len, &mut data).map_err(|e| match e {
                Error::Decompress(msg) => Error::Decompress(format!("{msg} (XEX chunk {i})")),
                other => other,
            })?;
        }
        if data.len() != size {
            return Err(Error::invalid(format!(
                "XEX payload decoded to {:#x} bytes, image size is {size:#x}",
                data.len()
            )));
        }
        Ok(Image { data, blocks, frames: chunks.len() })
    }
}

impl<'a> Xex<'a> {
    pub fn verify_page_digests(&self, image: &[u8]) -> Result<u32> {
        let s = &self.security;
        let page_size = s.page_size() as usize;
        let raw = |i: usize| {
            let start = s.page_descriptors_offset + i * PAGE_DESCRIPTOR_SIZE;
            &self.data[start..start + PAGE_DESCRIPTOR_SIZE]
        };
        let mut expected = s.section_digest;
        let mut page = 0usize;
        for (i, d) in s.page_descriptors.iter().enumerate() {
            let len = d.page_count as usize * page_size;
            let run = image.get(page * page_size..page * page_size + len).ok_or_else(|| {
                Error::invalid(format!("XEX page descriptor {i} reaches beyond the {:#x}-byte image", image.len()))
            })?;
            let mut h = Sha1::new();
            h.update(run);
            h.update(raw(i));
            if <[u8; 20]>::from(h.finalize()) != expected {
                return Err(Error::invalid(format!("XEX page digest mismatch at descriptor {i} (page {page})")));
            }
            expected = d.digest;
            page += d.page_count as usize;
        }
        if expected != [0; 20] {
            return Err(Error::invalid("XEX last page descriptor has a non-zero digest"));
        }
        Ok(page as u32)
    }

    pub fn import_records(&self, libraries: &[ImportLibrary], image: &[u8]) -> Result<Vec<Vec<ImportRecord>>> {
        let base = self.security.load_address;
        libraries
            .iter()
            .map(|lib| {
                lib.record_addresses
                    .iter()
                    .map(|&address| {
                        let mut r = Reader::be(image);
                        r.seek(address.wrapping_sub(base) as usize)?;
                        let value = r.u32()?;
                        Ok(ImportRecord {
                            address,
                            kind: (value >> 24) as u8,
                            library_index: (value >> 16) as u8,
                            ordinal: value as u16,
                        })
                    })
                    .collect()
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEC: usize = 0x100;
    const PAGE: usize = 0x1000;
    const BASE: u32 = 0x8200_0000;

    fn put(buf: &mut Vec<u8>, at: usize, bytes: &[u8]) {
        if buf.len() < at + bytes.len() {
            buf.resize(at + bytes.len(), 0);
        }
        buf[at..at + bytes.len()].copy_from_slice(bytes);
    }

    fn be(v: u32) -> [u8; 4] {
        v.to_be_bytes()
    }

    fn synthetic(image: &[u8], runs: &[(u8, u32)]) -> Vec<u8> {
        let mut x = vec![0u8; 0x1000];
        put(&mut x, 0, b"XEX2");
        for (i, v) in [1, 0x1000, 0, SEC as u32, 7].into_iter().enumerate() {
            put(&mut x, 4 + 4 * i, &be(v));
        }
        let opts = [
            (KEY_ENTRY_POINT, BASE + 0x1234),
            (KEY_IMAGE_BASE_ADDRESS, BASE),
            (KEY_ORIGINAL_PE_NAME, 0x400),
            (KEY_FILE_FORMAT_INFO, 0x420),
            (0x0004_0006, 0x460),
            (KEY_RESOURCE_INFO, 0x480),
            (KEY_IMPORT_LIBRARIES, 0x500),
        ];
        for (i, (key, value)) in opts.into_iter().enumerate() {
            put(&mut x, 0x18 + 8 * i, &be(key));
            put(&mut x, 0x1C + 8 * i, &be(value));
        }

        put(&mut x, SEC, &be(0x1000));
        put(&mut x, SEC + 4, &be(image.len() as u32));
        put(&mut x, SEC + 0x108, &be(0x184));
        put(&mut x, SEC + 0x10C, &be(IMAGE_FLAG_PAGE_SIZE_4KB));
        put(&mut x, SEC + 0x110, &be(BASE));
        put(&mut x, SEC + 0x150, &[0xAA; 16]);
        put(&mut x, SEC + 0x180, &be(runs.len() as u32));
        let desc = SEC + 0x184;
        let (mut digest, mut page) = ([0u8; 20], image.len() / PAGE);
        for (i, &(section, count)) in runs.iter().enumerate().rev() {
            let at = desc + PAGE_DESCRIPTOR_SIZE * i;
            put(&mut x, at, &be(count << 4 | section as u32));
            put(&mut x, at + 4, &digest);
            page -= count as usize;
            let mut h = Sha1::new();
            h.update(&image[page * PAGE..(page + count as usize) * PAGE]);
            h.update(&x[at..at + PAGE_DESCRIPTOR_SIZE]);
            digest = h.finalize().into();
        }
        put(&mut x, SEC + 0x114, &digest);

        put(&mut x, 0x400, &be(0x10));
        put(&mut x, 0x404, b"game.exe\0");
        put(&mut x, 0x420, &be(0x24));
        put(&mut x, 0x424, &[0, 1, 0, 2]);
        put(&mut x, 0x428, &be(0x8000));
        put(&mut x, 0x42C, &be(0x1234));
        put(&mut x, 0x430, &[7; 20]);
        put(&mut x, 0x460, &[0x11; 24]);
        put(&mut x, 0x480, &be(20));
        put(&mut x, 0x484, b"res1");
        put(&mut x, 0x48C, &be(BASE + 0x100));
        put(&mut x, 0x490, &be(0x10));

        put(&mut x, 0x500, &be(0x80));
        put(&mut x, 0x504, &be(24));
        put(&mut x, 0x508, &be(2));
        put(&mut x, 0x50C, b"xboxkrnl.exe\0");
        put(&mut x, 0x51C, b"xam.xex\0");
        let lib = 0x524;
        put(&mut x, lib, &be(0x30));
        put(&mut x, lib + 0x18, &be(0xAB));
        put(&mut x, lib + 0x24, &[0, 1, 0, 2]);
        put(&mut x, lib + 0x28, &be(BASE + 0x10));
        put(&mut x, lib + 0x2C, &be(BASE + 0x14));
        let lib = lib + 0x30;
        put(&mut x, lib, &be(0x2C));
        put(&mut x, lib + 0x24, &[0, 0, 0, 1]);
        put(&mut x, lib + 0x28, &be(BASE + 0x20));
        x
    }

    fn test_image() -> Vec<u8> {
        let mut image: Vec<u8> = (0..4 * PAGE).map(|i| (i * 7 % 251) as u8).collect();
        put(&mut image, 0x10, &be(0x0001_0005));
        put(&mut image, 0x14, &be(0x0101_0005));
        put(&mut image, 0x20, &be(0x0000_0123));
        image
    }

    const RUNS: [(u8, u32); 3] = [(3, 1), (1, 2), (2, 1)];

    #[test]
    fn parses_headers() {
        let file = synthetic(&test_image(), &RUNS);
        let x = Xex::parse(&file).unwrap();
        assert_eq!((x.header.module_flags, x.header.header_size, x.header.opt_headers.len()), (1, 0x1000, 7));
        assert_eq!((x.entry_point(), x.image_base()), (Some(BASE + 0x1234), Some(BASE)));
        assert_eq!(x.opt_data(KEY_ENTRY_POINT).unwrap().unwrap(), &be(BASE + 0x1234));
        assert_eq!(x.opt_data(0x0004_0006).unwrap().unwrap(), &[0x11; 24]);
        assert_eq!(x.opt_data(0x0004_0310).unwrap(), None);
        assert_eq!(x.original_pe_name().unwrap().as_deref(), Some("game.exe"));
        let info = x.file_format_info().unwrap();
        assert_eq!(info.encryption, ENCRYPTION_NORMAL);
        let Compression::Normal { window_size, first_block } = info.compression else { panic!("not normal") };
        assert_eq!((window_size, first_block), (0x8000, BlockInfo { size: 0x1234, sha1: [7; 20] }));
        let s = &x.security;
        assert_eq!((s.image_size, s.load_address, s.page_size(), s.aes_key), (0x4000, BASE, 0x1000, [0xAA; 16]));
        let runs: Vec<_> = s.page_descriptors.iter().map(|d| (d.section, d.page_count)).collect();
        assert_eq!(runs, RUNS);
        assert_eq!(x.resources().unwrap(), [Resource { name: "res1".into(), address: BASE + 0x100, size: 0x10 }]);
    }

    #[test]
    fn import_libraries_and_records() {
        let image = test_image();
        let file = synthetic(&image, &RUNS);
        let x = Xex::parse(&file).unwrap();
        let libs = x.import_libraries().unwrap();
        let names: Vec<_> = libs.iter().map(|l| (l.name.as_str(), l.id, l.record_addresses.len())).collect();
        assert_eq!(names, [("xam.xex", 0xAB, 2), ("xboxkrnl.exe", 0, 1)]);
        let records = x.import_records(&libs, &image).unwrap();
        let rec = |address, kind, library_index, ordinal| ImportRecord { address, kind, library_index, ordinal };
        assert_eq!(records[0], [rec(BASE + 0x10, 0, 1, 5), rec(BASE + 0x14, 1, 1, 5)]);
        assert_eq!(records[1], [rec(BASE + 0x20, 0, 0, 0x123)]);
        let res = &x.resources().unwrap()[0];
        assert_eq!(x.resource_data(res, &image).unwrap(), &image[0x100..0x110]);
    }

    #[test]
    fn page_digest_chain() {
        let mut image = test_image();
        let file = synthetic(&image, &RUNS);
        let x = Xex::parse(&file).unwrap();
        assert_eq!(x.verify_page_digests(&image).unwrap(), 4);
        image[2 * PAGE + 5] ^= 1;
        let err = x.verify_page_digests(&image).unwrap_err().to_string();
        assert!(err.contains("descriptor 1 (page 1)"), "{err}");
        assert!(x.verify_page_digests(&image[..3 * PAGE]).is_err());
    }

    fn block(next: Option<&[u8]>, chunks: &[&[u8]], pad: usize) -> Vec<u8> {
        let mut b = Vec::new();
        match next {
            Some(next) => {
                b.extend_from_slice(&be(next.len() as u32));
                b.extend_from_slice(&<[u8; 20]>::from(Sha1::digest(next)));
            }
            None => b.extend_from_slice(&[0; 24]),
        }
        for c in chunks {
            b.extend_from_slice(&(c.len() as u16).to_be_bytes());
            b.extend_from_slice(c);
        }
        b.extend_from_slice(&[0; 2]);
        b.resize(b.len() + pad, 0);
        b
    }

    #[test]
    fn deblock_checks_every_block() {
        let second = block(None, &[b"cc"], 6);
        let first = block(Some(&second), &[b"aaaa", b"bbb"], 3);
        let info = BlockInfo { size: first.len() as u32, sha1: Sha1::digest(&first).into() };
        let mut payload = [first.clone(), second, vec![0; 32]].concat();
        let (chunks, blocks) = deblock(&payload, info).unwrap();
        assert_eq!((chunks, blocks), (vec![&b"aaaa"[..], b"bbb", b"cc"], 2));

        payload[first.len() + 27] ^= 1;
        let err = deblock(&payload, info).unwrap_err().to_string();
        assert!(err.contains("block 1") && err.contains("SHA-1"), "{err}");
        assert!(deblock(&payload[..10], info).is_err());
    }

    #[test]
    fn aes_cbc_decryption() {
        let key: [u8; 16] = *b"\x2b\x7e\x15\x16\x28\xae\xd2\xa6\xab\xf7\x15\x88\x09\xcf\x4f\x3c";
        let mut data = *b"\x76\x49\xab\xac\x81\x19\xb2\x46\xce\xe9\x8e\x9b\x12\xe9\x19\x7d\
                          \x50\x86\xcb\x9b\x50\x72\x19\xee\x95\xdb\x11\x3a\x91\x76\x78\xb2";
        aes_cbc_decrypt(&key, &mut data);
        let iv: Vec<u8> = (0..16).collect();
        let p1 = b"\x6b\xc1\xbe\xe2\x2e\x40\x9f\x96\xe9\x3d\x7e\x11\x73\x93\x17\x2a";
        let first: Vec<u8> = p1.iter().zip(&iv).map(|(p, v)| p ^ v).collect();
        assert_eq!(&data[..16], &first[..]);
        assert_eq!(&data[16..], b"\xae\x2d\x8a\x57\x1e\x03\xac\x9c\x9e\xb7\x6f\xac\x45\xaf\x8e\x51");
    }
}
