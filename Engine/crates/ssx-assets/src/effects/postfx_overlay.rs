use std::path::Path;

use anyhow::{Context, Result};

use crate::postfx::{PostFxPreset, load_presets};
use crate::vault_db::fnv1;

pub const FIELD_OVERLAY_1_INTENSITY: &str = "overlay_1_intensity";
pub const FIELD_OVERLAY_1_TINT: &str = "overlay_1_tintcolour";
pub const FIELD_OVERLAY_2_INTENSITY: &str = "overlay_2_intensity";
pub const FIELD_OVERLAY_2_TINT: &str = "overlay_2_tintcolour";
pub const FIELD_OVERLAY_FADEOFFBOUNDS: &str = "overlay_fadeoffbounds";

pub const UV_SCALE_16_9: [f32; 4] = [1.6, 0.9, 0.8, 0.45];
const INTENSITY_EPSILON: f32 = 1.192_092_9e-7;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OverlayState {
    pub tint: [Option<[f32; 4]>; 2],
    pub intensity: [Option<f32>; 2],
    pub fadeoffbounds: Option<[f32; 3]>,
}

fn lerp_field<const N: usize>(acc: &mut Option<[f32; N]>, src: Option<[f32; N]>, w: f32) {
    if let Some(s) = src {
        *acc = Some(match *acc {
            None => s,
            Some(a) => std::array::from_fn(|i| (s[i] - a[i]) * w + a[i]),
        });
    }
}

impl OverlayState {
    pub fn of(p: &PostFxPreset) -> OverlayState {
        let key = |n: &str| format!("{:#010x}", fnv1(n));
        let float = |n: &str| p.floats.get(n).or_else(|| p.floats.get(&key(n))).copied();
        let vector = |n: &str| p.vectors.get(n).or_else(|| p.vectors.get(&key(n))).cloned();
        let v4 =
            |n: &str| vector(n).filter(|v| v.len() >= 3).map(|v| [v[0], v[1], v[2], v.get(3).copied().unwrap_or(0.0)]);
        OverlayState {
            tint: [v4(FIELD_OVERLAY_1_TINT), v4(FIELD_OVERLAY_2_TINT)],
            intensity: [float(FIELD_OVERLAY_1_INTENSITY), float(FIELD_OVERLAY_2_INTENSITY)],
            fadeoffbounds: vector(FIELD_OVERLAY_FADEOFFBOUNDS).filter(|v| v.len() >= 3).map(|v| [v[0], v[1], v[2]]),
        }
    }

    pub fn blend(&mut self, src: &OverlayState, w: f32) {
        for i in 0..2 {
            lerp_field(&mut self.tint[i], src.tint[i], w);
            let mut acc = self.intensity[i].map(|x| [x]);
            lerp_field(&mut acc, src.intensity[i].map(|x| [x]), w);
            self.intensity[i] = acc.map(|[x]| x);
        }
        lerp_field(&mut self.fadeoffbounds, src.fadeoffbounds, w);
    }

    pub fn shader_params(&self) -> Option<OverlayParams> {
        let intensity = self.intensity.map(|i| i.unwrap_or(0.0));
        if !(intensity[0] > 0.0 || intensity[1] > 0.0) {
            return None;
        }
        let tint = |i: usize| {
            let t = self.tint[i].unwrap_or([0.0; 4]);
            let one_minus = 1.0 - intensity[i];
            let floor = if INTENSITY_EPSILON - one_minus >= 0.0 { INTENSITY_EPSILON } else { one_minus };
            [t[0], t[1], t[2], 1.0 / floor]
        };
        Some(OverlayParams {
            tint_intensity: [tint(0), tint(1)],
            fadeoffbounds: self.fadeoffbounds.unwrap_or([0.0; 3]),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OverlayParams {
    pub tint_intensity: [[f32; 4]; 2],
    pub fadeoffbounds: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayDrop {
    Default,
    Cold,
    LowOxygen,
    Avalanche,
}

#[derive(Clone, Debug)]
pub struct SurvivalOverlay {
    pub drop: OverlayDrop,
    pub maps: [&'static str; 2],
    pub base: OverlayState,
    pub presets: std::collections::BTreeMap<String, (Option<u32>, OverlayState)>,
}

pub fn layer_priority(layer: &str) -> Option<u32> {
    match layer {
        "default" => Some(0),
        "overlay_layer2" => Some(6),
        "overlay_layer1" => Some(7),
        _ => None,
    }
}

impl OverlayDrop {
    pub fn maps(self) -> [&'static str; 2] {
        match self {
            OverlayDrop::Default => ["postfx_vignette_base", "postfx_physical_damage"],
            OverlayDrop::Cold => ["postfx_physical_damage", "postfx_ice_damage"],
            OverlayDrop::LowOxygen => ["postfx_physical_damage", "postfx_thin_air_damage"],
            OverlayDrop::Avalanche => ["postfx_physical_damage", "postfx_bury_damage"],
        }
    }
}

pub fn texture_path(name: &str) -> String {
    format!("postfx\\textures\\{name}.xpr")
}

impl SurvivalOverlay {
    pub fn load(big_root: &Path, drop: OverlayDrop, names: &[String]) -> Result<(SurvivalOverlay, Vec<String>)> {
        let base = load_presets(big_root, &["default"]).context("loading the default post-FX preset")?;
        let base = OverlayState::of(&base.first().context("no default preset")?.0);
        let mut o = SurvivalOverlay { drop, maps: drop.maps(), base, presets: Default::default() };
        let mut failed = Vec::new();
        for name in names {
            match load_presets(big_root, &[name.as_str()]) {
                Ok(mut v) if !v.is_empty() => {
                    let (p, _) = v.remove(0);
                    let priority = p.layer.as_deref().and_then(layer_priority);
                    o.presets.insert(name.clone(), (priority, OverlayState::of(&p)));
                }
                _ => failed.push(name.clone()),
            }
        }
        Ok((o, failed))
    }

    pub fn state_of(&self, running: &[(String, f32)]) -> OverlayState {
        let mut by_layer: std::collections::BTreeMap<u32, (&OverlayState, f32)> = Default::default();
        for (name, w) in running {
            if let Some((Some(priority), st)) = self.presets.get(name) {
                by_layer.insert(*priority, (st, *w));
            }
        }
        let mut acc = OverlayState::default();
        acc.blend(&self.base, 1.0);
        for (st, w) in by_layer.into_values() {
            if w > 0.0 {
                acc.blend(st, w);
            }
        }
        acc
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(i1: f32, i2: f32, fob: Option<[f32; 3]>) -> OverlayState {
        OverlayState {
            tint: [Some([0.2, 0.3, 0.4, 0.0]), Some([1.0, 0.5, 0.0, 0.0])],
            intensity: [Some(i1), Some(i2)],
            fadeoffbounds: fob,
        }
    }

    #[test]
    fn field_names_match_the_vault_keys() {
        assert_eq!(fnv1(FIELD_OVERLAY_1_INTENSITY), 0xA73C_2ADD);
        assert_eq!(fnv1(FIELD_OVERLAY_1_TINT), 0x4C2B_5DDD);
        assert_eq!(fnv1(FIELD_OVERLAY_2_INTENSITY), 0xBC9D_74C2);
        assert_eq!(fnv1(FIELD_OVERLAY_2_TINT), 0x7643_277C);
        assert_eq!(fnv1(FIELD_OVERLAY_FADEOFFBOUNDS), 0xB62D_5EC0);
    }

    #[test]
    fn blend_copies_absent_fields_and_lerps_present_ones() {
        let mut acc = OverlayState::default();
        acc.blend(&state(0.8, 0.0, None), 0.25);
        assert_eq!(acc.intensity[0], Some(0.8));
        let base = state(0.0, 0.0, Some([1.0, 0.9, 0.0]));
        let mut acc = OverlayState::default();
        acc.blend(&base, 1.0);
        acc.blend(
            &OverlayState {
                intensity: [None, Some(1.0)],
                fadeoffbounds: Some([0.9, 0.0, 0.0]),
                ..OverlayState::default()
            },
            0.5,
        );
        assert_eq!(acc.intensity, [Some(0.0), Some(0.5)]);
        let f = acc.fadeoffbounds.unwrap();
        assert!((f[0] - 0.95).abs() < 1e-6 && (f[1] - 0.45).abs() < 1e-6 && f[2] == 0.0);
    }

    #[test]
    fn shader_intensity_is_the_reciprocal_threshold() {
        assert!(state(0.0, 0.0, None).shader_params().is_none());
        let p = state(0.5, 1.0, Some([1.0, 0.0, 0.0])).shader_params().unwrap();
        assert_eq!(p.tint_intensity[0][3], 2.0);
        assert_eq!(p.tint_intensity[1][3], 1.0 / INTENSITY_EPSILON);
        assert_eq!(p.tint_intensity[0][..3], [0.2, 0.3, 0.4]);
    }

    #[test]
    fn running_presets_blend_by_layer() {
        let base = state(0.0, 0.0, Some([1.0, 0.9, 0.0]));
        let l2 = OverlayState { intensity: [None, Some(1.0)], ..OverlayState::default() };
        let l1 =
            OverlayState { intensity: [Some(1.0), None], fadeoffbounds: Some([0.0; 3]), ..OverlayState::default() };
        let mut presets = std::collections::BTreeMap::new();
        presets.insert("a2".to_string(), (Some(6), l2));
        presets.insert("b1".to_string(), (Some(7), l1));
        presets.insert("c1".to_string(), (Some(7), OverlayState::default()));
        let o = SurvivalOverlay { drop: OverlayDrop::Default, maps: OverlayDrop::Default.maps(), base, presets };
        let s = o.state_of(&[("b1".into(), 0.5), ("a2".into(), 0.25)]);
        assert_eq!(s.intensity, [Some(0.5), Some(0.25)]);
        let s = o.state_of(&[("b1".into(), 0.5), ("c1".into(), 1.0)]);
        assert_eq!(s.intensity, [Some(0.0), Some(0.0)]);
        assert_eq!(OverlayDrop::Cold.maps()[1], "postfx_ice_damage");
    }
}
