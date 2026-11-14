use crate::postfx_state::PostFxValues;
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
    pub tint: [[f32; 4]; 2],
    pub intensity: [f32; 2],
    pub fadeoffbounds: [f32; 3],
}

impl OverlayState {
    pub fn of(v: &PostFxValues) -> OverlayState {
        let fob = v.get(fnv1(FIELD_OVERLAY_FADEOFFBOUNDS));
        OverlayState {
            tint: [v.get(fnv1(FIELD_OVERLAY_1_TINT)), v.get(fnv1(FIELD_OVERLAY_2_TINT))],
            intensity: [v.float(fnv1(FIELD_OVERLAY_1_INTENSITY)), v.float(fnv1(FIELD_OVERLAY_2_INTENSITY))],
            fadeoffbounds: [fob[0], fob[1], fob[2]],
        }
    }

    pub fn shader_params(&self) -> Option<OverlayParams> {
        let intensity = self.intensity;
        if !(intensity[0] > 0.0 || intensity[1] > 0.0) {
            return None;
        }
        let tint = |i: usize| {
            let t = self.tint[i];
            let one_minus = 1.0 - intensity[i];
            let floor = if INTENSITY_EPSILON - one_minus >= 0.0 { INTENSITY_EPSILON } else { one_minus };
            [t[0], t[1], t[2], 1.0 / floor]
        };
        Some(OverlayParams { tint_intensity: [tint(0), tint(1)], fadeoffbounds: self.fadeoffbounds })
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
    pub fn of(drop: OverlayDrop) -> SurvivalOverlay {
        SurvivalOverlay { drop, maps: drop.maps() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(i1: f32, i2: f32, fadeoffbounds: [f32; 3]) -> OverlayState {
        OverlayState { tint: [[0.2, 0.3, 0.4, 0.0], [1.0, 0.5, 0.0, 0.0]], intensity: [i1, i2], fadeoffbounds }
    }

    #[test]
    fn field_names_match_the_vault_keys() {
        assert_eq!(fnv1(FIELD_OVERLAY_1_INTENSITY), 0xA73C_2ADD);
        assert_eq!(fnv1(FIELD_OVERLAY_1_TINT), 0x4C2B_5DDD);
        assert_eq!(fnv1(FIELD_OVERLAY_2_INTENSITY), 0xBC9D_74C2);
        assert_eq!(fnv1(FIELD_OVERLAY_2_TINT), 0x7643_277C);
        assert_eq!(fnv1(FIELD_OVERLAY_FADEOFFBOUNDS), 0xB62D_5EC0);
        assert_eq!(SurvivalOverlay::of(OverlayDrop::Cold).maps[1], "postfx_ice_damage");
    }

    #[test]
    fn shader_intensity_is_the_reciprocal_threshold() {
        assert!(state(0.0, 0.0, [0.0; 3]).shader_params().is_none());
        let p = state(0.5, 1.0, [1.0, 0.0, 0.0]).shader_params().unwrap();
        assert_eq!(p.tint_intensity[0][3], 2.0);
        assert_eq!(p.tint_intensity[1][3], 1.0 / INTENSITY_EPSILON);
        assert_eq!(p.tint_intensity[0][..3], [0.2, 0.3, 0.4]);
    }
}
