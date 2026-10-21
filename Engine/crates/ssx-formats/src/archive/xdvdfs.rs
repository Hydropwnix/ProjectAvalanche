use std::io::{Read, Seek, SeekFrom};

use crate::reader::Reader;
use crate::{Error, Result};

pub const SECTOR_SIZE: u64 = 0x800;
pub const VOLUME_DESCRIPTOR_SECTOR: u64 = 32;
pub const MAGIC: &[u8; 20] = b"MICROSOFT*XBOX*MEDIA";
const RESERVED_SIZE: usize = 0x7C8;
const ENTRY_HEADER: usize = 14;
const MAX_DIRECTORY_SIZE: u32 = 0x40000;
const PAD: u16 = 0xFFFF;

// TODO(guess): READ_ONLY, HIDDEN, SYSTEM and ARCHIVE are extract-xiso's FAT meanings, never set on the SSX disc (0x80 on 59 files, 0x90 on the 5 directories, 0x00 on `$SystemUpdate/system.manifest`; DIRECTORY is confirmed: exactly the entries whose extent parses as a table); settle: a disc that sets them.
pub mod attr {
    pub const READ_ONLY: u8 = 0x01;
    pub const HIDDEN: u8 = 0x02;
    pub const SYSTEM: u8 = 0x04;
    pub const DIRECTORY: u8 = 0x10;
    pub const ARCHIVE: u8 = 0x20;
    pub const NORMAL: u8 = 0x80;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    GamePartition,
    Xgd1,
    Xgd2,
    Xgd3,
}

impl Layout {
    pub const ALL: [Layout; 4] = [Layout::GamePartition, Layout::Xgd2, Layout::Xgd3, Layout::Xgd1];

    pub fn partition_offset(self) -> u64 {
        // TODO(guess): only the XGD2 offset is verified (the SSX image: the magic at 0xFD90000 + 0x10000, 7 838 695 424 bytes, nothing valid at the XGD3 offset); the trimmed, XGD1 and XGD3 offsets are extract-xiso's; settle: such images.
        match self {
            Layout::GamePartition => 0,
            Layout::Xgd1 => 0x1830_0000,
            Layout::Xgd2 => 0x0FD9_0000,
            Layout::Xgd3 => 0x0208_0000,
        }
    }
}

#[derive(Clone, Debug)]
pub struct VolumeDescriptor {
    pub root_sector: u32,
    pub root_size: u32,
    pub filetime: u64,
    pub unk_reserved: Vec<u8>,
}

impl VolumeDescriptor {
    pub fn parse(sector: &[u8]) -> Result<Self> {
        let mut r = Reader::le(sector);
        r.expect(MAGIC)?;
        let root_sector = r.u32()?;
        let root_size = r.u32()?;
        let filetime = r.u64()?;
        let unk_reserved = r.bytes(RESERVED_SIZE)?.to_vec();
        r.expect(MAGIC)?;
        if r.pos() as u64 != SECTOR_SIZE {
            return Err(Error::invalid("XDVDFS volume descriptor is not one sector"));
        }
        Ok(Self { root_sector, root_size, filetime, unk_reserved })
    }

    pub fn unix_time(&self) -> i64 {
        (self.filetime / 10_000_000) as i64 - 11_644_473_600
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Extent {
    pub sector: u32,
    pub size: u32,
}

impl Extent {
    pub fn offset(self) -> u64 {
        self.sector as u64 * SECTOR_SIZE
    }

    pub fn sectors(self) -> u64 {
        (self.size as u64).div_ceil(SECTOR_SIZE)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirEntry {
    pub offset: u32,
    pub left: u16,
    pub right: u16,
    pub extent: Extent,
    pub attributes: u8,
    pub name: String,
}

impl DirEntry {
    pub fn is_dir(&self) -> bool {
        self.attributes & attr::DIRECTORY != 0
    }

    pub fn parse(table: &[u8], offset: usize) -> Result<Self> {
        let mut r = Reader::le(table);
        r.seek(offset)?;
        let left = r.u16()?;
        let right = r.u16()?;
        let sector = r.u32()?;
        let size = r.u32()?;
        let attributes = r.u8()?;
        let len = r.u8()? as usize;
        let raw = r.bytes(len)?;
        if offset as u64 / SECTOR_SIZE != (offset + ENTRY_HEADER + len - 1) as u64 / SECTOR_SIZE {
            return Err(Error::invalid(format!("XDVDFS entry at {offset:#x} crosses a sector boundary")));
        }
        let valid = |b: &u8| b.is_ascii_graphic() || *b == b' ';
        if len == 0 || !raw.iter().all(|b| valid(b) && *b != b'/' && *b != b'\\') || raw == b"." || raw == b".." {
            return Err(Error::invalid(format!("XDVDFS entry at {offset:#x} has an unusable name {raw:02x?}")));
        }
        let name = String::from_utf8(raw.to_vec()).map_err(|_| Error::invalid("XDVDFS name is not ASCII"))?;
        Ok(Self { offset: offset as u32, left, right, extent: Extent { sector, size }, attributes, name })
    }

    pub fn stored_size(&self) -> usize {
        (ENTRY_HEADER + self.name.len()).next_multiple_of(4)
    }
}

pub fn parse_directory(table: &[u8]) -> Result<Vec<DirEntry>> {
    let mut out = Vec::new();
    if table.len() < 2 || u16::from_le_bytes([table[0], table[1]]) == PAD {
        return Ok(out);
    }
    let mut visited = std::collections::HashSet::new();
    let mut stack = vec![(0usize, false)];
    while let Some((offset, expanded)) = stack.pop() {
        let entry = DirEntry::parse(table, offset)?;
        if expanded {
            if entry.right != 0 {
                stack.push((entry.right as usize * 4, false));
            }
            out.push(entry);
            continue;
        }
        if !visited.insert(offset) {
            return Err(Error::invalid(format!("XDVDFS directory tree revisits offset {offset:#x}")));
        }
        stack.push((offset, true));
        if entry.left != 0 {
            stack.push((entry.left as usize * 4, false));
        }
    }
    Ok(out)
}

pub fn scan_directory(table: &[u8]) -> Result<Vec<DirEntry>> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos + 2 <= table.len() {
        if u16::from_le_bytes([table[pos], table[pos + 1]]) == PAD {
            pos = (pos as u64 / SECTOR_SIZE + 1) as usize * SECTOR_SIZE as usize;
            continue;
        }
        let entry = DirEntry::parse(table, pos)?;
        pos += entry.stored_size();
        out.push(entry);
    }
    Ok(out)
}

#[derive(Clone, Debug)]
pub struct Node {
    pub path: String,
    pub entry: DirEntry,
}

impl Node {
    pub fn is_dir(&self) -> bool {
        self.entry.is_dir()
    }

    pub fn extent(&self) -> Extent {
        self.entry.extent
    }
}

pub struct Disc<R> {
    source: R,
    pub layout: Layout,
    pub volume: VolumeDescriptor,
    pub nodes: Vec<Node>,
    pub image_size: u64,
}

impl<R: Read + Seek> Disc<R> {
    pub fn open(mut source: R) -> Result<Self> {
        let image_size = source.seek(SeekFrom::End(0))?;
        let mut found = None;
        for layout in Layout::ALL {
            let at = layout.partition_offset() + VOLUME_DESCRIPTOR_SECTOR * SECTOR_SIZE;
            if at + SECTOR_SIZE > image_size {
                continue;
            }
            let mut sector = vec![0u8; SECTOR_SIZE as usize];
            source.seek(SeekFrom::Start(at))?;
            source.read_exact(&mut sector)?;
            if let Ok(volume) = VolumeDescriptor::parse(&sector) {
                found = Some((layout, volume));
                break;
            }
        }
        let (layout, volume) = found
            .ok_or_else(|| Error::invalid("no XDVDFS volume descriptor (MICROSOFT*XBOX*MEDIA) at any known offset"))?;
        let mut disc = Disc { source, layout, volume, nodes: Vec::new(), image_size };
        let root = Extent { sector: disc.volume.root_sector, size: disc.volume.root_size };
        disc.nodes = disc.walk(root)?;
        Ok(disc)
    }

    fn walk(&mut self, dir: Extent) -> Result<Vec<Node>> {
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::from([dir.sector]);
        let mut stack = vec![(self.read_directory(dir)?.into_iter(), String::new())];
        while let Some((entries, prefix)) = stack.last_mut() {
            let Some(entry) = entries.next() else {
                stack.pop();
                continue;
            };
            self.check_extent(entry.extent)?;
            let path = format!("{prefix}{}", entry.name);
            let sub = entry.is_dir().then_some(entry.extent);
            out.push(Node { path: path.clone(), entry });
            if let Some(extent) = sub.filter(|e| e.size > 0) {
                if !seen.insert(extent.sector) {
                    return Err(Error::invalid(format!("XDVDFS directory {path} revisits sector {}", extent.sector)));
                }
                stack.push((self.read_directory(extent)?.into_iter(), format!("{path}/")));
            }
        }
        Ok(out)
    }

    fn read_directory(&mut self, extent: Extent) -> Result<Vec<DirEntry>> {
        if extent.size > MAX_DIRECTORY_SIZE {
            return Err(Error::invalid(format!("XDVDFS directory table of {} bytes", extent.size)));
        }
        parse_directory(&self.read_extent(extent)?)
    }

    fn check_extent(&self, extent: Extent) -> Result<()> {
        let end = self.layout.partition_offset() + extent.offset() + extent.size as u64;
        if end > self.image_size {
            return Err(Error::invalid(format!("XDVDFS extent {extent:?} ends past the image ({end:#x})")));
        }
        Ok(())
    }

    pub fn read_extent(&mut self, extent: Extent) -> Result<Vec<u8>> {
        self.check_extent(extent)?;
        let mut buf = vec![0u8; extent.size as usize];
        self.source.seek(SeekFrom::Start(self.layout.partition_offset() + extent.offset()))?;
        self.source.read_exact(&mut buf)?;
        Ok(buf)
    }
}

impl<R: Read + Seek> Disc<R> {
    pub fn files(&self) -> impl Iterator<Item = &Node> {
        self.nodes.iter().filter(|n| !n.is_dir())
    }

    pub fn find(&self, path: &str) -> Option<&Node> {
        let want = path.replace('\\', "/");
        let want = want.trim_matches('/');
        self.nodes.iter().find(|n| n.path.eq_ignore_ascii_case(want))
    }

    pub fn reader(&mut self, extent: Extent) -> Result<ExtentReader<'_, R>> {
        self.check_extent(extent)?;
        let start = self.layout.partition_offset() + extent.offset();
        self.source.seek(SeekFrom::Start(start))?;
        Ok(ExtentReader { source: &mut self.source, start, len: extent.size as u64, pos: 0 })
    }

    pub fn into_inner(self) -> R {
        self.source
    }
}

pub struct ExtentReader<'a, R> {
    source: &'a mut R,
    start: u64,
    len: u64,
    pos: u64,
}

impl<R> ExtentReader<'_, R> {
    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl<R: Read + Seek> Read for ExtentReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let left = self.len.saturating_sub(self.pos);
        let n = (buf.len() as u64).min(left) as usize;
        if n == 0 {
            return Ok(0);
        }
        let got = self.source.read(&mut buf[..n])?;
        self.pos += got as u64;
        Ok(got)
    }
}

impl<R: Read + Seek> Seek for ExtentReader<'_, R> {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let target = match to {
            SeekFrom::Start(p) => Some(p),
            SeekFrom::End(d) => self.len.checked_add_signed(d),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
        };
        let target = target.ok_or_else(|| std::io::Error::other("seek before the start of the file"))?;
        self.source.seek(SeekFrom::Start(self.start + target))?;
        self.pos = target;
        Ok(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Sparse {
        regions: Vec<(u64, Vec<u8>)>,
        len: u64,
        pos: u64,
    }

    impl Read for Sparse {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = (buf.len() as u64).min(self.len.saturating_sub(self.pos)) as usize;
            buf[..n].fill(0);
            for (at, data) in &self.regions {
                let (lo, hi) = (self.pos.max(*at), (self.pos + n as u64).min(at + data.len() as u64));
                if lo < hi {
                    buf[(lo - self.pos) as usize..(hi - self.pos) as usize]
                        .copy_from_slice(&data[(lo - at) as usize..(hi - at) as usize]);
                }
            }
            self.pos += n as u64;
            Ok(n)
        }
    }

    impl Seek for Sparse {
        fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
            self.pos = match to {
                SeekFrom::Start(p) => p,
                SeekFrom::End(d) => self.len.checked_add_signed(d).unwrap(),
                SeekFrom::Current(d) => self.pos.checked_add_signed(d).unwrap(),
            };
            Ok(self.pos)
        }
    }

    fn entry(table: &mut Vec<u8>, left: u16, right: u16, sector: u32, size: u32, attributes: u8, name: &str) {
        table.extend_from_slice(&left.to_le_bytes());
        table.extend_from_slice(&right.to_le_bytes());
        table.extend_from_slice(&sector.to_le_bytes());
        table.extend_from_slice(&size.to_le_bytes());
        table.extend_from_slice(&[attributes, name.len() as u8]);
        table.extend_from_slice(name.as_bytes());
        while !table.len().is_multiple_of(4) {
            table.push(0xFF);
        }
    }

    fn sector(mut data: Vec<u8>) -> Vec<u8> {
        data.resize(SECTOR_SIZE as usize, 0xFF);
        data
    }

    fn image(partition: u64) -> Sparse {
        let mut vd = MAGIC.to_vec();
        vd.extend_from_slice(&33u32.to_le_bytes());
        vd.extend_from_slice(&0x800u32.to_le_bytes());
        vd.extend_from_slice(&0x01CC_D7EE_978F_F808u64.to_le_bytes());
        vd.resize(0x7EC, 0);
        vd.extend_from_slice(MAGIC);
        let mut root = Vec::new();
        entry(&mut root, 5, 9, 36, 5, attr::NORMAL, "b.bin");
        entry(&mut root, 0, 0, 34, 0x800, attr::DIRECTORY, "a");
        entry(&mut root, 0, 0, 35, 3, attr::NORMAL, "c.txt");
        let mut sub = Vec::new();
        entry(&mut sub, 0, 0, 36, 5, attr::NORMAL, "x.dat");
        let s = |n: u64| partition + n * SECTOR_SIZE;
        Sparse {
            regions: vec![
                (s(32), vd),
                (s(33), sector(root)),
                (s(34), sector(sub)),
                (s(35), b"abc".to_vec()),
                (s(36), b"hello".to_vec()),
            ],
            len: s(37),
            pos: 0,
        }
    }

    #[test]
    fn walks_tree_and_reads_files() {
        let mut disc = Disc::open(image(0)).unwrap();
        assert_eq!(disc.layout, Layout::GamePartition);
        let paths: Vec<&str> = disc.nodes.iter().map(|n| n.path.as_str()).collect();
        assert_eq!(paths, ["a", "a/x.dat", "b.bin", "c.txt"]);
        assert_eq!(disc.volume.unix_time(), 1_327_117_303);
        let c = disc.find("C.TXT").unwrap().extent();
        let mut text = String::new();
        disc.reader(c).unwrap().read_to_string(&mut text).unwrap();
        assert_eq!(text, "abc");
        let x = disc.find("\\a\\x.dat").unwrap().extent();
        let mut r = disc.reader(x).unwrap();
        r.seek(SeekFrom::Start(1)).unwrap();
        let mut rest = Vec::new();
        r.read_to_end(&mut rest).unwrap();
        assert_eq!(rest, b"ello");
    }

    #[test]
    fn finds_xgd2_partition() {
        let disc = Disc::open(image(Layout::Xgd2.partition_offset())).unwrap();
        assert_eq!(disc.layout, Layout::Xgd2);
        assert_eq!(disc.files().count(), 3);
    }

    #[test]
    fn scan_matches_tree_and_rejects_cycles() {
        let mut root = Vec::new();
        entry(&mut root, 4, 0, 40, 1, attr::NORMAL, "m");
        entry(&mut root, 0, 0, 41, 1, attr::NORMAL, "c");
        let root = sector(root);
        let tree: Vec<String> = parse_directory(&root).unwrap().into_iter().map(|e| e.name).collect();
        let mut scan: Vec<String> = scan_directory(&root).unwrap().into_iter().map(|e| e.name).collect();
        scan.sort();
        assert_eq!(tree, scan);
        let mut cyclic = Vec::new();
        entry(&mut cyclic, 4, 0, 40, 1, attr::NORMAL, "m");
        entry(&mut cyclic, 0, 4, 41, 1, attr::NORMAL, "c");
        assert!(parse_directory(&sector(cyclic)).is_err());
        let mut bad = Vec::new();
        entry(&mut bad, 0, 0, 40, 1, attr::NORMAL, "../x");
        assert!(parse_directory(&sector(bad)).is_err());
    }
}
