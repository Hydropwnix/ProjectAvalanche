use super::*;

#[derive(Clone, Debug, PartialEq)]
pub struct RawAnimation {
    pub info: AnimationInfo,
    pub key_times: Vec<i16>,
    pub quat_count: u32,
    pub vec3_count: u32,
    pub float_count: u32,
    pub cycle: bool,
    pub data: Vec<f32>,
}

impl RawAnimation {
    pub const CLASS: &'static str = "RawAnimationAsset";
    pub const FRAME_CLASS: &'static str = "FrameAnimationAsset";

    pub fn parse(schema: &Schema, obj: &Object) -> Result<Self> {
        let fl = Fields::new(schema, obj);
        let frame = fl.class == Self::FRAME_CLASS;
        let info = fl.info()?;
        let (q, v, fc) = (fl.u32("QuatCount")?, fl.u32("Vec3Count")?, fl.u32("FloatCount")?);
        let key_times: Vec<i16> = if frame { vec![0] } else { fl.ints("KeyTimes")? };
        let nk = if frame { 1 } else { fl.u32("NumKeys")? as usize };
        let anim = RawAnimation {
            info,
            key_times,
            quat_count: q,
            vec3_count: v,
            float_count: fc,
            cycle: !frame && fl.bool("Cycle")?,
            data: fl.floats("Data")?,
        };
        let kt = &anim.key_times;
        let ascending = kt.first() == Some(&0) && kt.windows(2).all(|w| w[1] > w[0]);
        let end_ok = frame || kt.last().map(|&t| i32::from(t)) == Some(i32::from(anim.info.end_frame));
        if nk == 0 || kt.len() != nk || !ascending || !end_ok || nk.checked_mul(anim.stride()) != Some(anim.data.len())
        {
            return invalid(fl.class, format!("{} key times for {nk} keys, {} floats", kt.len(), anim.data.len()));
        }
        Ok(anim)
    }

    pub(super) fn stride(&self) -> usize {
        4 * (self.quat_count as usize + self.vec3_count as usize) + (self.float_count as usize).next_multiple_of(4)
    }

    pub fn counts(&self) -> ChannelCounts {
        ChannelCounts {
            quats: self.quat_count as usize,
            vec3s: self.vec3_count as usize,
            floats: self.float_count as usize,
        }
    }

    pub fn key(&self, k: usize) -> Result<Pose> {
        let s = self.stride();
        let d = self.data.get(k * s..(k + 1) * s).ok_or_else(|| Error::invalid("RawAnimation: no such key"))?;
        let (q, v) = (self.quat_count as usize, self.vec3_count as usize);
        Ok(Pose {
            quats: (0..q).map(|i| std::array::from_fn(|l| d[4 * i + l])).collect(),
            vec3s: (q..q + v).map(|i| [d[4 * i], d[4 * i + 1], d[4 * i + 2]]).collect(),
            floats: d[4 * (q + v)..4 * (q + v) + self.float_count as usize].to_vec(),
        })
    }

    pub fn sample(&self, frame: f32) -> Result<Pose> {
        let (k0, k1, t) = interval(&self.key_times, frame);
        let (a, b) = (self.key(k0)?, self.key(k1)?);
        Ok(blend(&a, &b, t))
    }
}
