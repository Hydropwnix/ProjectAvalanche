use super::*;

const MATRIX: [[&[u8]; 8]; 8] = [
    [
        &[0x00],
        &[0x00, 0x08],
        &[],
        &[0x00, 0x08, 0x10, 0x18],
        &[],
        &[0x08, 0x10, 0x20, 0x18, 0x00],
        &[],
        &[0x08, 0x00, 0x18, 0x28, 0x10, 0x20, 0x30],
    ],
    [
        &[0x40, 0x41],
        &[0x00, 0x09],
        &[],
        &[0x00, 0x09, 0x10, 0x19],
        &[],
        &[0x48, 0x00, 0x18, 0x49, 0x11, 0x21],
        &[],
        &[0x48, 0x00, 0x18, 0x28, 0x49, 0x11, 0x21, 0x31],
    ],
    [&[]; 8],
    [
        &[0x40, 0x41],
        &[0x00, 0x09],
        &[],
        &[0x00, 0x09, 0x12, 0x1b],
        &[],
        &[0x48, 0x00, 0x1a, 0x49, 0x11, 0x23],
        &[],
        &[0x48, 0x00, 0x1a, 0x2a, 0x49, 0x11, 0x23, 0x33],
    ],
    [&[]; 8],
    [
        &[0x01],
        &[0x00, 0x0a],
        &[],
        &[0x00, 0x0a, 0x13, 0x1c],
        &[],
        &[0x00, 0x09, 0x12, 0x1b, 0x24, 0x2d],
        &[],
        &[0x09, 0x00, 0x1b, 0x2b, 0x12, 0x24, 0x34, 0x3d],
    ],
    [&[]; 8],
    [
        &[0x01],
        &[0x00, 0x0a],
        &[],
        &[0x00, 0x0a, 0x13, 0x1c],
        &[],
        &[0x09, 0x00, 0x1b, 0x12, 0x24, 0x2f],
        &[],
        &[0x00, 0x09, 0x12, 0x1b, 0x24, 0x2d, 0x36, 0x3f],
    ],
];
const MATRIX_GAINS: [f32; 4] = [1.0, 0.707, 0.5, 0.0];

pub fn channel_terms(inputs: usize, outputs: usize) -> Vec<(usize, usize, f32)> {
    if !(1..=8).contains(&inputs) || !(1..=8).contains(&outputs) {
        return Vec::new();
    }
    MATRIX[outputs - 1][inputs - 1]
        .iter()
        .map(|&e| (usize::from(e >> 3 & 7), usize::from(e & 7), MATRIX_GAINS[usize::from(e >> 6)]))
        .collect()
}

pub fn rechannel(input: &[Vec<f32>], outputs: usize) -> Vec<Vec<f32>> {
    if input.len() == outputs {
        return input.to_vec();
    }
    let n = input.first().map_or(0, Vec::len);
    let mut out = vec![vec![0.0; n]; outputs];
    fused(
        #[inline(always)]
        |f| {
            for (s, t, g) in channel_terms(input.len(), outputs) {
                for (o, i) in out[t].iter_mut().zip(&input[s]) {
                    *o = madd(f, g, *i, *o);
                }
            }
        },
    );
    out
}

pub fn send_into(input: &[Vec<f32>], target: &mut [Vec<f32>], old: f32, new: f32) {
    fused(
        #[inline(always)]
        |f| {
            for (s, t, g) in channel_terms(input.len(), target.len()) {
                let (Some(src), Some(dst)) = (input.get(s), target.get_mut(t)) else { continue };
                if old == new {
                    let gain = g * new;
                    for (o, i) in dst.iter_mut().zip(src) {
                        *o = madd(f, gain, *i, *o);
                    }
                } else {
                    for (k, (o, i)) in dst.iter_mut().zip(src).enumerate() {
                        *o = madd(f, g * ramp_gain_in(f, k, old, new), *i, *o);
                    }
                }
            }
        },
    )
}

pub fn route_into(input: &[Vec<f32>], target: &mut [Vec<f32>], window: (usize, usize, usize), old: f32, new: f32) {
    let src = window.0.min(input.len());
    let dst = window.1.min(target.len());
    let n = window.2.min(input.len() - src).min(target.len() - dst);
    fused(
        #[inline(always)]
        |f| {
            for c in 0..n {
                for (k, (o, i)) in target[dst + c].iter_mut().zip(&input[src + c]).enumerate() {
                    let gain = if old == new { new } else { ramp_gain_in(f, k, old, new) };
                    *o = madd(f, gain, *i, *o);
                }
            }
        },
    )
}

#[derive(Clone, Debug, Default)]
pub struct Resampler {
    pub index: u64,
    pub frac: u32,
}

const FRAC_SCALE: f32 = f32::from_bits(0x377F_FC9C);

impl Resampler {
    pub fn step(ratio: f32) -> u32 {
        let f = ratio * 65536.0;
        let i = (if f < 0.0 { f - 0.5 } else { f + 0.5 }) as i32;
        i.clamp(0, 0x40000) as u32
    }

    pub fn sample(&self, a: f32, b: f32) -> f32 {
        self.sample_in(true, a, b)
    }

    #[inline(always)]
    pub fn sample_in(&self, f: bool, a: f32, b: f32) -> f32 {
        let t = self.frac as f32 * FRAC_SCALE;
        madd(f, b - a, t, a)
    }

    pub fn advance(&mut self, step: u32) {
        let p = self.frac + step;
        self.index += u64::from(p >> 16);
        self.frac = p & 0xFFFF;
    }
}
