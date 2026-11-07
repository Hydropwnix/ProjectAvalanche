use anyhow::{Result, bail};

use crate::curve::Curve;
use crate::tuning::{AssetView, Bundle};

pub use ssx_rules::character::*;

mod controller_tuning;

pub use controller_tuning::*;

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn units_and_drag_from_cp_max_values() {
        let mut cp = CharacterProperties { name: "test".into(), ..Default::default() };
        for (k, v) in [
            ("BoostAccelLow", 0.0),
            ("AwesomeBoostAccelLow", 0.0),
            ("BoostAccelHigh", 30.0),
            ("AwesomeBoostAccelHigh", 33.0),
            ("BoostDrainRate", 18.0),
            ("AwesomeBoostDrainRate", 0.0),
            ("MaxBoostSpeed", 205.0),
            ("AwesomeMaxBoostSpeed", 229.0),
            ("UnstuckAssistAccelLow", 0.0),
            ("UnstuckAssistAccelHigh", 27.0),
            ("MaxUnstuckAssistSpeed", 97.0),
            ("HorizontalTerminalVelocity_Ground", 200.0),
            ("HorizontalTerminalVelocity_Air", 120.0),
            ("VerticalTerminalVelocity_Ground", 220.0),
            ("VerticalTerminalVelocity_Air", 250.0),
            ("TuckedVelocityMultiplier", 1.15),
            ("GravityAcceleration_Ground", -22.0),
            ("GravityAcceleration_Air", -24.0),
            ("GravityAcceleration_Wingsuit", -6.0),
            ("HorizontalTerminalVelocity_Wingsuit", 275.0),
            ("VerticalTerminalVelocity_Wingsuit", 250.0),
            ("HorizontalTerminalVelocityFalloffTime_Wingsuit", 320.0),
            ("VerticalTerminalVelocityFalloffTime_Wingsuit", 300.0),
            ("GravityAccelerationMultiplierTime_Wingsuit", 300.0),
            ("WingsuitDurabilityModifier", 3.0),
            ("IceAxeDigModifier", 1.0),
            ("PhysicalDamageModifier", 0.15),
            ("HealthRegenModifier", 3.0),
            ("IceAxeDurabilityModifier", 5.0),
            ("TemperatureLossModifier", 0.2),
            ("TemperatureGainModifier", 5.0),
            ("OxygenLossThresholdModifier", 1.0),
            ("OxygenGainThresholdModifier", 1.0),
            ("OxygenTankUsageModifier", 0.4),
            ("GainFromTankRateModifier", 1.15),
            ("HeadLampBatteryModifier", 2.9),
            ("PulseGogglesBatteryModifier", 3.0),
        ]
        .into_iter()
        .chain(crate::testutil::AIR_CP_MAX)
        {
            cp.floats.insert(k.into(), v);
        }
        let r = ResolvedCharacterProperties::resolve(&cp).unwrap();
        assert!((r.htv_ground - 0.925_926).abs() < 1e-5);
        assert!((r.gravity_ground + 0.006_111).abs() < 1e-6);
        assert!((r.k_h_ground - 0.007_13).abs() < 1e-5);
        assert!((r.k_v_air - 0.004_98).abs() < 1e-5);
        assert!((r.k_h_air - 0.0216).abs() < 1e-4);
        assert!((r.k_h_ground_tuck - r.k_h_ground / (1.15 * 1.15)).abs() < 1e-9);
        assert_eq!(r.g_air, Vec3::new(0.0, -24.0 / 3600.0, 0.0));
        assert!((r.awesome_boost_accel_high - 33.0 / 3600.0).abs() < 1e-9);
        assert!((r.awesome_max_boost_speed - 229.0 / 216.0).abs() < 1e-6);
        assert_eq!((r.boost_drain_rate, r.awesome_boost_drain_rate), (0.3, 0.0));
        let half = CharacterProperties::mix(&cp, &cp, 0.5).unwrap();
        assert_eq!(half.get("MaxBoostSpeed").unwrap(), 205.0);
    }

    #[test]
    fn gear_mix_keeps_gravity_and_unstuck_from_cp_max() {
        let cp = |pairs: &[(&str, f32)]| CharacterProperties {
            name: "t".into(),
            floats: pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        };
        let min = cp(&[("GravityAcceleration_Air", -0.2), ("MaxBoostSpeed", 145.0), ("UnstuckAssistAccelHigh", 0.0)]);
        let max = cp(&[("GravityAcceleration_Air", -24.0), ("MaxBoostSpeed", 205.0), ("UnstuckAssistAccelHigh", 27.0)]);
        let m = CharacterProperties::mix(&min, &max, 0.25).unwrap();
        assert_eq!(m.get("GravityAcceleration_Air").unwrap(), -24.0);
        assert_eq!(m.get("UnstuckAssistAccelHigh").unwrap(), 27.0);
        assert_eq!(m.get("MaxBoostSpeed").unwrap(), 160.0);
    }

    #[test]
    fn default_gear_maxes_board_and_wingsuit_and_leaves_the_bonuses_at_min() {
        let g = GearStats::default_gear();
        let (ts, tb, tt) = (g.n(0), g.n(1), g.n(2));
        assert_eq!((ts, tb, tt, g.n(3)), (1.0, 0.0, 1.0, 1.0));
        let at = |f: &str| mixed_field(f, &g, ts, tb, tt);
        assert_eq!(at("HorizontalTerminalVelocity_Ground"), FieldMix::Lerp(1.0));
        assert_eq!(at("SpinMaxVelocity"), FieldMix::Lerp(1.0));
        assert_eq!(at("MaxBoostSpeed"), FieldMix::Lerp(0.0));
        assert_eq!(at("TrickPointBonus"), FieldMix::Lerp(0.0));
        assert_eq!(at("HorizontalTerminalVelocity_Wingsuit"), FieldMix::Lerp(1.0));
        assert_eq!(at("GearDragArmourSpeed"), FieldMix::Max);
        assert_eq!(at("PhysicalDamageModifier"), FieldMix::Const(-1.0));
        assert_eq!(at("GravityAcceleration_Air"), FieldMix::Max);
        let cp = |pairs: &[(&str, f32)]| CharacterProperties {
            name: "t".into(),
            floats: pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        };
        let min =
            cp(&[("TrickPointBonus", 1.0), ("MaxBoostSpeed", 145.0), ("HorizontalTerminalVelocity_Ground", 180.0)]);
        let max =
            cp(&[("TrickPointBonus", 1.5), ("MaxBoostSpeed", 205.0), ("HorizontalTerminalVelocity_Ground", 200.0)]);
        let m = CharacterProperties::mix_gear(&min, &max, &g).unwrap();
        assert_eq!(m.get("TrickPointBonus").unwrap(), 1.0);
        assert_eq!(m.get("MaxBoostSpeed").unwrap(), 145.0);
        assert_eq!(m.get("HorizontalTerminalVelocity_Ground").unwrap(), 200.0);
        let u = CharacterProperties::mix_gear(&min, &max, &GearStats::uniform(0.5)).unwrap();
        assert_eq!(u.get("TrickPointBonus").unwrap(), 1.25);
    }
}
