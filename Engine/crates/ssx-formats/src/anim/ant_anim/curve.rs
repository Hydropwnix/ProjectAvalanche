use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CurveGroup {
    pub num_keys: usize,
    pub num_channels: usize,
    pub key_offset: usize,
    pub channel_offset: usize,
    pub value_offset: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CurveAnimation {
    pub info: AnimationInfo,
    pub fps: f32,
    pub num_rotations: u16,
    pub num_vectors: u16,
    pub num_floats: u16,
    pub values: Vec<f32>,
    pub keys: Vec<u16>,
    pub consts: Vec<f32>,
    pub const_offsets: Vec<u16>,
    pub keys_blocks: Vec<[u16; 2]>,
    pub channel_groups: Vec<[u16; 2]>,
    pub channel_offsets: Vec<u16>,
    pub groups: Vec<CurveGroup>,
}

impl CurveAnimation {
    pub const CLASS: &'static str = "CurveAnimationAsset";

    pub fn parse(schema: &Schema, obj: &Object) -> Result<Self> {
        let c = Self::CLASS;
        let fl = Fields::new(schema, obj);
        let to16 = |v: Vec<[i64; 2]>| -> Result<Vec<[u16; 2]>> {
            v.into_iter()
                .map(|[a, b]| Ok([u16::try_from(a).map_err(|_| fl.bad(c))?, u16::try_from(b).map_err(|_| fl.bad(c))?]))
                .collect()
        };
        let keys_blocks = to16(fl.struct_ints("KeysBlocks", ["NumChannelGroups", "KeysCount"])?)?;
        let channel_groups = to16(fl.struct_ints("ChannelGroups", ["NumKeys", "NumChannels"])?)?;
        let anim = CurveAnimation {
            info: fl.info()?,
            fps: fl.f32("FPS")?,
            num_rotations: fl.u16("NumRotations")?,
            num_vectors: fl.u16("NumVectors")?,
            num_floats: fl.u16("NumFloats")?,
            values: fl.floats("Values")?,
            keys: fl.ints("Keys")?,
            consts: fl.floats("Consts")?,
            const_offsets: fl.ints("ConstOffsets")?,
            channel_offsets: fl.ints("ChannelOffsets")?,
            groups: Vec::new(),
            keys_blocks,
            channel_groups,
        };
        anim.validated()
    }

    fn validated(mut self) -> Result<Self> {
        let c = Self::CLASS;
        if self.keys_blocks.len() > 1 {
            return invalid(c, "more than one KeysBlock (not seen on the disc)");
        }
        let block_groups = self.keys_blocks.first().map_or(0, |b| usize::from(b[0]));
        let block_keys = self.keys_blocks.first().map_or(0, |b| usize::from(b[1]));
        let (mut k, mut ch, mut v) = (0, 0, 0);
        for g in &self.channel_groups {
            let (num_keys, num_channels) = (usize::from(g[0]), usize::from(g[1]));
            let keys = self.keys.get(k..k + num_keys).ok_or_else(|| Error::invalid(format!("{c}: Keys too short")))?;
            if num_keys == 0 || keys.windows(2).any(|w| w[1] < w[0]) || keys[0] != 0 {
                return invalid(c, "group keys empty, decreasing or not starting at 0");
            }
            self.groups.push(CurveGroup { num_keys, num_channels, key_offset: k, channel_offset: ch, value_offset: v });
            (k, ch, v) = (k + num_keys, ch + num_channels, v + num_keys * num_channels);
        }
        if block_groups != self.channel_groups.len() || block_keys != k || self.keys.len() != k {
            return invalid(
                c,
                format!(
                    "{block_groups} groups / {block_keys} keys in the block, {} / {k} found",
                    self.channel_groups.len()
                ),
            );
        }
        if self.values.len() != v || self.channel_offsets.len() != ch || self.consts.len() != self.const_offsets.len() {
            return invalid(c, "Values / ChannelOffsets / Consts sizes");
        }
        let (r, vv) = (usize::from(self.num_rotations), usize::from(self.num_vectors));
        let slots = 4 * (r + vv) + usize::from(self.num_floats);
        let mut seen = vec![false; slots];
        for &s in self.channel_offsets.iter().chain(&self.const_offsets) {
            let s = usize::from(s);
            if s >= slots || (s < 4 * (r + vv) && s % 4 == 3) || std::mem::replace(&mut seen[s], true) {
                return invalid(c, format!("slot {s} invalid or used twice"));
            }
        }
        if seen.iter().enumerate().any(|(s, &u)| !u && !(s < 4 * (r + vv) && s % 4 == 3)) {
            return invalid(c, "an output slot has no channel");
        }
        Ok(self)
    }

    pub fn counts(&self) -> ChannelCounts {
        ChannelCounts {
            quats: self.num_rotations.into(),
            vec3s: self.num_vectors.into(),
            floats: self.num_floats.into(),
        }
    }

    pub(super) fn segment(k: &[u16], frame: f32) -> (u8, [usize; 4], f32) {
        let n = k.len();
        if n <= 1 {
            return (1, [0; 4], 0.0);
        }
        let t = frame.min(f32::from(k[n - 1]));
        let ti = if t > 0.0 { t as u32 } else { 0 };
        let search = |mut lo: usize, mut hi: usize| loop {
            let mid = (lo + hi) >> 1;
            if u32::from(k[mid]) <= ti {
                if mid + 1 >= hi {
                    break mid;
                }
                lo = mid;
            } else {
                if mid <= lo + 1 {
                    break lo;
                }
                hi = mid;
            }
        };
        let mid = (n - 1) >> 1;
        let mut i = if u32::from(k[mid]) > ti {
            if mid <= 1 { 0 } else { search(0, mid) }
        } else if mid + 1 < n - 1 {
            search(mid, n - 1)
        } else {
            mid
        };
        if i < n - 1 && k[i] == k[i + 1] {
            i += 1;
        }
        let next_step = i + 2 < n && k[i + 1] == k[i + 2];
        let prev_step = i > 0 && k[i] == k[i - 1];
        if (prev_step && next_step) || i == n - 1 {
            return (1, [i; 4], 0.0);
        }
        let no_prev = i == 0 || prev_step;
        let span = f32::from(k[i + 1] - k[i]);
        let alpha = if span > 0.0 { (t - f32::from(k[i])) / span } else { 0.0 };
        match (no_prev, i + 2 < n && !next_step) {
            (false, true) => (2, [i - 1, i, i + 1, i + 2], alpha),
            (true, true) => (4, [i, i, i + 1, i + 2], alpha),
            (false, false) => (0, [i - 1, i, i + 1, i + 1], alpha),
            (true, false) => (3, [i, i, i + 1, i + 1], alpha),
        }
    }

    pub(super) fn spline(mode: u8, p: [f32; 4], t: f32) -> f32 {
        let [mut p0, p1, p2, mut p3] = p;
        match mode {
            1 => return p1,
            0 => p3 = (p2 - p1) + p2,
            4 => p0 = p1 - (p2 - p1),
            3 => (p0, p3) = (p1 - (p2 - p1), (p2 - p1) + p2),
            _ => {}
        }
        let a = -(p2 * 3.0 - (p1 * 3.0 - p0)) + p3;
        let b = (p2 * 4.0 + (p0 * 2.0 - p1 * 5.0)) - p3;
        (((a * t + b) * t + (p2 - p0)) * t + p1 * 2.0) * 0.5
    }

    pub fn slots_at(&self, frame: f32) -> Vec<f32> {
        let (r, v) = (usize::from(self.num_rotations), usize::from(self.num_vectors));
        let mut out = vec![0.0f32; 4 * (r + v) + usize::from(self.num_floats)];
        for (&s, &x) in self.const_offsets.iter().zip(&self.consts) {
            out[usize::from(s)] = x;
        }
        for g in &self.groups {
            let (mode, idx, t) = Self::segment(&self.keys[g.key_offset..g.key_offset + g.num_keys], frame);
            for ch in 0..g.num_channels {
                let p = idx.map(|i| self.values[g.value_offset + i * g.num_channels + ch]);
                out[usize::from(self.channel_offsets[g.channel_offset + ch])] = Self::spline(mode, p, t);
            }
        }
        out
    }

    pub fn sample(&self, frame: f32) -> Pose {
        let s = self.slots_at(frame);
        let (r, v) = (usize::from(self.num_rotations), usize::from(self.num_vectors));
        Pose {
            quats: (0..r).map(|i| euler_xyz_to_quat([s[4 * i], s[4 * i + 1], s[4 * i + 2]])).collect(),
            vec3s: (r..r + v).map(|i| [s[4 * i], s[4 * i + 1], s[4 * i + 2]]).collect(),
            floats: s[4 * (r + v)..].to_vec(),
        }
    }
}
