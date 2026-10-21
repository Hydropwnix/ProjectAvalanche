use std::f32::consts::FRAC_PI_2;

use anyhow::{Context, Result, bail};
use glam::{Quat, Vec3};
use ssx_formats::ant_anim::{Animation, ChannelToDof, dof_list};

use crate::tuning::Bundle;

#[derive(Clone, Debug)]
pub struct RootClip {
    anim: Animation,
    q: usize,
    t: usize,
    num_ticks: f32,
    fps_scale: f32,
}

impl RootClip {
    pub fn load(bundle: &Bundle, name: &str) -> Result<Self> {
        Self::load_with(bundle, name, true)
    }

    pub fn load_pose(bundle: &Bundle, name: &str) -> Result<Self> {
        Self::load_with(bundle, name, false)
    }

    fn load_with(bundle: &Bundle, name: &str, delta: bool) -> Result<Self> {
        let clip = bundle.view(name, "ClipControllerAsset")?;
        if (clip.int("DeltaTrajectory")? == 1) != delta {
            bail!("{name}: DeltaTrajectory is not {}", u8::from(delta));
        }
        let obj = clip.asset("Anim")?.object;
        let anim =
            Animation::from_object(bundle.schema(), obj)?.with_context(|| format!("{name}: Anim is no animation"))?;
        let table =
            bundle.object_by_guid(anim.info().channel_to_dof).with_context(|| format!("{name}: no ChannelToDof"))?;
        let table = ChannelToDof::parse(bundle.schema(), table)?;
        let dofs = dof_list(&bundle.stream, "genchar_rig", "AnimRig")?;
        let dof = |n: &str| dofs.iter().position(|d| d.layout == "Trajectory" && d.name == n);
        let channel = |d: Option<usize>| d.and_then(|d| table.indices.iter().position(|&x| usize::from(x) == d));
        let c = anim.counts();
        let (Some(q), Some(t)) = (channel(dof("AITrajectory.q")), channel(dof("AITrajectory.t"))) else {
            bail!("{name}: no AITrajectory channels");
        };
        if q >= c.quats || t < c.quats || t - c.quats >= c.vec3s {
            bail!("{name}: AITrajectory channels {q} / {t} outside {c:?}");
        }
        Ok(Self { anim, q, t: t - c.quats, num_ticks: clip.f32("NumTicks")?, fps_scale: clip.f32("FPSScale")? })
    }

    pub fn from_nis(clip: &ssx_assets::nis::NisClip) -> Result<Self> {
        use ssx_assets::nis::Channel;
        if !clip.delta_trajectory {
            bail!("{}: DeltaTrajectory is not 1", clip.name);
        }
        let (Some(&Channel::Quat(q)), Some(&Channel::Vec3(t))) =
            (clip.channels.get("AITrajectory.q"), clip.channels.get("AITrajectory.t"))
        else {
            bail!("{}: no AITrajectory channels", clip.name);
        };
        Ok(Self { anim: clip.anim.clone(), q, t, num_ticks: clip.num_ticks, fps_scale: clip.fps_scale })
    }

    pub fn num_ticks(&self) -> f32 {
        self.num_ticks
    }

    pub fn sample(&self, tick: f32) -> Result<(Quat, Vec3)> {
        let p = self.anim.sample(tick.clamp(0.0, self.num_ticks) * self.fps_scale)?;
        Ok((Quat::from_array(p.quats[self.q]).normalize(), Vec3::from_array(p.vec3s[self.t])))
    }

    pub fn delta(&self, t0: f32, t1: f32) -> Result<(Vec3, Quat)> {
        let ((q0, p0), (q1, p1)) = (self.sample(t0)?, self.sample(t1)?);
        let c = q0.conjugate();
        let (d, r) = (c * (p1 - p0), (c * q1).normalize());
        let y = Quat::from_rotation_y(-FRAC_PI_2);
        Ok((y * d, (y * r * y.conjugate()).normalize()))
    }
}

#[derive(Clone, Debug, Default)]
pub struct RootMotion {
    pub janked: Option<RootClip>,
    pub get_going: Option<[RootClip; 4]>,
    pub ready_loop: Option<RootClip>,
    pub go: Option<Vec<RootClip>>,
    pub random_seed: u32,
}

impl RootMotion {
    pub fn push_go(&mut self, clip: RootClip) -> usize {
        let go = self.go.get_or_insert_with(Vec::new);
        go.push(clip);
        go.len() - 1
    }
}

pub const GET_GOING_ITEMS: [(f32, &str); 4] = [
    (-1.57, "ride_get_going_right"),
    (0.002_089_262, "ride_get_going_fwd"),
    (1.57, "ride_get_going_left"),
    (std::f32::consts::PI, "ride_get_going_bwd"),
];

pub const GET_GOING_END_TICKS: f32 = 36.0;

impl RootMotion {
    pub fn load(bundle: &Bundle) -> Result<Self> {
        let get_going = [
            RootClip::load(bundle, GET_GOING_ITEMS[0].1)?,
            RootClip::load(bundle, GET_GOING_ITEMS[1].1)?,
            RootClip::load(bundle, GET_GOING_ITEMS[2].1)?,
            RootClip::load(bundle, GET_GOING_ITEMS[3].1)?,
        ];
        let ls = crate::levelstart::GO_CLIPS;
        let go = vec![RootClip::load(bundle, ls[0])?, RootClip::load(bundle, ls[1])?, RootClip::load(bundle, ls[2])?];
        Ok(Self {
            janked: Some(RootClip::load(bundle, "bail_freeze_01")?),
            get_going: Some(get_going),
            ready_loop: Some(RootClip::load_pose(bundle, crate::levelstart::READY_LOOP_CLIP)?),
            go: Some(go),
            random_seed: bundle
                .view(crate::levelstart::GO_CHOOSER, "ChooserControllerAsset")?
                .asset("PolicyAsset")?
                .asset("RandomValueAsset")?
                .int("Seed")? as u32,
        })
    }
}

pub const GET_GOING_ANGLE: f32 = FRAC_PI_2;

pub fn get_going_blend(angle: f32) -> Option<Vec<(usize, f32)>> {
    use std::f32::consts::PI;
    const X: [(f32, Option<usize>); 6] =
        [(-PI, None), (-1.57, Some(0)), (-0.001, None), (0.002_089_262, Some(1)), (1.57, Some(2)), (PI, Some(3))];
    let s = angle.clamp(X[0].0, X[5].0);
    let i = (0..5).find(|&i| (s - s.clamp(X[i].0, X[i + 1].0)).abs() < 0.001)?;
    let width = X[i + 1].0 - X[i].0;
    let t = if (s - X[i].0).abs() < 0.001 || width <= 1.525_879e-5 { 0.0 } else { (s - X[i].0) / width };
    let picks: Vec<(Option<usize>, f32)> = if t < 0.03 {
        vec![(X[i].1, 1.0)]
    } else if t > 0.97 {
        vec![(X[i + 1].1, 1.0)]
    } else {
        vec![(X[i].1, 1.0 - t), (X[i + 1].1, t)]
    };
    picks.into_iter().map(|(k, w)| k.map(|k| (k, w))).collect()
}

pub fn get_going_delta(rm: &RootMotion, items: &[(usize, f32)], t0: f32, t1: f32) -> Option<(Vec3, Quat)> {
    let clips = rm.get_going.as_ref()?;
    let mut acc: Option<(Vec3, Quat, f32)> = None;
    for &(k, w) in items {
        let (d, r) = clips[k].delta(t0, t1).ok()?;
        acc = Some(match acc {
            None => (d, r, w),
            Some((d0, r0, w0)) => {
                let t = w / (w0 + w);
                (d0.lerp(d, t), nlerp(r0, r, t), w0 + w)
            }
        });
    }
    acc.map(|(d, r, _)| (d, r))
}

fn rig_frame() -> Quat {
    Quat::from_rotation_y(-FRAC_PI_2)
}

pub fn step_physics(traj: Quat, dp: Vec3, root: Quat, controllers: &[&str]) -> ssx_assets::anim_pose::StepPhysics {
    let j = traj * rig_frame();
    let (t, q) = (j.conjugate() * dp, (j.conjugate() * root * rig_frame()).normalize());
    ssx_assets::anim_pose::StepPhysics {
        q: q.to_array(),
        t: t.to_array(),
        controllers: controllers.iter().map(|c| c.to_string()).collect(),
    }
}

pub fn world_motion(traj: Quat, q: [f32; 4], t: [f32; 3]) -> (Vec3, Quat) {
    let j = traj * rig_frame();
    (j * Vec3::from_array(t), (j * Quat::from_array(q) * rig_frame().conjugate()).normalize())
}

pub fn nlerp(a: Quat, b: Quat, t: f32) -> Quat {
    let b = if a.dot(b) < 0.0 { -b } else { b };
    (a + (b - a) * t).normalize()
}

pub fn blend_out_weight(elapsed: f32, dt: f32) -> f32 {
    const BLEND_TICKS: f32 = 8.0;
    if BLEND_TICKS - (elapsed - dt) <= dt {
        return 0.0;
    }
    ssx_assets::anim_eval::curve_weight(elapsed / BLEND_TICKS, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_going_items_and_blend_weights() {
        assert_eq!(get_going_blend(GET_GOING_ANGLE), Some(vec![(2, 1.0)]));
        let mid = get_going_blend(0.8).expect("no mirrored item");
        assert_eq!((mid[0].0, mid[1].0), (1, 2));
        assert!((mid[0].1 + mid[1].1 - 1.0).abs() < 1e-6);
        assert_eq!(get_going_blend(0.0), None, "fwd_mirrored takes part");
        let w: Vec<f32> = [2.0, 4.0, 6.0, 8.0].iter().map(|&e| blend_out_weight(e, 2.0)).collect();
        assert!((w[0] - 0.923_88).abs() < 2e-3 && (w[1] - std::f32::consts::FRAC_1_SQRT_2).abs() < 2e-3, "{w:?}");
        assert!((w[2] - 0.382_68).abs() < 2e-3 && w[3] == 0.0, "{w:?}");
    }

    #[test]
    fn delta_set_round_trip() {
        let traj = Quat::from_euler(glam::EulerRot::YXZ, 0.7, -0.2, 0.1);
        let root = traj * Quat::from_rotation_y(0.05);
        let dp = Vec3::new(0.3, -0.1, 0.8);
        let p = step_physics(traj, dp, root, &["CarvingControllerAsset"]);
        let (d, r) = world_motion(traj, p.q, p.t);
        assert!((d - dp).length() < 1e-5 && r.angle_between(root) < 1e-4, "{d} {r}");
    }
}
