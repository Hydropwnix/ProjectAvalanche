mod curve;
mod dct;
mod dof;
mod raw;
mod vbr;

use std::collections::HashSet;

use crate::ant::{Guid, Object, Schema, Stream, Value};
use crate::{Error, Result};

pub use curve::*;
pub use dct::*;
pub use dof::*;
pub use raw::*;
pub use vbr::*;

pub const CODEC_VBR: u32 = 0x5642_5220;
pub const CODEC_DCT: u32 = 0x4443_5420;
pub const CODEC_CURVE: u32 = 0x4355_5256;
pub const CODEC_RAW: u32 = 0x5241_5720;
pub const CODEC_FRAME: u32 = 0x4652_414D;

#[derive(Clone, Debug, PartialEq)]
pub struct AnimationInfo {
    pub name: String,
    pub codec_type: u32,
    pub anim_id: u32,
    pub trim_offset: f32,
    pub end_frame: u16,
    pub additive: bool,
    pub channel_to_dof: Guid,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChannelCounts {
    pub quats: usize,
    pub vec3s: usize,
    pub floats: usize,
}

impl ChannelCounts {
    pub fn total(&self) -> usize {
        self.quats + self.vec3s + self.floats
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pose {
    pub quats: Vec<[f32; 4]>,
    pub vec3s: Vec<[f32; 3]>,
    pub floats: Vec<f32>,
}

impl Pose {
    pub fn counts(&self) -> ChannelCounts {
        ChannelCounts { quats: self.quats.len(), vec3s: self.vec3s.len(), floats: self.floats.len() }
    }
}

struct Fields<'a> {
    schema: &'a Schema,
    obj: &'a Object,
    class: &'a str,
}

impl<'a> Fields<'a> {
    fn new(schema: &'a Schema, obj: &'a Object) -> Self {
        Fields { schema, obj, class: &obj.type_desc(schema).name }
    }

    fn value(&self, name: &str) -> Result<&'a Value> {
        self.obj.get(self.schema, name).ok_or_else(|| Error::invalid(format!("{}: no field {name}", self.class)))
    }

    fn bad(&self, name: &str) -> Error {
        Error::invalid(format!("{}: field {name} has an unexpected type", self.class))
    }

    fn int(&self, name: &str) -> Result<i64> {
        self.value(name)?.as_i64().ok_or_else(|| self.bad(name))
    }

    fn u16(&self, name: &str) -> Result<u16> {
        u16::try_from(self.int(name)?).map_err(|_| self.bad(name))
    }

    fn u32(&self, name: &str) -> Result<u32> {
        u32::try_from(self.int(name)?).map_err(|_| self.bad(name))
    }

    fn f32(&self, name: &str) -> Result<f32> {
        self.value(name)?.as_f32().ok_or_else(|| self.bad(name))
    }

    fn bool(&self, name: &str) -> Result<bool> {
        self.value(name)?.as_bool().ok_or_else(|| self.bad(name))
    }

    fn guid(&self, name: &str) -> Result<Guid> {
        self.value(name)?.as_guid().ok_or_else(|| self.bad(name))
    }

    fn bytes(&self, name: &str) -> Result<&'a [u8]> {
        self.value(name)?.as_bytes().ok_or_else(|| self.bad(name))
    }

    fn array(&self, name: &str) -> Result<&'a [Value]> {
        self.value(name)?.as_array().ok_or_else(|| self.bad(name))
    }

    fn ints<T: TryFrom<i64>>(&self, name: &str) -> Result<Vec<T>> {
        self.array(name)?
            .iter()
            .map(|v| v.as_i64().and_then(|x| T::try_from(x).ok()).ok_or_else(|| self.bad(name)))
            .collect()
    }

    fn floats(&self, name: &str) -> Result<Vec<f32>> {
        self.array(name)?.iter().map(|v| v.as_f32().ok_or_else(|| self.bad(name))).collect()
    }

    fn info(&self) -> Result<AnimationInfo> {
        Ok(AnimationInfo {
            name: self.obj.name(self.schema).unwrap_or_default().to_string(),
            codec_type: self.u32("CodecType")?,
            anim_id: self.u32("AnimId")?,
            trim_offset: self.f32("TrimOffset")?,
            end_frame: self.u16("EndFrame")?,
            additive: self.bool("Additive")?,
            channel_to_dof: self.guid("ChannelToDofAsset")?,
        })
    }

    fn struct_ints(&self, name: &str, fields: [&str; 2]) -> Result<Vec<[i64; 2]>> {
        self.array(name)?
            .iter()
            .map(|v| {
                let s = v.as_struct().ok_or_else(|| self.bad(name))?;
                let get = |f: &str| s.own(self.schema, f).and_then(Value::as_i64).ok_or_else(|| self.bad(name));
                Ok([get(fields[0])?, get(fields[1])?])
            })
            .collect()
    }
}

fn invalid<T>(class: &str, msg: impl std::fmt::Display) -> Result<T> {
    Err(Error::invalid(format!("{class}: {msg}")))
}

fn f(bits: u32) -> f32 {
    f32::from_bits(bits)
}

pub fn normalize(q: [f32; 4]) -> [f32; 4] {
    let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if n > 0.0 { q.map(|x| x / n) } else { q }
}

pub fn nlerp(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    let dot = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
    normalize(std::array::from_fn(|i| if dot > 0.0 { (b[i] - a[i]) * t + a[i] } else { a[i] - (b[i] + a[i]) * t }))
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    std::array::from_fn(|i| (b[i] - a[i]) * t + a[i])
}

fn qmul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ]
}

pub fn euler_xyz_to_quat(r: [f32; 3]) -> [f32; 4] {
    let (sx, cx) = (r[0] * 0.5).sin_cos();
    let (sy, cy) = (r[1] * 0.5).sin_cos();
    let (sz, cz) = (r[2] * 0.5).sin_cos();
    qmul([0.0, 0.0, sz, cz], qmul([0.0, sy, 0.0, cy], [sx, 0.0, 0.0, cx]))
}

fn interval<T: Copy + Into<f32>>(times: &[T], frame: f32) -> (usize, usize, f32) {
    let last = times.len() - 1;
    if frame <= times[0].into() {
        return (0, 0, 0.0);
    }
    if frame >= times[last].into() {
        return (last, last, 0.0);
    }
    let i = times.partition_point(|&t| t.into() <= frame) - 1;
    let (a, b) = (times[i].into(), times[i + 1].into());
    (i, i + 1, (frame - a) / (b - a))
}

pub fn blend(a: &Pose, b: &Pose, t: f32) -> Pose {
    Pose {
        quats: a.quats.iter().zip(&b.quats).map(|(x, y)| nlerp(*x, *y, t)).collect(),
        vec3s: a.vec3s.iter().zip(&b.vec3s).map(|(x, y)| lerp3(*x, *y, t)).collect(),
        floats: a.floats.iter().zip(&b.floats).map(|(x, y)| (y - x).mul_add(t, *x)).collect(),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Animation {
    Vbr(VbrAnimation),
    Dct(DctAnimation),
    Curve(CurveAnimation),
    Raw(RawAnimation),
    Frame(RawAnimation),
}

impl Animation {
    pub fn from_object(schema: &Schema, obj: &Object) -> Result<Option<Animation>> {
        let class = obj.type_desc(schema).name.as_str();
        let (anim, codec) = match class {
            VbrAnimation::CLASS => (Animation::Vbr(VbrAnimation::parse(schema, obj)?), CODEC_VBR),
            DctAnimation::CLASS => (Animation::Dct(DctAnimation::parse(schema, obj)?), CODEC_DCT),
            CurveAnimation::CLASS => (Animation::Curve(CurveAnimation::parse(schema, obj)?), CODEC_CURVE),
            RawAnimation::CLASS => (Animation::Raw(RawAnimation::parse(schema, obj)?), CODEC_RAW),
            RawAnimation::FRAME_CLASS => (Animation::Frame(RawAnimation::parse(schema, obj)?), CODEC_FRAME),
            _ => return Ok(None),
        };
        if anim.info().codec_type != codec {
            return invalid(class, format!("CodecType {:#x}", anim.info().codec_type));
        }
        Ok(Some(anim))
    }

    pub fn info(&self) -> &AnimationInfo {
        match self {
            Animation::Vbr(a) => &a.info,
            Animation::Dct(a) => &a.info,
            Animation::Curve(a) => &a.info,
            Animation::Raw(a) | Animation::Frame(a) => &a.info,
        }
    }

    pub fn counts(&self) -> ChannelCounts {
        match self {
            Animation::Vbr(a) => a.counts(),
            Animation::Dct(a) => a.counts(),
            Animation::Curve(a) => a.counts(),
            Animation::Raw(a) | Animation::Frame(a) => a.counts(),
        }
    }

    pub fn key_frames(&self) -> Vec<u32> {
        match self {
            Animation::Vbr(a) => a.key_frames.clone(),
            Animation::Dct(a) => a.key_times.iter().map(|&t| t.into()).collect(),
            Animation::Curve(a) => {
                let mut k: Vec<u32> = a.keys.iter().map(|&t| t.into()).collect();
                k.sort_unstable();
                k.dedup();
                if k.is_empty() {
                    k.push(0);
                }
                k
            }
            Animation::Raw(a) | Animation::Frame(a) => a.key_times.iter().map(|&t| t.max(0) as u32).collect(),
        }
    }

    pub fn key(&self, k: usize) -> Result<Pose> {
        match self {
            Animation::Vbr(a) => a.key(k),
            Animation::Dct(a) => a.key(k),
            Animation::Raw(a) | Animation::Frame(a) => a.key(k),
            Animation::Curve(a) => {
                let frames = self.key_frames();
                let f = *frames.get(k).ok_or_else(|| Error::invalid("CurveAnimation: no such key"))?;
                Ok(a.sample(f as f32))
            }
        }
    }

    pub fn sample(&self, frame: f32) -> Result<Pose> {
        match self {
            Animation::Vbr(a) => a.sample(frame),
            Animation::Dct(a) => a.sample(frame),
            Animation::Curve(a) => Ok(a.sample(frame)),
            Animation::Raw(a) | Animation::Frame(a) => a.sample(frame),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_time_runs() {
        assert_eq!(vbr_key_frames(&[1, 5, 5]).unwrap(), vec![0, 6, 7, 8, 9, 10]);
        assert_eq!(vbr_key_frames(&[2, 3, 1]).unwrap(), vec![0, 1, 5]);
        assert!(vbr_key_frames(&[0]).is_err());
    }

    #[test]
    fn bit_readers() {
        let mut lsb = LsbBits { data: &[0b1011_0001, 0xFF], pos: 0 };
        assert_eq!(lsb.bits(1).unwrap(), 1);
        assert_eq!(lsb.bits(4).unwrap(), 0b1000);
        assert_eq!(lsb.bits(5).unwrap(), 0b11101);
        let mut msb = MsbBits { data: &[0b1011_0001, 0x80], pos: 0 };
        assert_eq!(msb.signed(3).unwrap(), -3);
        assert_eq!(msb.signed(0).unwrap(), 0);
        assert_eq!(msb.signed(6).unwrap(), 0b10_0011 - 64);
        assert!(MsbBits { data: &[0], pos: 7 }.signed(2).is_err());
    }

    #[test]
    fn euler_matches_axis_products() {
        let q = euler_xyz_to_quat([0.0, std::f32::consts::FRAC_PI_2, 0.0]);
        assert!((q[1] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6 && (q[3] - q[1]).abs() < 1e-6);
        let h = euler_xyz_to_quat([std::f32::consts::FRAC_PI_2, 0.0, std::f32::consts::FRAC_PI_2]);
        assert!(h.iter().all(|x| (x - 0.5).abs() < 1e-6), "{h:?}");
    }

    #[test]
    fn nlerp_takes_the_short_way() {
        let (a, b) = ([0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 0.0, -1.0]);
        assert_eq!(nlerp(a, b, 0.5), [0.0, 0.0, 0.0, 1.0]);
        let h = nlerp([0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 0.0], 0.5);
        assert!((h[2] + h[3]).abs() < 1e-6 && (h[3] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
    }

    #[test]
    fn channel_map_walk() {
        let map = [2, 1, 1, 2, 0];
        assert_eq!(walk_chan_map("t", &map, 0, 0, 3).unwrap(), vec![0, 1, 3]);
        assert_eq!(walk_chan_map("t", &map, 1, 2, 3).unwrap(), vec![2, 4, 5]);
        assert!(walk_chan_map("t", &[1], 0, 0, 3).is_err());
    }

    #[test]
    fn offset_tracks() {
        let t = parse_offset_tracks("t", &[2, 1, 2, 10, 3, 20, 4, 1, 0x81], 8).unwrap();
        assert_eq!(t[0], VbrOffsetTrack { channel: 1, values: vec![10, 20], blocks: vec![3] });
        assert_eq!(t[1], VbrOffsetTrack { channel: 5, values: vec![-127], blocks: vec![] });
        assert!(parse_offset_tracks("t", &[1, 9, 1, 0], 8).is_err());
        assert!(parse_offset_tracks("t", &[1, 0, 1, 0, 0], 8).is_err());
    }

    #[test]
    fn curve_segments() {
        let k = [0u16, 10, 20, 20, 30];
        assert_eq!(CurveAnimation::segment(&k, 5.0), (4, [0, 0, 1, 2], 0.5));
        assert_eq!(CurveAnimation::segment(&k, 15.0).0, 0);
        assert_eq!(CurveAnimation::segment(&k, 25.0), (3, [3, 3, 4, 4], 0.5));
        assert_eq!(CurveAnimation::segment(&k, 99.0), (3, [3, 3, 4, 4], 1.0));
        assert_eq!(CurveAnimation::segment(&[0, 0, 5, 5], 2.0).0, 1);
        assert!((CurveAnimation::spline(3, [0.0, 1.0, 3.0, 0.0], 0.25) - 1.5).abs() < 1e-6);
        assert_eq!(CurveAnimation::spline(1, [9.0, 2.0, 9.0, 9.0], 0.7), 2.0);
    }

    #[test]
    fn idct_tables_match_the_cosine_formula() {
        for (fr, dct) in DCT_IDCT.iter().enumerate() {
            for (k, &d) in dct.iter().enumerate() {
                let c =
                    ((fr as f64 + 0.5) * k as f64 * std::f64::consts::PI / 8.0).cos() * if k == 0 { 0.5 } else { 1.0 };
                assert!((f64::from(vbr_idct(fr, k, 0)) - 0.25 * c).abs() < 1e-6, "vbr {fr} {k}");
                assert!((f64::from(f(d)) - 0.5 * c).abs() < 1e-6, "dct {fr} {k}");
            }
        }
        assert_eq!(vbr_idct(4, 6, 1), -vbr_idct(4, 6, 0));
    }
}
