use super::*;

pub struct Imdct {
    n: usize,
    twiddle: Vec<(f32, f32)>,
    roots: Vec<(f32, f32)>,
    bitrev: Vec<u16>,
    scale: f32,
}

impl Imdct {
    pub fn new(n: usize, scale: f64) -> Self {
        let h = n / 2;
        let bits = h.trailing_zeros();
        let twiddle = (0..h)
            .map(|j| {
                let a = -PI * (j as f64 + 0.125) / n as f64;
                (a.cos() as f32, a.sin() as f32)
            })
            .collect();
        let roots = (0..h / 2)
            .map(|j| {
                let a = -2.0 * PI * j as f64 / h as f64;
                (a.cos() as f32, a.sin() as f32)
            })
            .collect();
        let bitrev = (0..h).map(|j| ((j as u32).reverse_bits() >> (32 - bits)) as u16).collect();
        Self { n, twiddle, roots, bitrev, scale: scale as f32 }
    }

    pub fn run(&self, coef: &[f32], out: &mut [f32]) {
        let (n, h) = (self.n, self.n / 2);
        let mut z = [(0f32, 0f32); 256];
        let z = &mut z[..h];
        for j in 0..h {
            let (a, b) = (coef[2 * j], coef[n - 1 - 2 * j]);
            let (c, s) = self.twiddle[j];
            z[usize::from(self.bitrev[j])] = (a * c - b * s, a * s + b * c);
        }
        let mut len = 2;
        while len <= h {
            let step = h / len;
            for start in (0..h).step_by(len) {
                for k in 0..len / 2 {
                    let (wr, wi) = self.roots[k * step];
                    let (ar, ai) = z[start + k];
                    let (br, bi) = z[start + k + len / 2];
                    let (tr, ti) = (br * wr - bi * wi, br * wi + bi * wr);
                    z[start + k] = (ar + tr, ai + ti);
                    z[start + k + len / 2] = (ar - tr, ai - ti);
                }
            }
            len *= 2;
        }
        for (p, &(vr, vi)) in z.iter().enumerate() {
            let (c, s) = self.twiddle[p];
            let (wr, wi) = (vr * c - vi * s, vr * s + vi * c);
            out[n - 1 - 2 * p] = -wr * self.scale;
            out[2 * p] = wi * self.scale;
        }
    }
}

pub struct Transforms {
    pub imdct: [Imdct; BLOCK_SIZES],
    pub windows: [Vec<f32>; BLOCK_SIZES],
}

pub static TRANSFORMS: LazyLock<Transforms> = LazyLock::new(|| {
    let size = |i: usize| FRAME_SAMPLES >> i;
    let window = |n: usize| (0..n).map(|i| ((i as f64 + 0.5) * PI / (2.0 * n as f64)).sin() as f32).collect();
    Transforms {
        // TODO(guess): the negative sign gives FFmpeg's output polarity (its half-IMDCT is the negated textbook one; ours matched it at −6.02 dB before, i.e. inverted) — polarity cannot be seen in the disc data; settle: a hardware capture or xma2encode's decoder.
        imdct: std::array::from_fn(|i| Imdct::new(size(i), -2.0 / size(i) as f64 / 32768.0)),
        windows: std::array::from_fn(|i| window(size(i))),
    }
});

#[derive(Clone, Debug)]
pub struct BandLayout {
    pub num_bands: [usize; BLOCK_SIZES],
    pub offsets: [[u16; MAX_BANDS + 1]; BLOCK_SIZES],
    pub resample: [[[u8; MAX_BANDS]; BLOCK_SIZES]; BLOCK_SIZES],
}

impl BandLayout {
    pub fn new(rate: u32) -> Self {
        let mut l = Self { num_bands: [0; 3], offsets: [[0; MAX_BANDS + 1]; 3], resample: [[[0; MAX_BANDS]; 3]; 3] };
        for i in 0..BLOCK_SIZES {
            let len = (FRAME_SAMPLES >> i) as u32;
            let mut band = 1;
            for &f in t::CRITICAL_FREQ.iter().take(MAX_BANDS - 1) {
                if u32::from(l.offsets[i][band - 1]) >= len {
                    break;
                }
                let edge = ((2 * len * f) / rate + 2) & !3;
                if edge > u32::from(l.offsets[i][band - 1]) {
                    l.offsets[i][band] = edge as u16;
                    band += 1;
                }
                if edge >= len {
                    break;
                }
            }
            l.offsets[i][band - 1] = len as u16;
            l.num_bands[i] = band - 1;
        }
        for i in 0..BLOCK_SIZES {
            for b in 0..l.num_bands[i] {
                let centre = ((u32::from(l.offsets[i][b]) + u32::from(l.offsets[i][b + 1]) - 1) << i) >> 1;
                for x in 0..BLOCK_SIZES {
                    let mut v = 0;
                    while v + 1 < l.num_bands[x] && (u32::from(l.offsets[x][v + 1]) << x) < centre {
                        v += 1;
                    }
                    l.resample[i][x][b] = v as u8;
                }
            }
        }
        l
    }
}

pub fn rate_of_index(index: u8) -> u32 {
    [24000, 32000, 44100, 48000][usize::from(index.min(3))]
}
