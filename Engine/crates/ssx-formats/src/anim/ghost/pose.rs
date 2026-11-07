use super::pack::{
    pack_half, pack_quaternion, pack_vector, unpack_half, unpack_quaternion, unpack_vector, vector_length,
};
use crate::reader::Reader;
use crate::{Error, Result, vmx};

pub const POSE_BLOCK_SIZE: usize = 0xF0;
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

    pub fn to_bytes(&self) -> [u8; POSE_BLOCK_SIZE] {
        let mut out = Vec::with_capacity(POSE_BLOCK_SIZE);
        for v in self.trajectory_rotation.iter().chain(&self.trajectory_position) {
            out.extend_from_slice(&v.to_be_bytes());
        }
        out.extend_from_slice(&self.velocity.to_be_bytes());
        out.extend_from_slice(&[self.flags, self.snow_coverage]);
        out.extend_from_slice(&self.speed.to_be_bytes());
        out.extend_from_slice(&self.unk_24);
        for w in [self.ground_contact, self.ground_normal, self.snow_spray_carving] {
            out.extend_from_slice(&w.to_be_bytes());
        }
        for h in [self.depth_below_snow, self.snow_spray_speed, self.landing_impact, self.collision_entity] {
            out.extend_from_slice(&h.to_be_bytes());
        }
        out.extend_from_slice(&self.impact_speed.to_be_bytes());
        out.extend_from_slice(&[self.ticks_to_landing, self.unk_47]);
        out.extend_from_slice(&self.pose_weight.to_be_bytes());
        for w in self.rotations.iter().chain(&self.translations) {
            out.extend_from_slice(&w.to_be_bytes());
        }
        let mut block = [0u8; POSE_BLOCK_SIZE];
        block.copy_from_slice(&out);
        block
    }

    pub fn record(src: &PoseSource) -> Self {
        let flags = src.flags.iter().enumerate().fold(0u8, |acc, (i, &b)| acc | (u8::from(b) << i));
        let range = vmx::mul(vector_length(src.velocity), SPEED_RANGE_FACTOR);
        let velocity =
            pack_vector(src.velocity, [range; 3]) | (u32::from(src.grinding) << 30) | (u32::from(src.boosting) << 31);
        let p = src.trajectory_position;
        let contact = std::array::from_fn(|i| vmx::sub(src.ground_contact[i], p[i]));
        Self {
            trajectory_rotation: src.trajectory_rotation,
            trajectory_position: p,
            velocity,
            flags,
            snow_coverage: low_byte(src.snow_coverage * 255.0),
            speed: pack_half(range) | 0x8000,
            unk_24: [0; 12],
            ground_contact: pack_vector(contact, [CHANNEL_RANGE; 3]),
            ground_normal: pack_vector(src.ground_normal, [CHANNEL_RANGE; 3]),
            snow_spray_carving: pack_vector(src.snow_spray_carving, [CARVING_RANGE; 3]),
            depth_below_snow: pack_half(src.depth_below_snow),
            snow_spray_speed: pack_half(vector_length(src.snow_spray_velocity)),
            landing_impact: pack_half(src.landing_impact),
            collision_entity: src.collision_entity as u16,
            impact_speed: pack_half(vector_length(src.impact_velocity)),
            ticks_to_landing: ticks_byte(src.ticks_to_landing),
            unk_47: 0,
            pose_weight: 1.0,
            rotations: src.rotations.map(pack_quaternion),
            translations: src.translations.map(|t| pack_vector(t, [CHANNEL_RANGE; 3])),
        }
    }

    pub fn rotation(&self, i: usize) -> [f32; 4] {
        unpack_quaternion(self.rotations[i])
    }

    pub fn translation(&self, i: usize) -> [f32; 3] {
        unpack_vector(self.translations[i], [CHANNEL_RANGE; 3])
    }

    pub fn ground_contact_offset(&self) -> [f32; 3] {
        unpack_vector(self.ground_contact, [CHANNEL_RANGE; 3])
    }

    pub fn ground_normal(&self) -> [f32; 3] {
        unpack_vector(self.ground_normal, [CHANNEL_RANGE; 3])
    }

    pub fn snow_spray_carving(&self) -> [f32; 3] {
        unpack_vector(self.snow_spray_carving, [CARVING_RANGE; 3])
    }

    pub fn speed(&self) -> f32 {
        unpack_half(self.speed & 0x7FFF)
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

pub const CHANNEL_RANGE: f32 = 2.0;
pub const CARVING_RANGE: f32 = 1.5;
const SPEED_RANGE_FACTOR: f32 = f32::from_bits(0x3F80_20C5);

#[derive(Clone, Debug, PartialEq)]
pub struct PoseSource {
    pub trajectory_rotation: [f32; 4],
    pub trajectory_position: [f32; 3],
    pub rotations: [[f32; 4]; 29],
    pub translations: [[f32; 3]; 12],
    pub flags: [bool; 8],
    pub snow_coverage: f32,
    pub depth_below_snow: f32,
    pub ground_contact: [f32; 3],
    pub ground_normal: [f32; 3],
    pub velocity: [f32; 3],
    pub grinding: bool,
    pub boosting: bool,
    pub snow_spray_velocity: [f32; 3],
    pub snow_spray_carving: [f32; 3],
    pub landing_impact: f32,
    pub collision_entity: i32,
    pub impact_velocity: [f32; 3],
    pub ticks_to_landing: f32,
}

fn low_byte(x: f32) -> u8 {
    (x as i64) as u8
}

fn ticks_byte(ticks: f32) -> u8 {
    let t = ticks / 2.0;
    let t = if -t >= 0.0 { 0.0 } else { t };
    let t = if 255.0 - t >= 0.0 { t } else { 255.0 };
    low_byte(t)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn a_parsed_block_writes_back_its_bytes() {
        let bytes: [u8; POSE_BLOCK_SIZE] = std::array::from_fn(|i| (i as u8).wrapping_mul(37).wrapping_add(11));
        assert_eq!(PoseBlock::parse(&bytes).unwrap().to_bytes(), bytes);
    }

    fn source() -> PoseSource {
        let q = [0.1, -0.7, 0.2, 0.678_233];
        let n = q.iter().map(|c: &f32| c * c).sum::<f32>().sqrt();
        PoseSource {
            trajectory_rotation: [0.0, std::f32::consts::FRAC_1_SQRT_2, 0.0, std::f32::consts::FRAC_1_SQRT_2],
            trajectory_position: [100.0, 2000.0, -50.0],
            rotations: [q.map(|c| c / n); 29],
            translations: [[0.25, -1.0, 1.5]; 12],
            flags: [true, false, true, true, false, false, true, false],
            snow_coverage: 0.5,
            depth_below_snow: 0.12,
            ground_contact: [100.5, 1999.0, -50.25],
            ground_normal: [0.0, 1.0, 0.0],
            velocity: [0.3, -0.2, 0.6],
            grinding: true,
            boosting: false,
            snow_spray_velocity: [0.0, 0.0, 0.0],
            snow_spray_carving: [0.5, 0.0, -0.25],
            landing_impact: 0.4,
            collision_entity: 0x1_2345,
            impact_velocity: [0.0, 0.3, 0.4],
            ticks_to_landing: 77.0,
        }
    }

    #[test]
    fn a_recorded_block_reads_back_its_source() {
        let s = source();
        let b = PoseBlock::record(&s);
        assert_eq!(PoseBlock::parse(&b.to_bytes()).unwrap(), b);
        assert_eq!((b.flags, b.snow_coverage, b.collision_entity, b.ticks_to_landing), (0b0100_1101, 127, 0x2345, 38));
        assert_eq!(b.speed & 0x8000, 0x8000);
        assert!(b.grinding() && !b.boosting());
        let speed = s.velocity.iter().map(|c| c * c).sum::<f32>().sqrt() * 1.001;
        assert!((b.speed() - speed).abs() <= speed / 1024.0);
        let v = b.velocity();
        assert!(v.iter().zip(s.velocity).all(|(a, b)| (a - b).abs() <= speed / 256.0), "{v:?}");
        let c = b.ground_contact_offset();
        assert!(c.iter().zip([0.5, -1.0, -0.25]).all(|(a, b)| (a - b).abs() <= 1.0 / 256.0), "{c:?}");
        assert_eq!(b.ground_normal(), [0.0, 1.0, 0.0]);
        let q = b.rotation(0);
        assert!(q.iter().zip(s.rotations[0]).all(|(a, b)| (a - b).abs() < 4e-3), "{q:?}");
        assert_eq!(b.translation(11), [0.25, -1.0, 1.5]);
        assert_eq!((unpack_half(b.impact_speed), unpack_half(b.snow_spray_speed)), (0.5, 0.0));
    }

    #[test]
    fn the_byte_fields_at_their_edges() {
        assert_eq!([low_byte(f32::NAN), low_byte(-1.0), low_byte(1e30), low_byte(255.9)], [0, 255, 255, 255]);
        assert_eq!([ticks_byte(-4.0), ticks_byte(f32::NAN), ticks_byte(1e9), ticks_byte(9.9)], [0, 255, 255, 4]);
    }
}
