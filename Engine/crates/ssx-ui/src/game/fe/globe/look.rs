use std::collections::BTreeMap;

use ssx_assets::track_lighting::LightingPreset;

pub type V3 = [f32; 3];

fn norm(v: V3) -> V3 {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l > 0.0 { [v[0] / l, v[1] / l, v[2] / l] } else { v }
}

fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn lerp3(a: V3, b: V3, t: f32) -> V3 {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}

const DEG: f32 = f32::from_bits(0x3C8E_FA35);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LightState {
    pub key: u32,
    pub name: String,
    pub floats: BTreeMap<String, f32>,
    pub vectors: BTreeMap<String, [f32; 4]>,
    pub textures: BTreeMap<String, String>,
    pub sun: V3,
    pub sun_offset: V3,
    pub bounce: V3,
    pub fill: V3,
    pub rim: V3,
}

impl LightState {
    pub fn of(key: u32, p: &LightingPreset) -> Self {
        let f = |n: &str| p.float(n).unwrap_or(0.0);
        let d = p.vector("dirLight_Direction").unwrap_or([0.0, 1.0, 0.0, 0.0]);
        let sun = norm([d[0], d[1], d[2]]);
        let sun_offset = norm([sun[0], sun[1] + f("dirLight_Offset"), sun[2]]);
        let nl = [-sun_offset[0], -sun_offset[1], -sun_offset[2]];
        let r = norm(cross([0.0, 1.0, 0.0], nl));
        let u = cross(nl, r);
        let a = f("rampBounceOffset") * DEG;
        let bounce = std::array::from_fn(|i| -a.sin() * u[i] + a.cos() * nl[i]);
        let (px, py) = (f("fillOffsetPitchAngle") * DEG, f("fillOffsetYawAngle") * DEG);
        let v = [px.cos() * py.sin(), -px.sin(), px.cos() * py.cos()];
        let fill = std::array::from_fn(|i| v[0] * r[i] + v[1] * u[i] + v[2] * nl[i]);
        let rr = f("rimOffsetAngle") * DEG;
        LightState {
            key,
            name: p.name.clone(),
            floats: p.floats.clone(),
            vectors: p.vectors.clone(),
            textures: p.textures.clone(),
            sun,
            sun_offset,
            bounce,
            fill,
            rim: norm([rr.sin(), rr.cos(), 0.0]),
        }
    }

    pub fn lerp(a: &LightState, b: &LightState, t: f32) -> LightState {
        let floats =
            b.floats.iter().map(|(k, v)| (k.clone(), a.floats.get(k).map_or(*v, |p| p + (v - p) * t))).collect();
        let vectors = b
            .vectors
            .iter()
            .map(|(k, v)| {
                let w = a.vectors.get(k).map_or(*v, |p| std::array::from_fn(|i| p[i] + (v[i] - p[i]) * t));
                (k.clone(), w)
            })
            .collect();
        LightState {
            key: b.key,
            name: b.name.clone(),
            floats,
            vectors,
            textures: b.textures.clone(),
            sun: lerp3(a.sun, b.sun, t),
            sun_offset: lerp3(a.sun_offset, b.sun_offset, t),
            bounce: lerp3(a.bounce, b.bounce, t),
            fill: lerp3(a.fill, b.fill, t),
            rim: lerp3(a.rim, b.rim, t),
        }
    }

    pub fn float(&self, name: &str) -> Option<f32> {
        self.floats.get(name).copied()
    }

    pub fn vector(&self, name: &str) -> Option<[f32; 4]> {
        self.vectors.get(name).copied()
    }

    pub fn preset(&self) -> LightingPreset {
        LightingPreset {
            name: self.name.clone(),
            floats: self.floats.clone(),
            vectors: self.vectors.clone(),
            textures: self.textures.clone(),
            sun_node: None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LightFade {
    pub prev: LightState,
    pub target: LightState,
    pub clock: f32,
    pub duration: f32,
}

const EPS16: f32 = 1.0 / 65536.0;

impl LightFade {
    pub fn set(&mut self, key: u32, preset: &LightingPreset, time: f32) {
        self.prev = self.current();
        self.target = LightState::of(key, preset);
        self.clock = -EPS16;
        self.duration = time + EPS16;
    }

    pub fn step(&mut self, dt: f32) {
        self.clock += dt;
    }

    pub fn alpha(&self) -> f32 {
        if self.duration <= 0.0 { 1.0 } else { (self.clock / self.duration).clamp(0.0, 1.0) }
    }

    pub fn current(&self) -> LightState {
        let a = self.alpha();
        if a >= 1.0 { self.target.clone() } else { LightState::lerp(&self.prev, &self.target, a) }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PostFxLayer {
    pub current: Option<String>,
    pub from: f32,
    pub to: f32,
    pub remaining: f32,
    pub total: f32,
    pub previous: Option<(String, f32)>,
}

impl PostFxLayer {
    fn weight(&self) -> f32 {
        if self.remaining > 0.0 {
            (1.0 - self.remaining / self.total) * (self.to - self.from) + self.from
        } else {
            self.to
        }
    }

    pub fn start(&mut self, name: &str, time: f32, weight: f32) {
        let w = self.weight();
        self.previous = if time <= 0.0 || w <= 0.0 { None } else { self.current.take().map(|n| (n, w)) };
        self.current = Some(name.to_string());
        self.from = 0.0;
        self.to = weight;
        self.remaining = time;
        self.total = time;
    }

    pub fn step(&mut self, dt: f32) {
        if self.remaining > 0.0 {
            self.remaining -= dt;
            if self.remaining <= 0.0 {
                self.remaining = 0.0;
                self.previous = None;
            }
        }
    }

    pub fn weights(&self) -> Vec<(String, f32)> {
        let mut out = Vec::new();
        if self.remaining > 0.0
            && let Some((n, wp)) = &self.previous
        {
            let w = self.remaining / self.total * wp;
            if w > 0.0 {
                out.push((n.clone(), w));
            }
        }
        let w = self.weight();
        if w > 0.0
            && let Some(n) = &self.current
        {
            out.push((n.clone(), w));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preset(key: &str, x: f32) -> LightingPreset {
        let mut p = LightingPreset { name: key.into(), ..Default::default() };
        p.floats.insert("globalLightingMult".into(), x);
        p.vectors.insert("dirLight_Direction".into(), [0.0, 1.0, x, 0.0]);
        p
    }

    #[test]
    fn a_zero_time_change_switches_on_the_next_frame() {
        let mut f = LightFade::default();
        f.set(1, &preset("a", 1.0), 0.0);
        f.step(1.0 / 30.0);
        f.set(2, &preset("b", 3.0), 0.0);
        assert_eq!(f.current().float("globalLightingMult"), Some(1.0));
        f.step(1.0 / 30.0);
        assert_eq!(f.current().float("globalLightingMult"), Some(3.0));
    }

    #[test]
    fn a_timed_change_blends_linearly() {
        let mut f = LightFade::default();
        f.set(1, &preset("a", 1.0), 0.0);
        f.step(0.1);
        f.set(2, &preset("b", 3.0), 1.0);
        f.step(0.5);
        let v = f.current().float("globalLightingMult").unwrap();
        assert!((v - 2.0).abs() < 1e-3, "{v}");
    }

    #[test]
    fn the_layer_keeps_the_previous_preset_while_blending() {
        let mut l = PostFxLayer::default();
        l.start("fe_globe", 0.0, 1.0);
        assert_eq!(l.weights(), vec![("fe_globe".to_string(), 1.0)]);
        l.start("fe_dd", 1.0, 1.0);
        l.step(0.25);
        let w = l.weights();
        assert_eq!(w.len(), 2);
        assert!((w[0].1 - 0.75).abs() < 1e-6 && (w[1].1 - 0.25).abs() < 1e-6, "{w:?}");
        l.step(1.0);
        assert_eq!(l.weights(), vec![("fe_dd".to_string(), 1.0)]);
    }
}
