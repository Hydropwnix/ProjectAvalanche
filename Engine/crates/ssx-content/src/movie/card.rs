#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Card {
    pub width: u32,
    pub height: u32,
    pub fps: (u32, u32),
    pub seconds: u32,
    pub rate: u32,
    pub channels: u16,
}

impl Card {
    pub fn pictures(&self) -> u64 {
        u64::from(self.seconds) * u64::from(self.fps.0) / u64::from(self.fps.1)
    }

    pub fn time(&self, k: u64) -> f64 {
        k as f64 * f64::from(self.fps.1) / f64::from(self.fps.0)
    }

    pub fn is_mark(&self, k: u64) -> bool {
        let t = self.time(k);
        let prev = if k == 0 { -1.0 } else { self.time(k - 1) };
        t.floor() > prev.floor() || k == 0
    }

    pub fn scene(&self) -> Scene {
        let h = self.height as usize;
        let mut band = vec![0u8; TILE * h * 3];
        for y in 0..h {
            for x in 0..TILE {
                let (fx, fy) = (x as f64 / 64.0, y as f64 / 64.0);
                let n = 0.6 * noise(fx, fy, 32)
                    + 0.3 * noise(fx * 4.0, fy * 4.0, 128)
                    + 0.1 * noise(fx * 16.0, fy * 16.0, 512);
                let sky = y as f64 / h as f64;
                let rgb = [n * 200.0 + 30.0 * sky, n * 180.0 + 60.0 * (1.0 - sky), 120.0 + n * 100.0];
                for (c, v) in rgb.into_iter().enumerate() {
                    band[(y * TILE + x) * 3 + c] = v.clamp(0.0, 255.0) as u8;
                }
            }
        }
        Scene { card: *self, band }
    }

    pub fn audio(&self, from: u64, to: u64) -> Vec<f32> {
        let rate = f64::from(self.rate);
        let ch = usize::from(self.channels);
        let mut out = Vec::with_capacity((to.saturating_sub(from)) as usize * ch);
        for i in from..to {
            let t = i as f64 / rate;
            let bed = 0.05 * ((t * 220.0 * std::f64::consts::TAU).sin() + (t * 277.18 * std::f64::consts::TAU).sin());
            let mark = if t.fract() < 0.1 { 0.5 * (t * 1000.0 * std::f64::consts::TAU).sin() } else { 0.0 };
            out.extend(std::iter::repeat_n((bed + mark) as f32, ch));
        }
        out
    }
}

const TILE: usize = 2048;

pub struct Scene {
    pub card: Card,
    band: Vec<u8>,
}

impl Scene {
    pub fn picture(&self, k: u64, out: &mut [u8]) {
        let c = &self.card;
        let (w, h) = (c.width as usize, c.height as usize);
        if c.is_mark(k) {
            out[..w * h * 3].fill(255);
            return;
        }
        let t = c.time(k);
        let scroll = (t * 120.0) as usize;
        for y in 0..h {
            for x in 0..w {
                let from = (y * TILE + (x + scroll) % TILE) * 3;
                out[(y * w + x) * 3..(y * w + x) * 3 + 3].copy_from_slice(&self.band[from..from + 3]);
            }
        }
        for i in 0..3 {
            let a = t * (0.6 + 0.25 * i as f64) + i as f64 * 2.1;
            let (cx, cy) = (w as f64 * (0.5 + 0.3 * a.cos()), h as f64 * (0.5 + 0.3 * (1.3 * a).sin()));
            let r = h as f64 * (0.06 + 0.02 * i as f64);
            let colour = [[230, 60, 40], [40, 200, 90], [50, 80, 230]][i];
            let (y0, y1) = ((cy - r).max(0.0) as usize, ((cy + r) as usize + 1).min(h));
            let (x0, x1) = ((cx - r).max(0.0) as usize, ((cx + r) as usize + 1).min(w));
            for y in y0..y1 {
                for x in x0..x1 {
                    if (x as f64 - cx).hypot(y as f64 - cy) < r {
                        out[(y * w + x) * 3..(y * w + x) * 3 + 3].copy_from_slice(&colour);
                    }
                }
            }
        }
    }
}

fn noise(x: f64, y: f64, period: i64) -> f64 {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let h = |i: f64, j: f64| {
        let i = (i as i64).rem_euclid(period) as f64;
        let mut v = (i as i64).wrapping_mul(0x9E37_79B9_7F4A_7C15_u64 as i64)
            ^ (j as i64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F_u64 as i64);
        v ^= v >> 29;
        v = v.wrapping_mul(0x1656_67B1_9E37_79F9);
        ((v >> 11) & 0xFFFF) as f64 / 65535.0
    };
    let a = h(x0, y0) + (h(x0 + 1.0, y0) - h(x0, y0)) * sx;
    let b = h(x0, y0 + 1.0) + (h(x0 + 1.0, y0 + 1.0) - h(x0, y0 + 1.0)) * sx;
    a + (b - a) * sy
}
