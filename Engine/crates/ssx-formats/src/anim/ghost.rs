use std::io::Read as _;

use crate::reader::Reader;
use crate::{Error, Result};

pub const CONTAINER_MAGIC: &[u8; 4] = b"GCMP";
pub const MAGIC: &[u8; 4] = b"GHST";
pub const VERSION: u16 = 0x1A;
pub const DESCRIPTOR_SIZE: usize = 0x24;
pub const POSE_BLOCK_SIZE: usize = 0xF0;
pub const POSI: [u8; 4] = *b"POSI";
pub const GMPY: [u8; 4] = *b"GMPY";
pub const SCOR: [u8; 4] = *b"SCOR";

pub const ROTATION_CHANNELS: [&str; 29] = [
    "Hips",
    "RightUpLeg",
    "RightLeg",
    "RightAnkleEffectorAux",
    "LeftUpLeg",
    "LeftLeg",
    "LeftAnkleEffectorAux",
    "Spine",
    "Spine1",
    "Spine2",
    "Spine3",
    "LeftShoulder",
    "LeftArm",
    "LeftForeArm",
    "RightShoulder",
    "RightArm",
    "RightForeArm",
    "LeftWing",
    "LeftShoulder_Wing",
    "LeftArm_Wing",
    "LeftForeArm_Wing",
    "RightWing",
    "RightShoulder_Wing",
    "RightArm_Wing",
    "RightForeArm_Wing",
    "Neck",
    "Neck1",
    "Head",
    "Board",
];
pub const TRANSLATION_CHANNELS: [&str; 12] = [
    "Hips",
    "RightAnkleEffectorAux",
    "LeftAnkleEffectorAux",
    "LeftWing",
    "LeftShoulder_Wing",
    "LeftArm_Wing",
    "LeftForeArm_Wing",
    "RightWing",
    "RightShoulder_Wing",
    "RightArm_Wing",
    "RightForeArm_Wing",
    "Board",
];

pub const FLAG_GROUND_RAY: u8 = 0x01;
pub const FLAG_IS_AWESOME: u8 = 0x02;
pub const FLAG_IS_SUPER_AWESOME: u8 = 0x04;
pub const FLAG_IN_AIR: u8 = 0x08;
pub const FLAG_GOOFY: u8 = 0x10;
pub const FLAG_BOARD_PARTICLE: u8 = 0x20;
pub const FLAG_WINGSUIT: u8 = 0x40;
pub const FLAG_IMPACT_THIS_FRAME: u8 = 0x80;

pub fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c ^= u32::from(b) << 24;
        for _ in 0..8 {
            c = if c & 0x8000_0000 != 0 { (c << 1) ^ 0x04C1_1DB7 } else { c << 1 };
        }
    }
    !c
}

pub fn decompress(data: &[u8]) -> Result<Vec<u8>> {
    let mut r = Reader::be(data);
    let size = r.u32()? as usize;
    r.expect(CONTAINER_MAGIC)?;
    let body = &data[8..];
    let mut z = flate2::read::ZlibDecoder::new(body);
    let mut out = Vec::with_capacity(size);
    z.read_to_end(&mut out).map_err(|e| Error::Decompress(format!("GCMP zlib stream: {e}")))?;
    if z.total_in() as usize != body.len() {
        return Err(Error::invalid(format!(
            "GCMP: {} bytes after the zlib stream",
            body.len() - z.total_in() as usize
        )));
    }
    if out.len() != size {
        return Err(Error::invalid(format!("GCMP: inflated to {} bytes, header says {size}", out.len())));
    }
    Ok(out)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserHeader {
    pub gamertag: String,
    pub gear_flags: u16,
    pub score: u32,
    pub build: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stream {
    pub tag: [u8; 4],
    pub capacity: u32,
    pub element_size: u32,
    pub alignment: u32,
    pub interval: f32,
    pub times_offset: u32,
    pub data_offset: u32,
    pub data_size: u32,
    pub count: u32,
    pub times: Vec<f32>,
    pub data: Vec<u8>,
}

impl Stream {
    pub fn element(&self, i: usize) -> &[u8] {
        let n = self.element_size as usize;
        &self.data[i * n..(i + 1) * n]
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GhostFile {
    pub crc: u32,
    pub version: u16,
    pub unk_1a: u16,
    pub header: UserHeader,
    pub streams: Vec<Stream>,
}

impl GhostFile {
    pub fn from_container(data: &[u8]) -> Result<Self> {
        Self::parse(&decompress(data)?)
    }

    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::be(data);
        let crc = r.u32()?;
        r.expect(MAGIC)?;
        if crc32(&data[4..]) != crc {
            return Err(Error::invalid(format!("GHST: CRC {crc:#010x} does not match {:#010x}", crc32(&data[4..]))));
        }
        let (table, count, user_offset, user_size) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?);
        let (version, unk_1a) = (r.u16()?, r.u16()?);
        if version != VERSION {
            return Err(Error::invalid(format!("GHST: version {version:#x}, the reader takes {VERSION:#x}")));
        }
        if user_offset != 0x1C || user_size != 0x3C || table != user_offset + user_size {
            return Err(Error::invalid(format!(
                "GHST: user header at {user_offset:#x} size {user_size:#x}, table at {table:#x} (0x1C / 0x3C / 0x58 on the disc)"
            )));
        }
        r.seek(user_offset as usize)?;
        let name = r.bytes(0x32)?;
        let end = name.iter().position(|&b| b == 0).unwrap_or(name.len());
        let gamertag = String::from_utf8_lossy(&name[..end]).into_owned();
        let header = UserHeader { gamertag, gear_flags: r.u16()?, score: r.u32()?, build: r.u32()? };
        let mut streams = Vec::with_capacity(count as usize);
        let mut image_end = table as usize + count as usize * DESCRIPTOR_SIZE;
        for i in 0..count as usize {
            r.seek(table as usize + i * DESCRIPTOR_SIZE)?;
            let tag = r.array::<4>()?;
            let (capacity, element_size, alignment) = (r.u32()?, r.u32()?, r.u32()?);
            let interval = r.f32()?;
            let (times_offset, data_offset, data_size, count) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?);
            let label = String::from_utf8_lossy(&tag).into_owned();
            if count > capacity || u64::from(capacity) * u64::from(element_size) != u64::from(data_size) {
                return Err(Error::invalid(format!(
                    "GHST {label}: count {count}, capacity {capacity} × {element_size} B vs data size {data_size}"
                )));
            }
            if alignment == 0 || data_offset % alignment != 0 {
                return Err(Error::invalid(format!("GHST {label}: data at {data_offset:#x}, alignment {alignment}")));
            }
            let times_end = times_offset as usize + capacity as usize * 4;
            let data_end = data_offset as usize + data_size as usize;
            if times_end > data.len() || data_end > data.len() {
                return Err(Error::invalid(format!("GHST {label}: regions past the image ({} bytes)", data.len())));
            }
            image_end = image_end.max(times_end).max(data_end);
            r.seek(times_offset as usize)?;
            let times = (0..count).map(|_| r.f32()).collect::<Result<Vec<_>>>()?;
            let used = count as usize * element_size as usize;
            let data = data[data_offset as usize..data_offset as usize + used].to_vec();
            streams.push(Stream {
                tag,
                capacity,
                element_size,
                alignment,
                interval,
                times_offset,
                data_offset,
                data_size,
                count,
                times,
                data,
            });
        }
        if image_end != data.len() {
            return Err(Error::invalid(format!("GHST: regions end at {image_end:#x}, image is {:#x}", data.len())));
        }
        Ok(Self { crc, version, unk_1a, header, streams })
    }

    pub fn stream(&self, tag: [u8; 4]) -> Option<&Stream> {
        self.streams.iter().find(|s| s.tag == tag)
    }

    fn typed(&self, tag: [u8; 4], size: usize) -> Result<&Stream> {
        let s =
            self.stream(tag).ok_or_else(|| Error::invalid(format!("no {} stream", String::from_utf8_lossy(&tag))))?;
        if s.element_size as usize != size {
            return Err(Error::invalid(format!("{}: {} B elements", String::from_utf8_lossy(&tag), s.element_size)));
        }
        Ok(s)
    }

    pub fn positions(&self) -> Result<Vec<[f32; 3]>> {
        let s = self.typed(POSI, 12)?;
        (0..s.count as usize).map(|i| Reader::be(s.element(i)).vec3()).collect()
    }

    pub fn scores(&self) -> Result<Vec<f32>> {
        let s = self.typed(SCOR, 4)?;
        (0..s.count as usize).map(|i| Reader::be(s.element(i)).f32()).collect()
    }

    pub fn poses(&self) -> Result<Vec<PoseBlock>> {
        let s = self.typed(GMPY, POSE_BLOCK_SIZE)?;
        (0..s.count as usize).map(|i| PoseBlock::parse(s.element(i))).collect()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PoseBlock {
    pub trajectory_rotation: [f32; 4],
    pub trajectory_position: [f32; 3],
    pub velocity: u32,
    pub flags: u8,
    pub snow_coverage: u8,
    pub speed: u16,
    pub unk_24: [u8; 12],
    pub ground_contact: u32,
    pub ground_normal: u32,
    pub snow_spray_carving: u32,
    pub depth_below_snow: u16,
    pub snow_spray_speed: u16,
    pub landing_impact: u16,
    pub collision_entity: u16,
    pub impact_speed: u16,
    pub ticks_to_landing: u8,
    pub unk_47: u8,
    pub pose_weight: f32,
    pub rotations: [u32; 29],
    pub translations: [u32; 12],
}

impl PoseBlock {
    pub fn parse(b: &[u8]) -> Result<Self> {
        let mut r = Reader::be(b);
        let q = [r.f32()?, r.f32()?, r.f32()?, r.f32()?];
        let p = r.vec3()?;
        let velocity = r.u32()?;
        let (flags, snow_coverage, speed) = (r.u8()?, r.u8()?, r.u16()?);
        let unk_24 = r.array::<12>()?;
        let (ground_contact, ground_normal, snow_spray_carving) = (r.u32()?, r.u32()?, r.u32()?);
        let (depth_below_snow, snow_spray_speed, landing_impact) = (r.u16()?, r.u16()?, r.u16()?);
        let (collision_entity, impact_speed) = (r.u16()?, r.u16()?);
        let (ticks_to_landing, unk_47, pose_weight) = (r.u8()?, r.u8()?, r.f32()?);
        let mut rotations = [0u32; 29];
        for w in &mut rotations {
            *w = r.u32()?;
        }
        let mut translations = [0u32; 12];
        for w in &mut translations {
            *w = r.u32()?;
        }
        if r.pos() != POSE_BLOCK_SIZE {
            return Err(Error::invalid(format!("pose block: {} bytes read", r.pos())));
        }
        Ok(Self {
            trajectory_rotation: q,
            trajectory_position: p,
            velocity,
            flags,
            snow_coverage,
            speed,
            unk_24,
            ground_contact,
            ground_normal,
            snow_spray_carving,
            depth_below_snow,
            snow_spray_speed,
            landing_impact,
            collision_entity,
            impact_speed,
            ticks_to_landing,
            unk_47,
            pose_weight,
            rotations,
            translations,
        })
    }

    pub fn rotation(&self, i: usize) -> [f32; 4] {
        unpack_quaternion(self.rotations[i])
    }

    pub fn translation(&self, i: usize) -> [f32; 3] {
        unpack_vector(self.translations[i], [2.0; 3])
    }

    pub fn speed(&self) -> f32 {
        f16_to_f32(self.speed & 0x7FFF)
    }

    pub fn velocity(&self) -> [f32; 3] {
        unpack_vector(self.velocity & 0x3FFF_FFFF, [self.speed(); 3])
    }

    pub fn grinding(&self) -> bool {
        self.velocity & 0x4000_0000 != 0
    }

    pub fn boosting(&self) -> bool {
        self.velocity & 0x8000_0000 != 0
    }

    pub fn in_air(&self) -> bool {
        self.flags & FLAG_IN_AIR != 0
    }

    pub fn classic_rig(&self) -> bool {
        self.speed & 0x8000 == 0
    }
}

const RING_STEP: f32 = f32::from_bits(0x3B49_7495);
const INV_1023: f32 = f32::from_bits(0x3A80_2008);
const QUARTER_PI: f32 = f32::from_bits(0x3F49_0FDB);
const RING_EPS: f32 = f32::from_bits(0x3780_0000);
const MIN_AXIS: f32 = f32::from_bits(0x3A83_126F);

pub fn unpack_vector(w: u32, range: [f32; 3]) -> [f32; 3] {
    let c = [(w >> 20) & 0x3FF, (w >> 10) & 0x3FF, w & 0x3FF];
    std::array::from_fn(|i| (c[i] as f32 - 512.0) * (range[i] / 512.0))
}

pub fn unpack_quaternion(w: u32) -> [f32; 4] {
    let n = (w >> 14) as f32;
    let a = (w & 0x3FF) as f32 * INV_1023;
    let s2 = 1.0 - a * a;
    let s = if s2 == 0.0 { 0.0 } else { s2.sqrt() };
    if s <= MIN_AXIS {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let r = ((if n == 0.0 { 0.0 } else { n.sqrt() }) + RING_EPS).floor();
    let k = (n - r * r).floor();
    let phi = if r > 0.0 { k / r * QUARTER_PI } else { 0.0 };
    let x = phi.sin() * s;
    let y = (r * RING_STEP).cos() * s;
    let t = (1.0 - (a * a + (y * y + x * x))).max(0.0);
    let z = if t == 0.0 { 0.0 } else { t.sqrt() };
    let mut q = [x, y, z, a];
    for (i, c) in q.iter_mut().enumerate() {
        if w & (1 << (10 + i)) != 0 {
            *c = -*c;
        }
    }
    q
}

pub fn f16_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = i32::from((h >> 10) & 0x1F);
    let man = f32::from(h & 0x3FF);
    sign * match exp {
        0 => man * 2f32.powi(-24),
        31 if man == 0.0 => f32::INFINITY,
        31 => f32::NAN,
        e => (1.0 + man / 1024.0) * 2f32.powi(e - 15),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_matches_bzip2_check() {
        assert_eq!(crc32(b"123456789"), 0xFC89_1918);
    }

    #[test]
    fn vector_codes() {
        let w = (512 << 20) | (768 << 10) | 256;
        assert_eq!(unpack_vector(w, [2.0; 3]), [0.0, 1.0, -1.0]);
    }

    #[test]
    fn velocity_uses_stored_range_once_and_ignores_flags() {
        let mut bytes = [0u8; POSE_BLOCK_SIZE];
        let direction = (768u32 << 20) | (512 << 10) | 256;
        bytes[0x1c..0x20].copy_from_slice(&(direction | 0xc000_0000).to_be_bytes());
        bytes[0x22..0x24].copy_from_slice(&0xc000u16.to_be_bytes());
        let pose = PoseBlock::parse(&bytes).unwrap();
        assert_eq!(pose.speed(), 2.0);
        assert_eq!(pose.velocity(), [1.0, 0.0, -1.0]);
        assert!(pose.grinding() && pose.boosting() && !pose.classic_rig());
        bytes[0x22..0x24].copy_from_slice(&0x4000u16.to_be_bytes());
        let classic = PoseBlock::parse(&bytes).unwrap();
        assert_eq!(classic.velocity(), pose.velocity());
        assert!(classic.classic_rig());
    }

    #[test]
    fn quaternion_codes() {
        assert_eq!(unpack_quaternion(1023), [0.0, 0.0, 0.0, 1.0]);
        let a = (std::f32::consts::FRAC_1_SQRT_2 * 1023.0) as u32;
        let q = unpack_quaternion(a);
        assert!((q[1] - (1.0 - (a as f32 / 1023.0).powi(2)).sqrt()).abs() < 1e-6 && q[0] == 0.0 && q[2] == 0.0);
        let neg = unpack_quaternion(a | (1 << 11) | (1 << 13));
        assert_eq!(neg, [q[0], -q[1], q[2], -q[3]]);
        let q = unpack_quaternion(((511 * 511) << 14) | a);
        assert!(q[1].abs() < 0.01 && (q[2] - q[3]).abs() < 0.01);
    }

    #[test]
    fn halves() {
        assert_eq!(f16_to_f32(0x3C00), 1.0);
        assert_eq!(f16_to_f32(0xC000), -2.0);
        assert_eq!(f16_to_f32(0x2FA0), 0.119_140_625);
    }
}
