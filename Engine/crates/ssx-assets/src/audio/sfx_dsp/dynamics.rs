use super::*;

pub fn distortion_clip(level: f64, chans: &mut [Vec<f32>]) {
    if level >= 100.0 {
        return;
    }
    let (lo, hi) = (-level as f32, level as f32);
    for v in chans.iter_mut().flatten() {
        *v = v.clamp(lo, hi);
    }
}

#[derive(Clone, Debug, Default)]
pub struct Compressor {
    env: Vec<f32>,
    gain: Vec<f32>,
    active: bool,
    pub settled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CompressorSetup {
    threshold: f32,
    slope: f32,
    detector: f32,
    attack: f32,
    release: f32,
    linked: bool,
}

impl CompressorSetup {
    pub fn new(threshold_db: f64, ratio: f64, attack: f64, release: f64, mode: i32) -> Self {
        let rate = f64::from(RATE);
        let round = |s: f64| {
            let v = (s.clamp(0.0, 1e9) * rate) as f32;
            ((if v < 0.0 { v - 0.5 } else { v + 0.5 }) as i32).max(1)
        };
        let (att, rel) = (round(attack.clamp(0.0, 10.0)), round(release.clamp(0.0, 30.0)));
        let threshold = 10f64.powf(f64::from((threshold_db.max(-500.0) * 0.05) as f32)) as f32;
        let slope = (1.0 / ratio) as f32 - 1.0;
        let att_ms = (f64::from(att) / rate) as f32 * 1000.0;
        let tau = if att_ms <= 75.0 { att as f32 * 0.2 } else { (rate * 0.015) as f32 };
        let e = std::f64::consts::E;
        let detector = if tau > 0.0 { e.powf(f64::from(-2.2 / tau)) as f32 } else { 0.0 };
        let attack = e.powf(f64::from(-2.2f32 / att as f32)) as f32;
        let release = e.powf(f64::from(-2.2f32 / rel as f32)) as f32;
        Self { threshold, slope, detector, attack, release, linked: mode == 1 }
    }
}

impl Compressor {
    pub fn process(
        &mut self,
        threshold_db: f64,
        ratio: f64,
        setup: &CompressorSetup,
        chans: &mut [Vec<f32>],
        side: Option<&[Vec<f32>]>,
    ) {
        let n = chans.len();
        if threshold_db >= 20.0 || ratio <= 1.0 {
            self.rest(threshold_db, ratio);
            return;
        }
        self.active = true;
        self.env.resize(n, 0.0);
        self.gain.resize(n, 0.0);
        let before = |v: &[f32]| v.iter().fold(0u64, |h, x| h.rotate_left(7) ^ u64::from(x.to_bits()));
        let was = (before(&self.env), before(&self.gain));
        let len = chans.first().map_or(0, Vec::len);
        let c = setup.detector;
        let one_c = 1.0 - c;
        let inv_thr = 1.0 / setup.threshold;
        // TODO(guess): the kernel computes the power with the VMX estimates vlogefp / vexptefp (their Xenon precision unknown), the port with powf; settle: a hardware capture of 0x82A27358's output.
        let mut curves = vec![vec![1.0f32; len]; n];
        let (envs, gains) = (&mut self.env, &mut self.gain);
        fused(
            #[inline(always)]
            |f| {
                for ch in 0..n {
                    let input = side.and_then(|s| s.get(ch)).unwrap_or(&chans[ch]);
                    let mut env = envs[ch];
                    for (k, x) in input.iter().enumerate().take(len) {
                        env = madd(f, x.abs(), one_c, c * env) + 1e-18;
                        if env > setup.threshold {
                            curves[ch][k] = (env * inv_thr).powf(setup.slope);
                        }
                    }
                    envs[ch] = env;
                }
                #[inline(always)]
                fn smooth(f: bool, setup: &CompressorSetup, g: &mut f32, curve: &mut [f32]) {
                    for x in curve.iter_mut() {
                        let a = if *x < *g { setup.attack } else { setup.release };
                        *g = madd(f, a, *g, madd(f, 1.0 - a, *x, 1e-18));
                        *x = *g;
                    }
                }
                if setup.linked {
                    let mut shared = curves[0].clone();
                    for c in &curves[1..] {
                        shared.iter_mut().zip(c).for_each(|(s, v)| *s = s.min(*v));
                    }
                    smooth(f, setup, &mut gains[n - 1], &mut shared);
                    for ch in chans.iter_mut() {
                        ch.iter_mut().zip(&shared).for_each(|(x, g)| *x *= g);
                    }
                } else {
                    for (ch, (buf, curve)) in chans.iter_mut().zip(curves.iter_mut()).enumerate() {
                        smooth(f, setup, &mut gains[ch], curve);
                        buf.iter_mut().zip(curve.iter()).for_each(|(x, g)| *x *= g);
                    }
                }
            },
        );
        self.settled = was == (before(&self.env), before(&self.gain));
    }

    pub fn rest(&mut self, threshold_db: f64, ratio: f64) {
        let bypass = threshold_db >= 20.0 || ratio <= 1.0;
        if bypass && self.active {
            self.env.iter_mut().chain(self.gain.iter_mut()).for_each(|v| *v = 0.0);
        }
        self.active = !bypass;
        self.settled |= bypass;
    }
}
