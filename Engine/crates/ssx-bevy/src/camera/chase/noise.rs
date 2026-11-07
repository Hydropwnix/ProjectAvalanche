use bevy::prelude::{Quat, Resource, Vec3, info, warn};

use super::{Chase, NoiseSignals, Wipeout};

pub fn load_noise(root: &std::path::Path) -> Option<CameraNoise> {
    match CameraNoise::load(root) {
        Ok(clips) => {
            info!(
                "chase camera: noise layers (shake {} frames, carving {} / {} / {}, wipeout {}){}",
                clips.shake.frames.len(),
                clips.carving[0].frames.len(),
                clips.carving[1].frames.len(),
                clips.carving[2].frames.len(),
                clips.wipeout.frames.len(),
                if clips.normal_layers { "" } else { "; the normal camera's are off (SSX_CAMERA_NOISE=off)" }
            );
            Some(clips)
        }
        Err(e) => {
            warn!("chase camera: the camera noise clips are unavailable: {e:#}");
            None
        }
    }
}

#[derive(Resource)]
pub struct CameraNoise {
    shake: EyeClip,
    carving: [EyeClip; 3],
    wipeout: EyeClip,
    normal_layers: bool,
}

struct EyeClip {
    frames: Vec<(Quat, Vec3)>,
}

impl CameraNoise {
    pub fn load(big_root: &std::path::Path) -> anyhow::Result<CameraNoise> {
        use anyhow::Context as _;
        use ssx_formats::ant::Value;
        use ssx_formats::ant_anim::{Animation, ChannelToDof, dof_list, normalize};
        let s = ssx_assets::content::data::tuning::anim_all(big_root)?;
        let idx = s.guid_index();
        let get = |o: &ssx_formats::ant::Object, f: &str| {
            o.get(&s.schema, f).and_then(Value::as_guid).and_then(|g| idx.get(&g)).map(|&i| &s.assets[i].object)
        };
        let dofs = dof_list(&s, "camera_rig", "AnimRig")?;
        let dof = |n: &str| dofs.iter().position(|d| d.name == n).with_context(|| format!("no DOF {n}"));
        let (eye_q, eye_t) = (dof("CameraEye.q")?, dof("CameraEye.t")?);
        let clip = |name: &str| -> anyhow::Result<EyeClip> {
            let clip = s.find(name).with_context(|| format!("no {name}"))?;
            let ticks = clip.object.get(&s.schema, "NumTicks").and_then(Value::as_f32).context("no NumTicks")?;
            let anim_obj = get(&clip.object, "Anim").context("clip without Anim")?;
            let anim = Animation::from_object(&s.schema, anim_obj)?.context("Anim is no animation")?;
            let c2d = ChannelToDof::parse(&s.schema, get(anim_obj, "ChannelToDofAsset").context("no ChannelToDof")?)?;
            let channel = |d: usize| c2d.indices.iter().position(|&i| usize::from(i) == d).context("DOF not animated");
            let (qc, tc) = (channel(eye_q)?, channel(eye_t)?);
            let nq = anim.counts().quats;
            let frames = (0..=ticks.round() as usize)
                .map(|f| {
                    let p = anim.sample(f as f32)?;
                    let q = Quat::from_array(normalize(*p.quats.get(qc).context("quat channel")?));
                    let t = Vec3::from_array(*p.vec3s.get(tc.wrapping_sub(nq)).context("vec3 channel")?);
                    Ok((q, t))
                })
                .collect::<anyhow::Result<Vec<_>>>()?;
            Ok(EyeClip { frames })
        };
        Ok(CameraNoise {
            shake: clip("camera_noise_avalanche_shake")?,
            carving: [
                clip("camera_noise_carving_slow")?,
                clip("camera_noise_carving_fast")?,
                clip("camera_noise_carving_boost")?,
            ],
            wipeout: clip("camera_noise_wipeout_mid")?,
            normal_layers: !crate::render::post::env_is("SSX_CAMERA_NOISE", "off"),
        })
    }
}

impl EyeClip {
    fn sample(&self, t: f32) -> (Quat, Vec3) {
        let n = (self.frames.len() - 1) as f32;
        let t = t.clamp(0.0, n);
        let (k, f) = (t.floor() as usize, t.fract());
        let ((qa, ta), (qb, tb)) = (self.frames[k], self.frames[(k + 1).min(self.frames.len() - 1)]);
        (nlerp(qa, qb, f), ta.lerp(tb, f))
    }
}

pub(super) fn blend_eye(a: (Quat, Vec3), b: (Quat, Vec3), w: f32) -> (Quat, Vec3) {
    (nlerp(a.0, b.0, w), a.1.lerp(b.1, w))
}

fn nlerp(a: Quat, b: Quat, w: f32) -> Quat {
    let b = if a.dot(b) < 0.0 { -b } else { b };
    a.lerp(b, w).normalize()
}

#[derive(Clone, Copy, Debug)]
pub(super) struct CarvingNoise {
    cur: f32,
    phase: f32,
}

const CARVING_ITEMS: [(f32, f32); 4] = [(0.4, 60.0), (0.6, 60.0), (0.85, 60.0), (1.0, 30.0)];

fn select(pos: f32) -> ([(usize, f32); 2], usize) {
    let xs = CARVING_ITEMS.map(|i| i.0);
    let n = xs.len();
    if pos <= xs[0] {
        return ([(0, 1.0), (0, 0.0)], 1);
    }
    if pos >= xs[n - 1] {
        return ([(n - 1, 1.0), (0, 0.0)], 1);
    }
    for i in 0..n - 1 {
        let (a, b) = (xs[i], xs[i + 1]);
        if a - 0.001 <= pos && pos <= b + 0.001 {
            if (pos - a).abs() < 0.001 {
                return ([(i, 1.0), (0, 0.0)], 1);
            }
            let t = (pos - a) / (b - a);
            return if t < 0.03 {
                ([(i, 1.0), (0, 0.0)], 1)
            } else if t > 0.97 {
                ([(i + 1, 1.0), (0, 0.0)], 1)
            } else {
                ([(i, 1.0 - t), (i + 1, t)], 2)
            };
        }
    }
    ([(n - 1, 1.0), (0, 0.0)], 1)
}

impl CarvingNoise {
    pub(super) fn new(signal: f32) -> Self {
        CarvingNoise { cur: signal.clamp(0.0, 1.0), phase: 0.0 }
    }

    fn active(&self) -> ([(usize, f32); 2], usize) {
        select(self.cur.clamp(0.4, 1.0))
    }

    fn length(&self) -> f32 {
        let (sel, n) = self.active();
        sel[..n].iter().map(|&(i, w)| w * CARVING_ITEMS[i].1).sum()
    }

    fn advance(&mut self, signal: f32, dt: f32) {
        let diff = signal - self.cur;
        self.cur = (self.cur + diff.signum() * diff.abs().min(dt)).clamp(0.0, 1.0);
        let l = self.length();
        if l > 0.0 {
            self.phase = (self.phase + dt / l).min(1.0);
        }
    }

    fn update(&mut self, signal: f32, dt: f32) {
        let rem = (1.0 - self.phase) * self.length();
        if rem <= dt && self.length() > 0.0 {
            self.advance(signal, rem);
            self.phase = 0.0;
            self.advance(signal, dt - rem);
        } else {
            self.advance(signal, dt);
        }
    }

    fn pose(&self, clips: &CameraNoise) -> (Quat, Vec3) {
        let (sel, n) = self.active();
        let at = |i: usize| clips.carving[i.min(2)].sample(60.0 * self.phase);
        let first = at(sel[0].0);
        if n == 2 { blend_eye(first, at(sel[1].0), sel[1].1) } else { first }
    }
}

impl Chase {
    pub(super) fn shake(&mut self, clip: Option<&CameraNoise>, w: f32, dt: f32) {
        const EPS: f32 = 1.5e-5;
        let Some(clip) = clip else { return };
        if self.shake_time.is_none() && w > EPS {
            self.shake_time = Some(0.0);
        }
        let Some(t) = self.shake_time.as_mut() else { return };
        *t = (*t + dt).rem_euclid(90.0);
        if w > EPS {
            let (q, p) = clip.shake.sample(*t);
            let (eq, et) = self.eye;
            let q = if eq.dot(q) < 0.0 { -q } else { q };
            self.eye = (eq.lerp(q, w).normalize(), et.lerp(p, w));
            if crate::render::post::env_is("SSX_AVALANCHE_LOG", "on") {
                let deg = 2.0 * self.eye.0.w.clamp(-1.0, 1.0).acos().to_degrees();
                info!("avalanche shake: clip tick {t:.0}, CameraEye {:.3} m, {deg:.2} deg", self.eye.1);
            }
        }
    }

    pub(super) fn noise(&mut self, clips: Option<&CameraNoise>, sig: NoiseSignals, wipeout_layer: Wipeout, dt: f32) {
        const EPS: f32 = 1.5e-5;
        let Some(clips) = clips.filter(|c| c.normal_layers) else { return };
        let (w1, carving_weight) = sig.carving;
        self.carving.update(carving_weight, dt);
        if w1.abs() > EPS {
            self.eye = blend_eye(self.eye, self.carving.pose(clips), w1);
        }
        let (w2, wipeout_weight) = if wipeout_layer == Wipeout::None { (0.0, 0.0) } else { sig.wipeout };
        if self.wipeout_time.is_none() && (w2 > EPS || wipeout_layer == Wipeout::Always) {
            self.wipeout_time = Some(0.0);
        }
        if let Some(t) = self.wipeout_time.as_mut() {
            *t = (*t + dt).rem_euclid(90.0);
            if w2.abs() > EPS {
                let pose = blend_eye(clips.wipeout.sample(1.0), clips.wipeout.sample(*t), wipeout_weight);
                self.eye = blend_eye(self.eye, pose, w2);
            }
        }
        if crate::render::post::env_is("SSX_CAMERA_NOISE_LOG", "on") {
            let (sel, n) = self.carving.active();
            let deg = 2.0 * self.eye.0.w.abs().clamp(-1.0, 1.0).acos().to_degrees();
            info!(
                "camera noise: carving signal {:.3} items {:?} phase {:.3}; wipeout {:?} weight {:.3}; CameraEye \
                 {:.3} m, {deg:.2} deg",
                self.carving.cur,
                &sel[..n],
                self.carving.phase,
                self.wipeout_time.filter(|_| w2 > EPS),
                wipeout_weight,
                self.eye.1.length()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn carving_array_selects_and_loops() {
        assert_eq!(CarvingNoise::new(0.2).active(), ([(0, 1.0), (0, 0.0)], 1));
        let (sel, n) = CarvingNoise::new(0.5).active();
        assert_eq!((n, sel[0].0, sel[1].0), (2, 0, 1));
        assert!((sel[1].1 - 0.5).abs() < 1e-6);
        assert_eq!(CarvingNoise::new(1.0).active(), ([(3, 1.0), (0, 0.0)], 1));
        let mut c = CarvingNoise::new(0.2);
        for _ in 0..29 {
            c.update(0.2, 2.0);
        }
        assert!((c.phase - 58.0 / 60.0).abs() < 1e-5);
        c.update(0.2, 3.0);
        assert!((c.phase - 1.0 / 60.0).abs() < 1e-5);
        let mut b = CarvingNoise::new(1.0);
        b.update(1.0, 2.0);
        assert!((b.phase - 2.0 / 30.0).abs() < 1e-6);
    }
}
