use bevy::math::{Vec3, Vec4};

use super::{Context, Particle, RNG_SEEDS, Rng, View, unit_float, xorshift};

#[derive(Clone, Copy, Debug)]
pub struct ActionPrivate {
    gust_target: Vec3,
    gust_current: Vec3,
    last_gust_ms: i32,
    stripe: usize,
    turb_b: Vec4,
    turb_c: Vec4,
}

impl Default for ActionPrivate {
    fn default() -> Self {
        ActionPrivate {
            gust_target: Vec3::ZERO,
            gust_current: Vec3::ZERO,
            last_gust_ms: 0,
            stripe: 0,
            turb_b: Vec4::new(1.0, 2.0, 3.0, 0.0),
            turb_c: Vec4::ZERO,
        }
    }
}

pub(super) fn weather(p: &View, ps: &mut [Particle], ctx: &Context, st: &mut ActionPrivate, rng: &mut Rng) {
    let w = ctx.weather.wind * p.f("fWeatherScalar");
    let interval = p.i("iGustIntervalMs");
    let mut stripes = (interval as f32 / ctx.dt_smoothed * 0.001 * p.f("fFieldFraction") + 0.5) as i64;
    if stripes <= 2 {
        stripes = 1;
    }
    if ctx.time_ms - st.last_gust_ms > interval {
        st.gust_target = p.v3("vGustVariance") * rng.spread4().truncate();
        st.last_gust_ms = ctx.time_ms;
    }
    st.gust_current += (st.gust_target - st.gust_current) * p.f("fGustChangeGain");
    let n = stripes as usize;
    st.stripe = (st.stripe + 1) % n;
    let (amp, adopt) = (p.v3("vTurbulentAmplitude"), p.f("fAdoption"));
    let drift = (w + st.gust_current) * adopt;
    for chunk in (0..ps.len()).step_by(64) {
        let end = (chunk + 64).min(ps.len());
        for q in ps[chunk..end].iter_mut().skip(st.stripe).step_by(n) {
            let r = rng.spread4().truncate();
            q.pos += (amp * r + drift) * ctx.dt;
        }
    }
}

pub(super) fn vec_cos_approx(x: f32) -> f32 {
    const C: [f32; 12] = [
        1.0,
        -0.5,
        0.041_666_668,
        -0.001_388_888_9,
        2.480_158_8e-5,
        -2.755_732e-7,
        2.087_675_8e-9,
        -1.147_074_5e-11,
        4.779_477_3e-14,
        -1.561_920_7e-16,
        4.110_317_6e-19,
        -8.896_791e-22,
    ];
    let n = (x * 0.159_154_94).round_ties_even();
    let r = (-6.283_185_5f32).mul_add(n, x);
    let x2 = r * r;
    let x4 = x2 * x2;
    let mut v = C[1].mul_add(x2, C[0]);
    let x6 = x4 * x2;
    v = C[2].mul_add(x4, v);
    let x8 = x4 * x4;
    let x10 = x6 * x4;
    v = C[3].mul_add(x6, v);
    let (x12, x14, x16, x18) = (x6 * x6, x8 * x6, x8 * x8, x10 * x8);
    v = C[4].mul_add(x8, v);
    let (x22, x20) = (x12 * x10, x10 * x10);
    for (c, p) in C[5..].iter().zip([x10, x12, x14, x16, x18, x20, x22]) {
        v = c.mul_add(p, v);
    }
    v
}

pub(super) fn turbulence_table() -> &'static [f32; 256] {
    static TABLE: std::sync::OnceLock<[f32; 256]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        std::array::from_fn(|i| {
            let c = vec_cos_approx(i as f32 * 0.024_543_693);
            c * (((c * c) * c) * c)
        })
    })
}

pub(super) fn turbulence(p: &View, ps: &mut [Particle], ctx: &Context, st: &mut ActionPrivate, rng: &mut Rng) {
    let (mag, freq, noise, scale, twist, curl) =
        (p.f("fMagnitude"), p.f("fFrequency"), p.f("fAmountNoise"), p.f("fScale"), p.f("fTwist"), p.f("fCurl"));
    let dt = ctx.dt;
    let k = dt * twist * freq;
    if k > 0.0 && dt > 1.0 / 6000.0 {
        let s = rng.next4() - Vec4::new(0.5, 0.5, 0.5, 0.0);
        let c = (st.turb_c + s * k).truncate().normalize_or_zero() * k;
        st.turb_c = c.extend(0.0);
        st.turb_b = (st.turb_b.truncate().normalize_or_zero() + c * k).extend(0.0);
    }
    let table = turbulence_table();
    let t = freq * ctx.time_ms as f32 * 0.001;
    let pi2 = std::f32::consts::PI * std::f32::consts::PI;
    for q in ps.iter_mut() {
        let lanes = [q.birth * pi2, q.birth * pi2 * pi2, q.birth * pi2 * pi2 * pi2];
        let h = lanes.map(|x| unit_float(xorshift(x.to_bits())) + 1.0 - 1.5);
        let idx = |l: usize, pl: f32, b: f32| {
            let c = pl / scale + b + t + noise * h[l];
            let f = c - c.floor();
            table[((f.to_bits() >> 15) & 0xFF) as usize]
        };
        let nv = Vec3::new(idx(0, q.pos.x, st.turb_b.x), idx(1, q.pos.y, st.turb_b.y), idx(2, q.pos.z, st.turb_b.z));
        let speed = q.vel.length();
        let v = q.vel + nv * mag * dt + nv.cross(q.vel) * curl * dt;
        q.vel = v.normalize_or_zero() * speed;
    }
}

pub(super) fn periodic_box(p: &View, ps: &mut [Particle]) {
    let d = p.v3("vDimensions");
    let h = d * 0.5;
    let wrap = |x: f32, d: f32, h: f32, inverted: bool| {
        if !inverted {
            let p1 = if x >= h { x - d } else { x };
            if -h >= p1 { x + d } else { p1 }
        } else {
            let p1 = if x >= h { x } else { x - d };
            if -h >= p1 { p1 } else { x + d }
        }
    };
    for chunk in (0..ps.len()).step_by(64) {
        let end = (chunk + 64).min(ps.len());
        let tail = end - (end - chunk) % 4;
        for (i, q) in ps[chunk..end].iter_mut().enumerate() {
            let inv = chunk + i >= tail;
            q.pos = Vec3::new(wrap(q.pos.x, d.x, h.x, inv), wrap(q.pos.y, d.y, h.y, inv), wrap(q.pos.z, d.z, h.z, inv));
        }
    }
}

pub(super) fn accelerate(p: &View, ps: &mut [Particle], dt: f32) {
    let (a, var) = (p.v3("vAcceleration"), p.v3("vAccelVariance"));
    for q in ps {
        let acc = if var == Vec3::ZERO {
            a
        } else {
            let b = q.birth.to_bits();
            let h = |k: usize| unit_float(xorshift(b.wrapping_add(1).wrapping_add(RNG_SEEDS[k]))) - 0.5;
            a + var * Vec3::new(h(0), h(1), h(2))
        };
        q.vel += acc * dt;
    }
}

pub(super) fn drag(p: &View, ps: &mut [Particle], dt: f32) {
    if dt == 0.0 {
        return;
    }
    let (c, wind, sd, power) =
        (p.f("fDragCoefficient"), p.v3("vAmbientWind"), p.f("fSizeDependence"), p.i("iSpeedPower"));
    let kmax = (1.0 / dt) * 0.99;
    for q in ps {
        let rel = wind - q.vel;
        let m = if power == 3 { rel.length_squared() } else { rel.length() };
        let mut k = m * c;
        if sd != 0.0 {
            k = (1.0 - sd) * k + sd * k * q.size * q.inv_mass;
        }
        q.vel += rel * (k.min(kmax) * dt);
    }
}

pub(super) fn speed_limit(p: &View, ps: &mut [Particle]) {
    let (lo, hi) = (p.f("fSpeedMin"), p.f("fSpeedMax"));
    for q in ps {
        let s = q.vel.length() + 1e-8;
        if lo > s {
            q.vel *= lo / s;
        } else if s > hi {
            q.vel *= hi / s;
        }
    }
}

pub(super) fn kill_age(p: &View, ps: &mut Vec<Particle>, time_ms: i32) {
    let (t, max) = (time_ms as f32, p.i("iAgeMs") as f32);
    match p.i("iOrdering") {
        1 => {
            let n = ps.iter().take_while(|q| t - q.birth > max || t - q.birth < 0.0).count();
            ps.drain(..n);
        }
        2 => {
            let n = ps.iter().filter(|q| t - q.birth > max || t - q.birth < 0.0).count();
            ps.drain(..n.min(ps.len()));
        }
        0 => {
            for i in (0..ps.len()).rev() {
                let q = ps[i];
                let dead = !((t - q.birth) - max < 0.0 && (q.birth - t) - 0.1 < 0.0);
                if dead {
                    ps.swap_remove(i);
                }
            }
        }
        _ => {}
    }
}
