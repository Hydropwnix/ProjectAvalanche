use crate::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TextureFormat {
    K8 = 2,
    K8888 = 6,
    K88 = 10,
    Dxt1 = 18,
    Dxt23 = 19,
    Dxt45 = 20,
    Dxn = 49,
    Dxt5a = 59,
}

impl TextureFormat {
    pub fn from_raw(raw: u32) -> Option<Self> {
        Some(match raw {
            2 => Self::K8,
            6 => Self::K8888,
            10 => Self::K88,
            18 => Self::Dxt1,
            19 => Self::Dxt23,
            20 => Self::Dxt45,
            49 => Self::Dxn,
            59 => Self::Dxt5a,
            _ => return None,
        })
    }

    pub fn raw(self) -> u32 {
        self as u32
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::K8 => "k_8",
            Self::K8888 => "k_8_8_8_8",
            Self::K88 => "k_8_8",
            Self::Dxt1 => "k_DXT1",
            Self::Dxt23 => "k_DXT2_3",
            Self::Dxt45 => "k_DXT4_5",
            Self::Dxn => "k_DXN",
            Self::Dxt5a => "k_DXT5A",
        }
    }

    pub fn block_size(self) -> [u32; 2] {
        if self.is_compressed() { [4, 4] } else { [1, 1] }
    }

    pub fn is_compressed(self) -> bool {
        matches!(self, Self::Dxt1 | Self::Dxt23 | Self::Dxt45 | Self::Dxn | Self::Dxt5a)
    }

    pub fn bytes_per_block(self) -> u32 {
        match self {
            Self::K8 => 1,
            Self::K88 => 2,
            Self::K8888 => 4,
            Self::Dxt1 | Self::Dxt5a => 8,
            Self::Dxt23 | Self::Dxt45 | Self::Dxn => 16,
        }
    }

    pub fn blocks(self, width: u32, height: u32) -> [u32; 2] {
        let [bw, bh] = self.block_size();
        [width.div_ceil(bw), height.div_ceil(bh)]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Endian {
    None,
    Swap8In16,
    Swap8In32,
    Swap16In32,
}

impl Endian {
    pub fn from_raw(raw: u32) -> Self {
        match raw & 3 {
            0 => Self::None,
            1 => Self::Swap8In16,
            2 => Self::Swap8In32,
            _ => Self::Swap16In32,
        }
    }

    pub fn raw(self) -> u32 {
        self as u32
    }

    pub fn apply(self, data: &mut [u8]) {
        match self {
            Self::None => {}
            Self::Swap8In16 => data.as_chunks_mut::<2>().0.iter_mut().for_each(|w| w.swap(0, 1)),
            Self::Swap8In32 => data.as_chunks_mut::<4>().0.iter_mut().for_each(|w| w.reverse()),
            Self::Swap16In32 => data.as_chunks_mut::<4>().0.iter_mut().for_each(|w| w.rotate_left(2)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DataDimension {
    D1,
    D2,
    D3,
    Cube,
}

impl DataDimension {
    pub fn from_raw(raw: u32) -> Self {
        match raw & 3 {
            0 => Self::D1,
            1 => Self::D2,
            2 => Self::D3,
            _ => Self::Cube,
        }
    }
}

pub const TILE_BLOCKS: u32 = 32;

pub const SUBRESOURCE_ALIGNMENT: usize = 4096;

pub fn tiled_offset_2d(x: u32, y: u32, pitch_blocks: u32, bpb_log2: u32) -> usize {
    let (x, y) = (x as usize, y as usize);
    let tiles_per_row = pitch_blocks.div_ceil(TILE_BLOCKS) as usize;
    let tile = (y >> 5) * tiles_per_row + (x >> 5);
    let linear = ((tile << 6) | (((y >> 1) & 7) << 3) | (x & 7)) << bpb_log2;
    let channel = ((x >> 3) & 3) ^ (((y >> 3) & 1) << 1);
    let bank = (y >> 4) & 1;
    (linear & 0xf)
        | ((y & 1) << 4)
        | (((linear >> 4) & 1) << 5)
        | (channel << 6)
        | (((linear >> 5) & 7) << 8)
        | (bank << 11)
        | ((linear >> 8) << 12)
}

fn bpb_log2(bytes_per_block: u32) -> Result<u32> {
    if !bytes_per_block.is_power_of_two() || bytes_per_block > 16 {
        return Err(Error::invalid(format!("unsupported tiled block size {bytes_per_block}")));
    }
    Ok(bytes_per_block.trailing_zeros())
}

pub fn untile_2d(
    src: &[u8],
    pitch_blocks: u32,
    bytes_per_block: u32,
    origin: [u32; 2],
    size: [u32; 2],
) -> Result<Vec<u8>> {
    let log2 = bpb_log2(bytes_per_block)?;
    let bpb = bytes_per_block as usize;
    let mut out = vec![0u8; size[0] as usize * size[1] as usize * bpb];
    let mut dst = out.chunks_exact_mut(bpb);
    for y in origin[1]..origin[1] + size[1] {
        for x in origin[0]..origin[0] + size[0] {
            let at = tiled_offset_2d(x, y, pitch_blocks, log2);
            let block = src.get(at..at + bpb).ok_or_else(|| {
                Error::invalid(format!("tiled block ({x}, {y}) at {at:#x} is outside the {:#x}-byte image", src.len()))
            })?;
            dst.next().expect("sized above").copy_from_slice(block);
        }
    }
    Ok(out)
}

pub fn tile_2d(
    blocks: &[u8],
    dst: &mut [u8],
    pitch_blocks: u32,
    bytes_per_block: u32,
    origin: [u32; 2],
    size: [u32; 2],
) -> Result<()> {
    let log2 = bpb_log2(bytes_per_block)?;
    let bpb = bytes_per_block as usize;
    if blocks.len() != size[0] as usize * size[1] as usize * bpb {
        return Err(Error::invalid("linear block buffer does not match the region size"));
    }
    let mut src = blocks.chunks_exact(bpb);
    for y in origin[1]..origin[1] + size[1] {
        for x in origin[0]..origin[0] + size[0] {
            let at = tiled_offset_2d(x, y, pitch_blocks, log2);
            let len = dst.len();
            let block = dst.get_mut(at..at + bpb).ok_or_else(|| {
                Error::invalid(format!("tiled block ({x}, {y}) at {at:#x} is outside {len:#x} bytes"))
            })?;
            block.copy_from_slice(src.next().expect("sized above"));
        }
    }
    Ok(())
}

fn log2_ceil(v: u32) -> u32 {
    v.max(1).next_power_of_two().trailing_zeros()
}

pub fn packed_mip_level(width: u32, height: u32) -> u32 {
    log2_ceil(width.min(height)).saturating_sub(4)
}

pub fn packed_mip_origin(width: u32, height: u32, level: u32) -> Option<[u32; 2]> {
    let (log2_w, log2_h) = (log2_ceil(width), log2_ceil(height));
    let first = packed_mip_level(width, height);
    if level < first {
        return None;
    }
    let packed = level - first;
    let wide = log2_w > log2_h;
    Some(if packed < 3 {
        let along = 16 >> packed;
        if wide { [0, along] } else { [along, 0] }
    } else if wide {
        [(1 << (log2_w - first)) >> (packed - 2), 0]
    } else {
        [0, (1 << (log2_h - first)) >> (packed - 2)]
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Surface2D {
    pub format: TextureFormat,
    pub width: u32,
    pub height: u32,
    pub base_pitch: u32,
    pub packed_mips: bool,
    pub max_level: u32,
    pub array_size: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LevelPlacement {
    pub level: u32,
    pub width: u32,
    pub height: u32,
    pub offset: usize,
    pub pitch_blocks: u32,
    pub height_blocks: u32,
    pub slice_stride: usize,
    pub origin_blocks: [u32; 2],
    pub packed: bool,
}

impl Surface2D {
    pub fn packed_level(&self) -> Option<u32> {
        self.packed_mips.then(|| packed_mip_level(self.width, self.height))
    }

    fn image_blocks(&self, base: bool, s: u32) -> [u32; 2] {
        let (pitch, rows) = if base {
            (self.base_pitch, self.height)
        } else {
            ((self.width.next_power_of_two() >> s).max(1), (self.height.next_power_of_two() >> s).max(1))
        };
        let [pb, rb] = self.format.blocks(pitch, rows);
        [pb.next_multiple_of(TILE_BLOCKS), rb.next_multiple_of(TILE_BLOCKS)]
    }

    fn placement(&self, level: u32, base: bool, storage: u32, offset: usize) -> LevelPlacement {
        let [pitch_blocks, height_blocks] = self.image_blocks(base, storage);
        let bpb = self.format.bytes_per_block() as usize;
        let slice_stride =
            (pitch_blocks as usize * height_blocks as usize * bpb).next_multiple_of(SUBRESOURCE_ALIGNMENT);
        let origin = if base {
            None
        } else {
            self.packed_level().and_then(|_| packed_mip_origin(self.width, self.height, level))
        };
        let [bw, bh] = self.format.block_size();
        LevelPlacement {
            level,
            width: (self.width >> level).max(1),
            height: (self.height >> level).max(1),
            offset,
            pitch_blocks,
            height_blocks,
            slice_stride,
            origin_blocks: origin.map_or([0, 0], |[x, y]| [x / bw, y / bh]),
            packed: origin.is_some(),
        }
    }

    pub fn levels(&self) -> Vec<LevelPlacement> {
        let mut out = vec![self.placement(0, true, 0, 0)];
        let packed = self.packed_level().unwrap_or(u32::MAX);
        let mut offset = 0;
        let mut prev: Option<(u32, usize)> = None;
        for level in 1..=self.max_level {
            let storage = level.min(packed);
            if let Some((s, stride)) = prev
                && s != storage
            {
                offset += stride * self.array_size as usize;
            }
            let p = self.placement(level, false, storage, offset);
            prev = Some((storage, p.slice_stride));
            out.push(p);
        }
        out
    }

    pub fn base_size(&self) -> usize {
        self.placement(0, true, 0, 0).slice_stride * self.array_size as usize
    }

    pub fn mips_size(&self) -> usize {
        self.levels().iter().skip(1).last().map_or(0, |p| p.offset + p.slice_stride * self.array_size as usize)
    }

    pub fn extract(&self, region: &[u8], level: &LevelPlacement, slice: u32) -> Result<Vec<u8>> {
        let start = level.offset + slice as usize * level.slice_stride;
        let end = (start + level.slice_stride).min(region.len());
        let image = region.get(start..end).ok_or_else(|| {
            Error::invalid(format!(
                "level {} starts at {start:#x}, past the {:#x}-byte region",
                level.level,
                region.len()
            ))
        })?;
        let size = self.format.blocks(level.width, level.height);
        untile_2d(image, level.pitch_blocks, self.format.bytes_per_block(), level.origin_blocks, size)
    }
}

pub const LINEAR_ROW_ALIGNMENT: usize = 256;

pub fn copy_linear_2d(src: &[u8], row_pitch: usize, bytes_per_block: u32, size: [u32; 2]) -> Result<Vec<u8>> {
    let row = size[0] as usize * bytes_per_block as usize;
    let mut out = Vec::with_capacity(row * size[1] as usize);
    for y in 0..size[1] as usize {
        let line = src
            .get(y * row_pitch..y * row_pitch + row)
            .ok_or_else(|| Error::invalid(format!("linear row {y} is outside the {:#x}-byte image", src.len())))?;
        out.extend_from_slice(line);
    }
    Ok(out)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinearVolume {
    pub format: TextureFormat,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub pitch: u32,
}

impl LinearVolume {
    pub fn row_pitch(&self) -> usize {
        self.format.blocks(self.pitch, 1)[0] as usize * self.format.bytes_per_block() as usize
    }

    pub fn slice_stride(&self) -> usize {
        self.row_pitch() * self.format.blocks(self.width, self.height)[1].next_multiple_of(TILE_BLOCKS) as usize
    }

    pub fn size(&self) -> usize {
        (self.slice_stride() * self.depth as usize).next_multiple_of(SUBRESOURCE_ALIGNMENT)
    }

    pub fn extract_slice(&self, region: &[u8], z: u32) -> Result<Vec<u8>> {
        let start = z as usize * self.slice_stride();
        let image = region.get(start..).ok_or_else(|| Error::invalid(format!("slice {z} is past the data")))?;
        let size = self.format.blocks(self.width, self.height);
        copy_linear_2d(image, self.row_pitch(), self.format.bytes_per_block(), size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endian_swaps() {
        let swapped = |e: Endian| {
            let mut d = [1, 2, 3, 4, 5];
            e.apply(&mut d);
            d
        };
        assert_eq!(swapped(Endian::None), [1, 2, 3, 4, 5]);
        assert_eq!(swapped(Endian::Swap8In16), [2, 1, 4, 3, 5]);
        assert_eq!(swapped(Endian::Swap8In32), [4, 3, 2, 1, 5]);
        assert_eq!(swapped(Endian::Swap16In32), [3, 4, 1, 2, 5]);
        assert_eq!((0..4).map(|r| Endian::from_raw(r).raw()).collect::<Vec<_>>(), [0, 1, 2, 3]);
    }

    #[test]
    fn tiled_offsets_permute_the_surface() {
        for log2 in 0..=4 {
            let (pitch, rows) = (64u32, 96u32);
            let size = ((pitch * rows) << log2) as usize;
            let mut seen = vec![false; size.next_multiple_of(SUBRESOURCE_ALIGNMENT) >> log2];
            for y in 0..rows {
                for x in 0..pitch {
                    let at = tiled_offset_2d(x, y, pitch, log2);
                    assert_eq!(at % (1 << log2), 0);
                    let slot = &mut seen[at >> log2];
                    assert!(!*slot, "bpb 2^{log2}: ({x}, {y}) collides");
                    *slot = true;
                }
            }
        }
    }

    #[test]
    fn tiled_offset_known_values() {
        assert_eq!(tiled_offset_2d(0, 0, 32, 2), 0);
        assert_eq!(tiled_offset_2d(3, 0, 32, 2), 12);
        assert_eq!(tiled_offset_2d(0, 1, 32, 2), 16);
        assert_eq!(tiled_offset_2d(1, 0, 32, 4), 32);
        assert_eq!(tiled_offset_2d(32, 0, 64, 2), 4096);
        assert_eq!(tiled_offset_2d(0, 32, 64, 2), 2 * 4096);
        assert_eq!(tiled_offset_2d(0, 32, 64, 3), 2 * 8192);
    }

    #[test]
    fn untile_inverts_tile() {
        let (pitch, bpb) = (64, 8);
        let mut surface = vec![0u8; 64 * 64 * bpb as usize];
        let region: Vec<u8> = (0..20 * 9 * bpb).map(|i| (i * 7 + 3) as u8).collect();
        tile_2d(&region, &mut surface, pitch, bpb, [5, 33], [20, 9]).unwrap();
        assert_eq!(untile_2d(&surface, pitch, bpb, [5, 33], [20, 9]).unwrap(), region);
        assert!(untile_2d(&surface[..4096], pitch, bpb, [5, 33], [20, 9]).is_err());
        assert!(untile_2d(&surface, pitch, 3, [0, 0], [1, 1]).is_err());
    }

    #[test]
    fn packed_mip_positions() {
        assert_eq!(packed_mip_level(256, 256), 4);
        assert_eq!(packed_mip_level(11, 9), 0);
        let small = Surface2D {
            format: TextureFormat::Dxt45,
            width: 11,
            height: 9,
            base_pitch: 128,
            packed_mips: true,
            max_level: 0,
            array_size: 1,
        };
        let base = small.levels()[0];
        assert_eq!((base.packed, base.origin_blocks, small.base_size()), (false, [0, 0], 0x4000));
        assert_eq!(packed_mip_origin(256, 256, 3), None);
        let square: Vec<_> = (4..=8).map(|l| packed_mip_origin(256, 256, l).unwrap()).collect();
        assert_eq!(square, [[16, 0], [8, 0], [4, 0], [0, 8], [0, 4]]);
        let tall: Vec<_> = (4..=9).map(|l| packed_mip_origin(256, 512, l).unwrap()).collect();
        assert_eq!(tall, [[16, 0], [8, 0], [4, 0], [0, 16], [0, 8], [0, 4]]);
        let wide: Vec<_> = (4..=9).map(|l| packed_mip_origin(512, 256, l).unwrap()).collect();
        assert_eq!(wide, [[0, 16], [0, 8], [0, 4], [16, 0], [8, 0], [4, 0]]);
    }

    #[test]
    fn mip_layout_of_a_1024_dxt5() {
        let s = Surface2D {
            format: TextureFormat::Dxt45,
            width: 1024,
            height: 1024,
            base_pitch: 1024,
            packed_mips: true,
            max_level: 10,
            array_size: 1,
        };
        assert_eq!(s.base_size(), 0x10_0000);
        let levels = s.levels();
        let offsets: Vec<_> = levels.iter().map(|l| l.offset).collect();
        assert_eq!(
            offsets,
            [0, 0, 0x4_0000, 0x5_0000, 0x5_4000, 0x5_8000, 0x5_c000, 0x5_c000, 0x5_c000, 0x5_c000, 0x5_c000]
        );
        assert_eq!(s.mips_size(), 0x6_0000);
        assert_eq!((levels[6].packed, levels[6].origin_blocks), (true, [4, 0]));
        assert_eq!((levels[10].width, levels[10].origin_blocks), (1, [0, 1]));
        assert!(!levels[5].packed);
    }
}
