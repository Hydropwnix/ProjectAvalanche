use super::*;

pub(super) const DCT_IDCT: [[u32; 8]; 8] = [
    [0x3E800000, 0x3EFB14BE, 0x3EEC835E, 0x3ED4DB31, 0x3EB504F3, 0x3E8E39D9, 0x3E43EF14, 0x3DC7C5BF],
    [0x3E800000, 0x3ED4DB31, 0x3E43EF14, 0xBDC7C5C4, 0xBEB504F4, 0xBEFB14BF, 0xBEEC835E, 0xBE8E39D8],
    [0x3E800000, 0x3E8E39D9, 0xBE43EF17, 0xBEFB14BF, 0xBEB504F2, 0x3DC7C5CB, 0x3EEC8360, 0x3ED4DB30],
    [0x3E800000, 0x3DC7C5BF, 0xBEEC835F, 0xBE8E39D8, 0x3EB504F5, 0x3ED4DB30, 0xBE43EF1C, 0xBEFB14BE],
    [0x3E800000, 0xBDC7C5C4, 0xBEEC835E, 0x3E8E39DC, 0x3EB504F1, 0xBED4DB34, 0xBE43EF0C, 0x3EFB14C0],
    [0x3E800000, 0xBE8E39DB, 0xBE43EF11, 0x3EFB14BE, 0xBEB504F6, 0xBDC7C5AE, 0x3EEC835C, 0xBED4DB35],
    [0x3E800000, 0xBED4DB32, 0x3E43EF1A, 0x3DC7C5B4, 0xBEB504F0, 0x3EFB14BD, 0xBEEC8361, 0x3E8E39E1],
    [0x3E800000, 0xBEFB14BF, 0x3EEC8360, 0xBED4DB34, 0x3EB504F7, 0xBE8E39E0, 0x3E43EF24, 0xBDC7C5E7],
];

pub(super) struct MsbBits<'a> {
    pub(super) data: &'a [u8],
    pub(super) pos: usize,
}

impl MsbBits<'_> {
    pub(super) fn signed(&mut self, n: u32) -> Result<i32> {
        if n == 0 {
            return Ok(0);
        }
        let mut v: u32 = 0;
        for _ in 0..n {
            let byte = *self.data.get(self.pos >> 3).ok_or_else(|| Error::invalid("DCT bit stream overrun"))?;
            v = (v << 1) | u32::from((byte >> (7 - (self.pos & 7))) & 1);
            self.pos += 1;
        }
        Ok(((v << (32 - n)) as i32) >> (32 - n))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DctVector {
    pub coefficients: usize,
    pub unk_desc_low: u8,
    pub base: [i16; 4],
    pub widths: Vec<[u32; 4]>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DctAnimation {
    pub info: AnimationInfo,
    pub num_keys: u16,
    pub num_quats: u16,
    pub num_vec3: u16,
    pub num_float: u16,
    pub num_float_vec: u16,
    pub cycle: bool,
    pub quantize_mult_block: u16,
    pub quantize_mult_subblock: u8,
    pub catch_all_bit_count: u8,
    pub key_times: Vec<u16>,
    pub vectors: Vec<DctVector>,
    pub data_size: u32,
    pub data: Vec<u8>,
    pub block_bits: usize,
    pub dc_bits: usize,
}

impl DctAnimation {
    pub const CLASS: &'static str = "DctAnimationAsset";

    pub fn parse(schema: &Schema, obj: &Object) -> Result<Self> {
        let c = Self::CLASS;
        let fl = Fields::new(schema, obj);
        let info = fl.info()?;
        let (nk, nq, nv) = (fl.u16("NumKeys")?, fl.u16("NumQuats")?, fl.u16("NumVec3")?);
        let (nf, nfv) = (fl.u16("NumFloat")?, fl.u16("NumFloatVec")?);
        let key_times: Vec<u16> = fl.ints("KeyTimes")?;
        let desc = fl.bytes("DofTableDescBytes")?;
        let base: [Vec<i16>; 4] =
            [fl.ints("DeltaBaseX")?, fl.ints("DeltaBaseY")?, fl.ints("DeltaBaseZ")?, fl.ints("DeltaBaseW")?];
        let bps: Vec<u16> = fl.ints("BitsPerSubblock")?;
        let n = usize::from(nq) + usize::from(nv) + usize::from(nfv);
        if desc.len() != n || base.iter().any(|b| b.len() != n) || nfv != nf.div_ceil(4) {
            return invalid(c, format!("{n} vectors, {} descriptors, {nf} floats in {nfv} vectors", desc.len()));
        }
        if nk == 0
            || key_times.len() != usize::from(nk)
            || key_times[0] != 0
            || key_times.windows(2).any(|w| w[1] <= w[0])
            || key_times[key_times.len() - 1] != info.end_frame
        {
            return invalid(c, "key times not 0 < … < EndFrame");
        }
        let mut vectors = Vec::with_capacity(n);
        let mut p = 0;
        for (v, &d) in desc.iter().enumerate() {
            let coefficients = usize::from(d >> 4);
            let entries = bps
                .get(p..p + coefficients)
                .ok_or_else(|| Error::invalid(format!("{c}: BitsPerSubblock too short")))?;
            if coefficients > 8 {
                return invalid(c, format!("vector {v} with {coefficients} coefficients"));
            }
            let widths = entries.iter().map(|&e| [12, 8, 4, 0].map(|s| u32::from(e >> s) & 15)).collect();
            vectors.push(DctVector {
                coefficients,
                unk_desc_low: d & 15,
                base: std::array::from_fn(|l| base[l][v]),
                widths,
            });
            p += coefficients;
        }
        let block_bits = vectors.iter().flat_map(|v| &v.widths).map(|w| w.iter().sum::<u32>() as usize).sum();
        let dc_bits = vectors.iter().filter_map(|v| v.widths.first()).map(|w| w.iter().sum::<u32>() as usize).sum();
        let data = fl.bytes("Data")?.to_vec();
        let data_size = fl.u32("DataSize")?;
        let image = (12 + n).next_multiple_of(2) + 8 * n + 2 * bps.len() + data.len();
        let (qmb, cab) = (fl.u16("QuantizeMultBlock")?, fl.int("CatchAllBitCount")?);
        if p != bps.len() || image != data_size as usize || qmb == 0 || cab >= 16 {
            return invalid(
                c,
                format!("tables {p}/{}, image {image} vs DataSize {data_size}, QMB {qmb}, catch-all {cab}", bps.len()),
            );
        }
        let total = usize::from(nk).div_ceil(8) * block_bits - dc_bits;
        let tail_zero = (total..8 * data.len()).all(|b| data[b >> 3] >> (7 - (b & 7)) & 1 == 0);
        if total > 8 * data.len() || !tail_zero || data.len() > total.div_ceil(8) + 32 {
            return invalid(c, format!("{total} stream bits in {} bytes (padding not zero or too long)", data.len()));
        }
        Ok(DctAnimation {
            info,
            num_keys: nk,
            num_quats: nq,
            num_vec3: nv,
            num_float: nf,
            num_float_vec: nfv,
            cycle: fl.bool("Cycle")?,
            quantize_mult_block: qmb,
            quantize_mult_subblock: fl.int("QuantizeMultSubblock")? as u8,
            catch_all_bit_count: cab as u8,
            key_times,
            vectors,
            data_size,
            data,
            block_bits,
            dc_bits,
        })
    }

    pub fn counts(&self) -> ChannelCounts {
        ChannelCounts { quats: self.num_quats.into(), vec3s: self.num_vec3.into(), floats: self.num_float.into() }
    }

    pub fn block_coefficients(&self, block: usize) -> Result<Vec<[[i16; 4]; 8]>> {
        let start = if block == 0 { 0 } else { block * self.block_bits - self.dc_bits };
        let mut br = MsbBits { data: &self.data, pos: start };
        let mut out = Vec::with_capacity(self.vectors.len());
        for v in &self.vectors {
            let mut rows = [[0i16; 4]; 8];
            rows[0] = v.base;
            for (k, w) in v.widths.iter().enumerate() {
                if block == 0 && k == 0 {
                    continue;
                }
                let mut vals = [0i16; 4];
                for l in 0..4 {
                    vals[l] = br.signed(w[l])? as i16;
                }
                rows[k] = if k == 0 { std::array::from_fn(|l| v.base[l].wrapping_add(vals[l])) } else { vals };
            }
            out.push(rows);
        }
        Ok(out)
    }

    pub fn decode_block(&self, block: usize) -> Result<Vec<Vec<[f32; 4]>>> {
        const TENTHS: [u32; 8] =
            [0x3F80_0000, 0x3DCC_CCCD, 0x3E4C_CCCD, 0x3E99_999A, 0x3ECC_CCCD, 0x3F00_0000, 0x3F19_999A, 0x3F33_3333];
        let coefs = self.block_coefficients(block)?;
        let (qms, recip) = (f32::from(self.quantize_mult_subblock), 1.0 / f32::from(self.quantize_mult_block));
        let m: [f32; 8] = std::array::from_fn(|k| if k == 0 { 1.0 } else { qms.mul_add(f(TENTHS[k]), 1.0) });
        let mut rows = Vec::with_capacity(8);
        for table in &DCT_IDCT {
            let w: [f32; 8] = std::array::from_fn(|k| m[k] * (f(table[k]) * recip));
            let row = coefs
                .iter()
                .enumerate()
                .map(|(v, c)| {
                    let x = std::array::from_fn(|l| {
                        (1..8).fold(f32::from(c[0][l]) * w[0], |a, k| f32::from(c[k][l]).mul_add(w[k], a))
                    });
                    if v < usize::from(self.num_quats) { normalize(x) } else { x }
                })
                .collect();
            rows.push(row);
        }
        Ok(rows)
    }

    pub fn pose_from(&self, a: &[[f32; 4]], b: &[[f32; 4]], t: f32) -> Pose {
        let (nq, nv) = (usize::from(self.num_quats), usize::from(self.num_vec3));
        let quats = (0..nq).map(|i| nlerp(a[i], b[i], t)).collect();
        let vec3s = (nq..nq + nv).map(|i| lerp3([a[i][0], a[i][1], a[i][2]], [b[i][0], b[i][1], b[i][2]], t)).collect();
        let floats = (0..usize::from(self.num_float))
            .map(|i| {
                let (x, y) = (a[nq + nv + i / 4][i % 4], b[nq + nv + i / 4][i % 4]);
                (y - x).mul_add(t, x)
            })
            .collect();
        Pose { quats, vec3s, floats }
    }

    pub fn key(&self, k: usize) -> Result<Pose> {
        if k >= usize::from(self.num_keys) {
            return invalid(Self::CLASS, format!("no key {k}"));
        }
        let rows = self.decode_block(k / 8)?;
        Ok(self.pose_from(&rows[k % 8], &rows[k % 8], 0.0))
    }

    pub fn stored_key(&self, k: usize) -> Result<Pose> {
        let mut pose = self.key(k)?;
        let rows = self.decode_block(k / 8)?;
        for (i, q) in pose.quats.iter_mut().enumerate() {
            *q = rows[k % 8][i];
        }
        Ok(pose)
    }

    pub fn sample(&self, frame: f32) -> Result<Pose> {
        let (k0, k1, t) = interval(&self.key_times, frame);
        let (r0, r1) = (self.decode_block(k0 / 8)?, self.decode_block(k1 / 8)?);
        Ok(self.pose_from(&r0[k0 % 8], &r1[k1 % 8], t))
    }

    pub fn keys_at(&self, frame: f32) -> (usize, usize, f32) {
        interval(&self.key_times, frame)
    }
}
