use std::path::Path;

use anyhow::{Context, Result};
use glam::Vec3;
use ssx_formats::attrib::collection;
use ssx_formats::attrib::{Schema, Value, Vault, fnv1};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RippleTuning {
    pub acceleration: f32,
    pub amplitude: f32,
    pub rise: f32,
    pub decay: f32,
    pub falloff: f32,
    pub frequency: f32,
    pub speed: f32,
    pub start: f32,
}

impl RippleTuning {
    pub fn load(big_root: &Path) -> Result<Self> {
        let dir = big_root.join("bootcache/data/xenon/attribdb");
        let (svlt, sbin) = crate::event::read_pair(&dir.join("attribdb_schema.vlt"))?;
        let schema = Schema::from_vault(&Vault::parse(&svlt, &sbin)?).context("reading the schema")?;
        let (v, b) = crate::event::read_pair(&dir.join("attribdb.vlt"))?;
        let vault = Vault::parse(&v, &b).context("parsing attribdb.vlt")?;
        let cols = collection::collections(&vault)?;
        let i = cols
            .iter()
            .position(|c| c.class == fnv1(b"ripple") && c.key == fnv1(b"default"))
            .context("no ripple/default collection")?;
        let fields = cols[i].decode(&vault, &schema)?;
        let f = |key: u32| {
            fields
                .field(key)
                .and_then(|x| {
                    crate::event::values(&x.data).into_iter().find_map(|v| match v {
                        Value::Float(f) => Some(*f),
                        _ => None,
                    })
                })
                .unwrap_or(0.0)
        };
        Ok(Self {
            acceleration: f(fnv1(b"acceleration")),
            amplitude: f(fnv1(b"amplitude")),
            rise: f(0xE333_2CA3),
            decay: f(fnv1(b"decay")),
            falloff: f(fnv1(b"falloff")),
            frequency: f(fnv1(b"frequency")),
            speed: f(fnv1(b"speed")),
            start: f(fnv1(b"start")),
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RippleParams {
    pub amplitude: f32,
    pub falloff: f32,
    pub frequency: f32,
    pub phase: f32,
    pub position: Vec3,
    pub start: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Ripple {
    pub position: Vec3,
    pub time: f32,
    pub active: bool,
}

impl Ripple {
    pub fn start(&mut self, position: Vec3) {
        *self = Ripple { position, time: 0.0, active: true };
    }

    pub fn update(&mut self, t: &RippleTuning, dt: f32) -> RippleParams {
        if self.active {
            self.time += dt;
        }
        let time = self.time;
        let r = time / t.rise;
        let s = if r - 1.0 >= 0.0 { 1.0 } else { r };
        let a = t.amplitude * s;
        let f =
            if time > t.rise { (f64::from(time - t.rise + 1.0)).powf(f64::from(t.decay)) as f32 - 1.0 } else { 0.0 };
        let (speed, accel) = (t.speed * s, t.acceleration * s);
        if a < f {
            self.active = false;
        }
        RippleParams {
            amplitude: if self.active { a - f } else { 0.0 },
            falloff: t.falloff,
            frequency: t.frequency,
            phase: (time * accel).mul_add(0.5, speed) * time,
            position: self.position,
            start: t.start,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DISC: RippleTuning = RippleTuning {
        acceleration: 200.0,
        amplitude: 3.0,
        rise: 0.051,
        decay: 2.0,
        falloff: 0.15,
        frequency: 0.15,
        speed: 10.0,
        start: 5.0,
    };

    #[test]
    fn rise_decay_and_end() {
        let mut r = Ripple::default();
        assert_eq!(r.update(&DISC, 1.0 / 60.0).amplitude, 0.0);
        r.start(Vec3::new(1.0, 2.0, 3.0));
        let p = r.update(&DISC, 0.0255);
        assert!((p.amplitude - 1.5).abs() < 1e-5);
        let mut last = p;
        while r.active {
            last = r.update(&DISC, 1.0 / 60.0);
        }
        assert_eq!(last.amplitude, 0.0);
        assert!((r.time - 1.051).abs() < 0.02, "ends at {}", r.time);
        let mut r2 = Ripple::default();
        r2.start(Vec3::ZERO);
        let q = r2.update(&DISC, 0.5);
        assert!((q.phase - (10.0 * 0.5 + 0.5 * 200.0 * 0.25)).abs() < 1e-3);
        assert_eq!((q.position, q.start, q.falloff), (Vec3::ZERO, 5.0, 0.15));
    }
}
