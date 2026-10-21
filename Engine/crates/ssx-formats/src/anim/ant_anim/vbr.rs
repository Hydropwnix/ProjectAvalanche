use super::*;

const VBR_IDCT: [[u32; 8]; 8] = [
    [0x3E000000, 0x3E7B14BE, 0x3E6C835E, 0x3E54DB31, 0x3E3504F3, 0x3E0E39D9, 0x3DC3EF15, 0x3D47C5BC],
    [0x3E000000, 0x3E54DB31, 0x3DC3EF15, 0xBD47C5C2, 0xBE3504F3, 0xBE7B14BF, 0xBE6C835E, 0xBE0E39D6],
    [0x3E000000, 0x3E0E39D9, 0xBDC3EF18, 0xBE7B14BF, 0xBE3504F1, 0x3D47C5C7, 0x3E6C835F, 0x3E54DB2D],
    [0x3E000000, 0x3D47C5BC, 0xBE6C8360, 0xBE0E39D6, 0x3E3504F7, 0x3E54DB2D, 0xBDC3EF26, 0xBE7B14BD],
    [0x3E000000, 0xBD47C5C2, 0xBE6C835E, 0x3E0E39DD, 0x3E3504F3, 0xBE54DB34, 0xBDC3EF05, 0x3E7B14C1],
    [0x3E000000, 0xBE0E39DC, 0xBDC3EF0B, 0x3E7B14BE, 0xBE3504FB, 0xBD47C5BA, 0x3E6C835B, 0xBE54DB31],
    [0x3E000000, 0xBE54DB32, 0x3DC3EF1B, 0x3D47C5B2, 0xBE3504EF, 0x3E7B14BF, 0xBE6C8362, 0x3E0E39E9],
    [0x3E000000, 0xBE7B14BF, 0x3E6C835F, 0xBE54DB34, 0x3E3504F4, 0xBE0E39E4, 0x3DC3EF26, 0xBD47C615],
];

const VBR_IDCT_F4K6_YZW: u32 = 0x3DC3EF05;

pub(super) fn vbr_idct(frame: usize, k: usize, lane: usize) -> f32 {
    if frame == 4 && k == 6 && lane != 0 { f(VBR_IDCT_F4K6_YZW) } else { f(VBR_IDCT[frame][k]) }
}

const LN_K2: [u32; 8] =
    [0x3F317218, 0x3F8C9F54, 0x3FB17218, 0x3FCE0210, 0x3FE55860, 0x3FF91395, 0x40051592, 0x400C9F54];

pub fn vbr_key_frames(rle: &[u8]) -> Result<Vec<u32>> {
    let mut t = vec![0u32];
    for (i, &b) in rle.iter().enumerate() {
        if i % 2 == 0 {
            if b == 0 {
                return invalid("VbrAnimationAsset", "key-time run of length 0");
            }
            for _ in 1..b {
                t.push(t[t.len() - 1] + 1);
            }
        } else {
            t.push(t[t.len() - 1] + u32::from(b) + 1);
        }
    }
    Ok(t)
}

pub(super) struct LsbBits<'a> {
    pub(super) data: &'a [u8],
    pub(super) pos: usize,
}

impl LsbBits<'_> {
    pub(super) fn bits(&mut self, n: u32) -> Result<u32> {
        let mut v = 0;
        for i in 0..n {
            let byte = *self.data.get(self.pos >> 3).ok_or_else(|| Error::invalid("VBR frame block overrun"))?;
            v |= u32::from((byte >> (self.pos & 7)) & 1) << i;
            self.pos += 1;
        }
        Ok(v)
    }
}

pub const VBR_FLAG_CYCLE: u16 = 0x01;
pub const VBR_FLAG_ALL_FRAMES: u16 = 0x02;
pub const VBR_FLAG_UNK_KERNEL: u16 = 0x04;
pub const VBR_FLAG_NO_CONSTANTS: u16 = 0x08;
pub const VBR_FLAG_UNK_80: u16 = 0x80;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VbrSections {
    pub key_times: std::ops::Range<usize>,
    pub const_indices: std::ops::Range<usize>,
    pub const_chan_map: std::ops::Range<usize>,
    pub control_words: std::ops::Range<usize>,
    pub vector_offsets: std::ops::Range<usize>,
    pub float_offsets: std::ops::Range<usize>,
    pub frame_blocks: std::ops::Range<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VbrAnimation {
    pub info: AnimationInfo,
    pub quat_min: f32,
    pub traj_min: f32,
    pub vec3_min: f32,
    pub float_min: f32,
    pub quat_max: f32,
    pub traj_max: f32,
    pub vec3_max: f32,
    pub float_max: f32,
    pub vector_offset_scale: f32,
    pub float_offset_scale: f32,
    pub dct: f32,
    pub quat_count: u16,
    pub vec3_count: u16,
    pub float_count: u16,
    pub const_quat_count: u16,
    pub const_vec3_count: u16,
    pub const_float_count: u16,
    pub key_time_size: u16,
    pub num_keys: u16,
    pub const_chan_map_size: u16,
    pub const_palette_size: u16,
    pub vector_offset_size: u16,
    pub float_offset_size: u16,
    pub flags: u16,
    pub constant_palette: Vec<f32>,
    pub frame_block_sizes: Vec<u16>,
    pub data: Vec<u8>,
    pub sections: VbrSections,
    pub key_frames: Vec<u32>,
    pub anim_channels: Vec<usize>,
    pub const_channels: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VbrOffsetTrack {
    pub channel: usize,
    pub values: Vec<i8>,
    pub blocks: Vec<u8>,
}

pub(super) fn parse_offset_tracks(class: &str, s: &[u8], lanes: usize) -> Result<Vec<VbrOffsetTrack>> {
    if s.is_empty() {
        return Ok(Vec::new());
    }
    let byte = |i: usize| s.get(i).copied().ok_or_else(|| Error::invalid(format!("{class}: offset stream overrun")));
    let (count, mut p, mut channel) = (byte(0)?, 1, 0usize);
    let mut tracks = Vec::with_capacity(count.into());
    for _ in 0..count {
        channel += usize::from(byte(p)?);
        let n = usize::from(byte(p + 1)?);
        if n == 0 || channel >= lanes {
            return invalid(class, format!("offset track with {n} points on lane {channel} of {lanes}"));
        }
        let pts = p + 2;
        let values = (0..n).map(|i| byte(pts + 2 * i).map(|b| b as i8)).collect::<Result<_>>()?;
        let blocks = (0..n - 1).map(|i| byte(pts + 2 * i + 1)).collect::<Result<_>>()?;
        tracks.push(VbrOffsetTrack { channel, values, blocks });
        p = pts + 2 * n - 1;
    }
    if p != s.len() {
        return invalid(class, format!("offset stream ends at {p} of {}", s.len()));
    }
    Ok(tracks)
}

pub(super) fn walk_chan_map(class: &str, map: &[u8], start: usize, first: usize, count: usize) -> Result<Vec<usize>> {
    let at = |i: usize| map.get(i).copied().ok_or_else(|| Error::invalid(format!("{class}: channel map overrun")));
    let (mut idx, mut run, mut slot, mut out) = (start, 0usize, first, Vec::with_capacity(count));
    for _ in 0..count {
        for _ in 0..2 {
            if run >= usize::from(at(idx)?) {
                slot += usize::from(at(idx + 1)?);
                idx += 2;
                run = 0;
            }
        }
        out.push(slot);
        slot += 1;
        run += 1;
    }
    Ok(out)
}

impl VbrAnimation {
    pub const CLASS: &'static str = "VbrAnimationAsset";

    pub fn parse(schema: &Schema, obj: &Object) -> Result<Self> {
        let c = Self::CLASS;
        let fl = Fields::new(schema, obj);
        let u = |n: &str| fl.u16(n);
        let (q, v, fc) = (u("QuaternionCount")?, u("Vector3Count")?, u("FloatCount")?);
        let (cq, cv, cf) = (u("ConstQuaternionCount")?, u("ConstVector3Count")?, u("ConstFloatCount")?);
        let (kts, nk, ccms, pal) = (u("KeyTimeSize")?, u("NumKeys")?, u("ConstChanMapSize")?, u("ConstPaletteSize")?);
        let (vos, fos, flags) = (u("VectorOffsetSize")?, u("FloatOffsetSize")?, u("Flags")?);
        let info = fl.info()?;
        let palette = fl.floats("ConstantPalette")?;
        let fbs: Vec<u16> = fl.ints("FrameBlockSizes")?;
        let data = fl.bytes("Data")?.to_vec();
        let us = usize::from;
        if nk == 0 || flags & !0x8F != 0 || palette.len() != us(pal) || fbs.len() != us(nk).div_ceil(8) {
            return invalid(
                c,
                format!("keys {nk}, flags {flags:#x}, palette {}/{pal}, blocks {}", palette.len(), fbs.len()),
            );
        }
        let mut p = 0;
        let mut take = |n: usize| {
            p += n;
            p - n..p
        };
        let key_times = take(us(kts));
        let const_indices = take(4 * us(cq) + 3 * us(cv) + us(cf));
        let const_chan_map = take(us(ccms));
        let control_words = take(4 * (4 * us(q) + 3 * us(v) + us(fc)));
        let vector_offsets = take(us(vos));
        let float_offsets = take(us(fos));
        let frame_blocks = take(fbs.iter().map(|&s| us(s)).sum());
        if frame_blocks.end != data.len() || fbs.iter().any(|&s| s < 2) {
            return invalid(c, format!("sections end at {} of {} data bytes", frame_blocks.end, data.len()));
        }
        let sections = VbrSections {
            key_times,
            const_indices,
            const_chan_map,
            control_words,
            vector_offsets,
            float_offsets,
            frame_blocks,
        };
        if data[sections.const_indices.clone()].iter().any(|&i| us(i.into()) >= palette.len()) {
            return invalid(c, "constant palette index out of range");
        }
        let key_frames = if kts == 0 {
            if flags & VBR_FLAG_ALL_FRAMES == 0 || u32::from(nk) != u32::from(info.end_frame) + 1 {
                return invalid(c, "no key times but not every frame a key");
            }
            (0..u32::from(nk)).collect()
        } else {
            let t = vbr_key_frames(&data[sections.key_times.clone()])?;
            if flags & VBR_FLAG_ALL_FRAMES != 0 || t.len() != us(nk) || t[t.len() - 1] != u32::from(info.end_frame) {
                return invalid(c, format!("{} key times ending at {:?} for {nk} keys", t.len(), t.last()));
            }
            t
        };
        let (na, nc) = (us(q) + us(v) + us(fc), us(cq) + us(cv) + us(cf));
        let (anim_channels, const_channels) = if flags & VBR_FLAG_NO_CONSTANTS != 0 {
            if nc != 0 || ccms != 0 {
                return invalid(c, "constants without a channel map");
            }
            ((0..na).collect(), Vec::new())
        } else {
            let map = &data[sections.const_chan_map.clone()];
            let first = usize::from(*map.first().ok_or_else(|| Error::invalid(format!("{c}: empty channel map")))?);
            (walk_chan_map(c, map, 0, 0, na)?, walk_chan_map(c, map, 1, first, nc)?)
        };
        let (eq, ev) = (us(q) + us(cq), us(q) + us(cq) + us(v) + us(cv));
        let mut seen = vec![false; na + nc];
        let ranges = [(0, eq), (eq, ev), (ev, na + nc)];
        let typed = |items: &[usize], counts: [u16; 3]| {
            let mut i = 0;
            counts.iter().zip(ranges).all(|(&n, (lo, hi))| {
                let ok = items[i..i + us(n)].iter().all(|&s| (lo..hi).contains(&s));
                i += us(n);
                ok
            })
        };
        if !typed(&anim_channels, [q, v, fc]) || !typed(&const_channels, [cq, cv, cf]) {
            return invalid(c, "channel map puts an item outside its type");
        }
        for &s in anim_channels.iter().chain(&const_channels) {
            if s >= seen.len() || std::mem::replace(&mut seen[s], true) {
                return invalid(c, format!("channel {s} mapped twice or out of range"));
            }
        }
        let anim = VbrAnimation {
            info,
            quat_min: fl.f32("QuatMin")?,
            traj_min: fl.f32("TrajMin")?,
            vec3_min: fl.f32("Vec3Min")?,
            float_min: fl.f32("FloatMin")?,
            quat_max: fl.f32("QuatMax")?,
            traj_max: fl.f32("TrajMax")?,
            vec3_max: fl.f32("Vec3Max")?,
            float_max: fl.f32("FloatMax")?,
            vector_offset_scale: fl.f32("VectorOffsetScale")?,
            float_offset_scale: fl.f32("FloatOffsetScale")?,
            dct: fl.f32("Dct")?,
            quat_count: q,
            vec3_count: v,
            float_count: fc,
            const_quat_count: cq,
            const_vec3_count: cv,
            const_float_count: cf,
            key_time_size: kts,
            num_keys: nk,
            const_chan_map_size: ccms,
            const_palette_size: pal,
            vector_offset_size: vos,
            float_offset_size: fos,
            flags,
            constant_palette: palette,
            frame_block_sizes: fbs,
            data,
            sections,
            key_frames,
            anim_channels,
            const_channels,
        };
        anim.vector_offset_tracks()?;
        anim.float_offset_tracks()?;
        Ok(anim)
    }

    pub fn counts(&self) -> ChannelCounts {
        ChannelCounts {
            quats: usize::from(self.quat_count) + usize::from(self.const_quat_count),
            vec3s: usize::from(self.vec3_count) + usize::from(self.const_vec3_count),
            floats: usize::from(self.float_count) + usize::from(self.const_float_count),
        }
    }

    pub fn vector_offset_tracks(&self) -> Result<Vec<VbrOffsetTrack>> {
        parse_offset_tracks(
            Self::CLASS,
            &self.data[self.sections.vector_offsets.clone()],
            4 * usize::from(self.vec3_count),
        )
    }

    pub fn float_offset_tracks(&self) -> Result<Vec<VbrOffsetTrack>> {
        let lanes = usize::from(self.float_count).next_multiple_of(4);
        parse_offset_tracks(Self::CLASS, &self.data[self.sections.float_offsets.clone()], lanes)
    }

    fn row_stride(&self) -> usize {
        4 * (usize::from(self.quat_count) + usize::from(self.vec3_count))
            + usize::from(self.float_count).next_multiple_of(4)
    }

    fn components(&self) -> usize {
        4 * usize::from(self.quat_count) + 3 * usize::from(self.vec3_count) + usize::from(self.float_count)
    }

    fn lane_of(&self, c: usize) -> usize {
        let (q4, v3) = (4 * usize::from(self.quat_count), 3 * usize::from(self.vec3_count));
        if c < q4 {
            c
        } else if c < q4 + v3 {
            q4 + 4 * ((c - q4) / 3) + (c - q4) % 3
        } else {
            c + usize::from(self.vec3_count)
        }
    }

    pub fn decode_block(&self, block: usize) -> Result<VbrBlock> {
        let c = Self::CLASS;
        let size = usize::from(
            *self.frame_block_sizes.get(block).ok_or_else(|| Error::invalid(format!("{c}: no block {block}")))?,
        );
        let start = self.sections.frame_blocks.start
            + self.frame_block_sizes[..block].iter().map(|&s| usize::from(s)).sum::<usize>();
        let bytes = &self.data[start..start + size];
        let quant_index = [bytes[0], bytes[1]];
        let stream: Vec<u8> = bytes[2..].iter().rev().copied().collect();
        let mut br = LsbBits { data: &stream, pos: 0 };
        let stride = self.row_stride();
        let mut coefs = vec![[0i32; 8]; stride];
        let ctrl = &self.data[self.sections.control_words.clone()];
        for comp in 0..self.components() {
            let w = &ctrl[4 * comp..4 * comp + 4];
            let width: [u32; 8] = std::array::from_fn(|k| u32::from(w[k / 2] >> (4 * (k % 2))) & 15);
            let mut sel = [false; 8];
            for k in 0..8 {
                sel[k] = width[k] != 0 && br.bits(1)? == 1;
            }
            let mut positive = [false; 8];
            for k in 0..8 {
                positive[k] = sel[k] && br.bits(1)? == 1;
            }
            let lane = self.lane_of(comp);
            for k in 0..8 {
                if sel[k] {
                    let m = br.bits(width[k])? as i32;
                    coefs[lane][k] = if positive[k] { m } else { -m };
                }
            }
        }
        let bits_available = 8 * stream.len();
        let padding_zero = (br.pos..bits_available).all(|b| stream[b >> 3] >> (b & 7) & 1 == 0);
        if bits_available - br.pos >= 8 || !padding_zero {
            return invalid(c, format!("block {block}: {} of {bits_available} bits used", br.pos));
        }
        let quant = |b: u8| -> [f32; 8] {
            let s = ((f32::from(b) + 1.0) * f(0x3E4CCCCD)) * self.dct;
            let unit = f(0x3800_0000) * self.dct;
            std::array::from_fn(|k| s.mul_add(f(LN_K2[k]), 1.0) * unit)
        };
        let (q0, q1) = (quant(quant_index[0]), quant(quant_index[1]));
        let (nq, nv) = (usize::from(self.quat_count), usize::from(self.vec3_count));
        let range = |kind: usize| match kind {
            0 => (self.quat_min, self.quat_max - self.quat_min),
            1 => (self.traj_min, self.traj_max - self.traj_min),
            2 => (self.vec3_min, self.vec3_max - self.vec3_min),
            _ => (self.float_min, self.float_max - self.float_min),
        };
        let mut rows = vec![vec![0.0f32; stride]; 8];
        for comp in 0..self.components() {
            let lane = self.lane_of(comp);
            let group = lane / 4;
            let kind = if group < nq {
                0
            } else if group == nq && nv != 0 {
                1
            } else if group < nq + nv {
                2
            } else {
                3
            };
            let q = if group < nq { &q0 } else { &q1 };
            let (min, span) = range(kind);
            let cf = &coefs[lane];
            for (fr, row) in rows.iter_mut().enumerate() {
                let mut acc = (cf[0] as f32 * q[0]).mul_add(vbr_idct(fr, 0, lane % 4), 0.5);
                for k in 1..8 {
                    acc = (cf[k] as f32 * q[k]).mul_add(vbr_idct(fr, k, lane % 4), acc);
                }
                row[lane] = acc.mul_add(span, min);
            }
        }
        let nk = usize::from(self.num_keys);
        self.apply_offsets(&mut rows, block, 4 * nq, &self.vector_offset_tracks()?, self.vector_offset_scale, nk);
        self.apply_offsets(&mut rows, block, 4 * (nq + nv), &self.float_offset_tracks()?, self.float_offset_scale, nk);
        Ok(VbrBlock { rows, quant_index, bits_used: br.pos, bits_available })
    }

    fn apply_offsets(
        &self,
        rows: &mut [Vec<f32>],
        block: usize,
        base: usize,
        tracks: &[VbrOffsetTrack],
        scale: f32,
        nk: usize,
    ) {
        let unit = scale * f(0x3C01_0204);
        let (mut blk, mut sub) = (block, 0);
        if blk != 0 && blk == nk >> 3 {
            blk -= 1;
            sub = nk & 7;
        }
        for t in tracks {
            let n = t.values.len();
            let (mut i, mut rem) = (0, blk);
            if n > 1 && blk >= usize::from(t.blocks[0]) {
                while i < n - 1 {
                    rem -= usize::from(t.blocks[i]);
                    i += 1;
                    if i < n - 1 && rem >= usize::from(t.blocks[i]) {
                        continue;
                    }
                    break;
                }
            }
            let lane = base + t.channel;
            if i == n - 1 {
                let v = f32::from(t.values[n - 1]) * unit;
                rows.iter_mut().for_each(|r| r[lane] += v);
            } else {
                let v0 = f32::from(t.values[i]) * unit;
                let slope = f32::from(t.values[i + 1]).mul_add(unit, -v0) / (usize::from(t.blocks[i]) * 8) as f32;
                let mut v = ((rem * 8 + sub) as f32).mul_add(slope, v0);
                for r in rows.iter_mut() {
                    r[lane] += v;
                    v += slope;
                }
            }
        }
    }

    pub fn key_location(&self, k: usize) -> (usize, usize) {
        let nk = usize::from(self.num_keys);
        let row = if k >= nk - nk % 8 && k > 7 { k + 8 - nk } else { k & 7 };
        (k >> 3, row)
    }

    pub fn key(&self, k: usize) -> Result<Pose> {
        if k >= usize::from(self.num_keys) {
            return invalid(Self::CLASS, format!("no key {k}"));
        }
        let (block, row) = self.key_location(k);
        self.assemble(&self.decode_block(block)?.rows[row], None, 0.0)
    }

    pub fn sample(&self, frame: f32) -> Result<Pose> {
        let (k0, k1, t) = self.keys_at(frame);
        let (b0, r0) = self.key_location(k0);
        let (b1, r1) = self.key_location(k1);
        let a = self.decode_block(b0)?;
        let b = if b1 == b0 { None } else { Some(self.decode_block(b1)?) };
        let rb = &b.as_ref().unwrap_or(&a).rows[r1];
        self.assemble(&a.rows[r0], Some(rb), t)
    }

    pub fn keys_at(&self, frame: f32) -> (usize, usize, f32) {
        let (fl, frac) = (frame.floor(), frame - frame.floor());
        let end = u32::from(self.info.end_frame);
        let k0 = if fl < 0.0 { 0 } else { fl as u32 };
        let (k0, k1) = if self.flags & VBR_FLAG_CYCLE != 0 {
            (k0 % (end + 1), (k0 + 1) % (end + 1))
        } else {
            (k0.min(end), (k0 + 1).min(end))
        };
        let last = usize::from(self.num_keys) - 1;
        if self.flags & VBR_FLAG_ALL_FRAMES != 0 {
            return ((k0 as usize).min(last), (k1 as usize).min(last), frac);
        }
        let i = self.key_frames.partition_point(|&t| t <= k0).saturating_sub(1);
        if i >= last {
            return (last, last, 0.0);
        }
        let len = self.key_frames[i + 1] - self.key_frames[i];
        let t = if len == 1 { frac } else { ((k0 - self.key_frames[i]) as f32 + frac) / len as f32 };
        (i, i + 1, t)
    }

    pub fn assemble(&self, a: &[f32], b: Option<&[f32]>, t: f32) -> Result<Pose> {
        let counts = self.counts();
        let mut pose = Pose {
            quats: vec![[0.0; 4]; counts.quats],
            vec3s: vec![[0.0; 3]; counts.vec3s],
            floats: vec![0.0; counts.floats],
        };
        let (nq, nv) = (usize::from(self.quat_count), usize::from(self.vec3_count));
        let (eq, ev) = (counts.quats, counts.quats + counts.vec3s);
        let b = b.unwrap_or(a);
        for (j, &ch) in self.anim_channels.iter().enumerate() {
            if j < nq {
                let (x, y) = (std::array::from_fn(|l| a[4 * j + l]), std::array::from_fn(|l| b[4 * j + l]));
                pose.quats[ch] = nlerp(x, y, t);
            } else if j < nq + nv {
                let o = 4 * j;
                pose.vec3s[ch - eq] = lerp3([a[o], a[o + 1], a[o + 2]], [b[o], b[o + 1], b[o + 2]], t);
            } else {
                let o = 4 * (nq + nv) + (j - nq - nv);
                pose.floats[ch - ev] = (b[o] - a[o]).mul_add(t, a[o]);
            }
        }
        let idx = &self.data[self.sections.const_indices.clone()];
        let pal = |i: usize, min: f32, max: f32| self.constant_palette[usize::from(idx[i])].mul_add(max - min, min);
        let (cq, cv) = (usize::from(self.const_quat_count), usize::from(self.const_vec3_count));
        for (j, &ch) in self.const_channels.iter().enumerate() {
            if j < cq {
                pose.quats[ch] = std::array::from_fn(|l| pal(4 * j + l, self.quat_min, self.quat_max));
            } else if j < cq + cv {
                let o = 4 * cq + 3 * (j - cq);
                pose.vec3s[ch - eq] = std::array::from_fn(|l| pal(o + l, self.vec3_min, self.vec3_max));
            } else {
                pose.floats[ch - ev] = pal(4 * cq + 3 * cv + (j - cq - cv), self.float_min, self.float_max);
            }
        }
        Ok(pose)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct VbrBlock {
    pub rows: Vec<Vec<f32>>,
    pub quant_index: [u8; 2],
    pub bits_used: usize,
    pub bits_available: usize,
}
