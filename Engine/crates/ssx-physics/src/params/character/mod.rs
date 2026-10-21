use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use glam::Vec3;

use crate::curve::Curve;
use crate::tuning::{AssetView, Bundle};
use crate::{KMH_PER_M_PER_TICK, MS2_PER_M_PER_TICK2, TICKS_PER_SECOND};

mod controller_tuning;

pub use controller_tuning::*;

pub const DEFAULT_GEAR_MIX: f32 = 1.0;

pub const GEAR_STAT_NAMES: [&str; 37] = [
    "BoardSpeed",
    "BoardBoost",
    "BoardTricks",
    "BoardStability",
    "BoardCarving",
    "WingsuitLift",
    "WingsuitSpeed",
    "WingsuitDurability",
    "IceAxeDig",
    "IceAxeAdhesion",
    "IceAxeDragReduction",
    "ArmourDamageReduction",
    "ArmourHealthRegen",
    "ArmourDragReduction",
    "ThermalLossModifier",
    "ThermalGainModifier",
    "ThermalDragReduction",
    "OxygenLossModifier",
    "OxygenGainModifier",
    "OxygenDragReduction",
    "HeadLampConeAngle",
    "HeadLampLightDistance",
    "HeadLampIntensity",
    "PulseGogglesRange",
    "PulseGogglesProximity",
    "PulseGogglesIntensity",
    "AntiGravity",
    "UberTricks",
    "TrickPointBonus",
    "NightVision",
    "Glowing",
    "BoostBonus",
    "MaxRewindCountBonus",
    "TrickyBonus",
    "BigHead",
    "BigHands",
    "BigHeadAndHands",
];

pub const SLOT_BOARD: usize = 0;
pub const SLOT_WINGSUIT: usize = 1;
pub const SLOT_ICE_AXE: usize = 2;
pub const SLOT_ARMOUR: usize = 3;
pub const SLOT_THERMAL: usize = 4;
pub const SLOT_OXYGEN: usize = 5;

#[derive(Clone, Debug, PartialEq)]
pub struct GearStats {
    pub stats: [f32; 37],
    pub slots: [bool; 15],
    pub null: bool,
}

impl GearStats {
    pub fn default_gear() -> Self {
        let mut stats = [0.0; 37];
        for i in [0, 2, 3, 5, 6, 7] {
            stats[i] = 100.0;
        }
        let mut slots = [false; 15];
        slots[SLOT_BOARD] = true;
        slots[SLOT_WINGSUIT] = true;
        Self { stats, slots, null: false }
    }

    pub fn none() -> Self {
        let mut stats = [0.0; 37];
        for (i, s) in stats.iter_mut().enumerate() {
            if i <= 25 || i == 27 {
                *s = 100.0;
            }
        }
        Self { stats, slots: [false; 15], null: true }
    }

    pub fn uniform(t: f32) -> Self {
        Self { stats: [100.0 * t; 37], ..Self::default_gear() }
    }

    pub fn n(&self, i: usize) -> f32 {
        (self.stats[i] / 100.0).min(1.0)
    }

    fn equipped(&self, slot: usize) -> bool {
        !self.null && self.slots[slot]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FieldMix {
    Max,
    Lerp(f32),
    Const(f32),
}

pub fn mixed_field(field: &str, gear: &GearStats, ts: f32, tb: f32, tt: f32) -> FieldMix {
    use FieldMix::*;
    let n = |i| gear.n(i);
    let item = |slot, i| if gear.equipped(slot) { Lerp(n(i)) } else { Max };
    match field {
        "BoostAccelLow"
        | "AwesomeBoostAccelLow"
        | "BoostAccelHigh"
        | "AwesomeBoostAccelHigh"
        | "BoostDrainRate"
        | "AwesomeBoostDrainRate"
        | "MaxBoostSpeed"
        | "AwesomeMaxBoostSpeed" => Lerp(tb),
        "BoostMeterIncreaseModifier"
        | "AwesomeBoostMeterIncreaseModifier"
        | "SuperAwesomeBoostMeterIncreaseModifier"
        | "TrickyTimerModifier"
        | "FlipActuatorStrength"
        | "FlipMaxVelocity"
        | "FlipMaxPrewindVelocity"
        | "FlipMaxLaunchVelocity"
        | "SpinActuatorStrength"
        | "SpinMaxVelocity"
        | "SpinMaxPrewindVelocity"
        | "SpinMaxLaunchVelocity" => Lerp(tt),
        "FlipMinAssistFactor" | "FlipMaxAssistFactor" | "SpinMinAssistFactor" | "SpinMaxAssistFactor" => Lerp(n(3)),
        "HorizontalTerminalVelocity_Ground"
        | "HorizontalTerminalVelocity_Air"
        | "VerticalTerminalVelocity_Ground"
        | "VerticalTerminalVelocity_Air"
        | "TuckedVelocityMultiplier" => Lerp(ts),
        "HorizontalTerminalVelocity_Wingsuit" | "VerticalTerminalVelocity_Wingsuit" => Lerp(n(6)),
        "HorizontalTerminalVelocityFalloffTime_Wingsuit" | "VerticalTerminalVelocityFalloffTime_Wingsuit" => Lerp(n(5)),
        "PhysicalDamageModifier" => {
            if gear.equipped(SLOT_ARMOUR) {
                Lerp(n(11))
            } else {
                Const(-1.0)
            }
        }
        "HealthRegenModifier" => {
            if gear.equipped(SLOT_ARMOUR) {
                Lerp(n(12))
            } else {
                Const(1.0)
            }
        }
        "WingsuitDurabilityModifier" => Lerp(n(7)),
        "IceAxeDigModifier" => Lerp(n(8)),
        "IceAxeDurabilityModifier" => Lerp(n(9)),
        "TemperatureLossModifier" => {
            if gear.equipped(SLOT_THERMAL) {
                Lerp(n(14))
            } else {
                Const(1.0)
            }
        }
        "TemperatureGainModifier" => {
            if gear.equipped(SLOT_THERMAL) {
                Lerp(n(15))
            } else {
                Const(0.0)
            }
        }
        "OxygenTankUsageModifier" => {
            if gear.equipped(SLOT_OXYGEN) {
                Lerp(n(17))
            } else {
                Const(-1.0)
            }
        }
        "GainFromTankRateModifier" => {
            if gear.equipped(SLOT_OXYGEN) {
                Lerp(n(18))
            } else {
                Const(-1.0)
            }
        }
        "HeadLampBatteryModifier" => Lerp(n(22)),
        "PulseGogglesBatteryModifier" => Lerp(n(25)),
        "TrickPointBonus" => Lerp(n(28)),
        "BoostBonus" => Lerp(n(31)),
        "TrickyBonus" => Lerp(n(33)),
        f if f.starts_with("GearDragIceAxe") => item(SLOT_ICE_AXE, 10),
        f if f.starts_with("GearDragArmour") => item(SLOT_ARMOUR, 13),
        f if f.starts_with("GearDragThermal") => item(SLOT_THERMAL, 16),
        f if f.starts_with("GearDragOxygen") => item(SLOT_OXYGEN, 19),
        _ => Max,
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CharacterProperties {
    pub name: String,
    pub floats: BTreeMap<String, f32>,
}

impl CharacterProperties {
    pub fn from_asset(a: &AssetView<'_>) -> Result<Self> {
        if a.class() != "CharacterPropertiesAsset" {
            bail!("{:?} is a {}, expected a CharacterPropertiesAsset", a.name(), a.class());
        }
        let floats = a.floats().into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        Ok(Self { name: a.name().to_string(), floats })
    }

    pub fn get(&self, field: &str) -> Result<f32> {
        self.floats.get(field).copied().with_context(|| format!("{} has no float field {field}", self.name))
    }

    pub fn mix_gear(min: &Self, max: &Self, gear: &GearStats) -> Result<Self> {
        let lerp = |k: &str, t: f32| -> Result<f32> {
            let (lo, hi) = (min.get(k)?, max.get(k)?);
            Ok(lo + (hi - lo) * t)
        };
        let (mut ts, mut tb, mut tt) = (gear.n(0), gear.n(1), gear.n(2));
        for (slot, red, item) in [
            (SLOT_ICE_AXE, 10, "IceAxe"),
            (SLOT_ARMOUR, 13, "Armour"),
            (SLOT_THERMAL, 16, "Thermal"),
            (SLOT_OXYGEN, 19, "Oxygen"),
        ] {
            if gear.equipped(slot) {
                let d = |what: &str| lerp(&format!("GearDrag{item}{what}"), gear.n(red));
                ts = (ts - d("Speed")?).clamp(0.0, 1.0);
                tb = (tb - d("Boost")?).clamp(0.0, 1.0);
                tt = (tt - d("Tricks")?).clamp(0.0, 1.0);
            }
        }
        let mut floats = BTreeMap::new();
        for (k, &hi) in &max.floats {
            let v = match mixed_field(k, gear, ts, tb, tt) {
                FieldMix::Max => hi,
                FieldMix::Lerp(t) => lerp(k, t)?,
                FieldMix::Const(c) => c,
            };
            floats.insert(k.clone(), v);
        }
        let htv = "HorizontalTerminalVelocity_Ground";
        if let (Some(&out_htv), Ok(min_htv)) = (floats.get(htv), min.get(htv)) {
            for k in ["OxygenLossThresholdModifier", "OxygenGainThresholdModifier"] {
                if let Some(&hi) = max.floats.get(k) {
                    let base = if gear.equipped(SLOT_OXYGEN) { hi } else { 1.0 };
                    floats.insert(k.to_string(), base * out_htv / min_htv);
                }
            }
        }
        if !gear.null
            && let Some(v) = floats.get_mut("MaxRewindCount")
        {
            *v += gear.stats[32];
        }
        let anti = gear.n(26);
        if anti > 0.0
            && let Some(v) = floats.get_mut("VerticalTerminalVelocity_Air")
        {
            *v *= 1.0 - 0.8 * anti;
        }
        Ok(Self { name: format!("mix({}, {})", min.name, max.name), floats })
    }

    pub fn mix(min: &Self, max: &Self, t: f32) -> Result<Self> {
        Self::mix_gear(min, max, &GearStats::uniform(t))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ResolvedCharacterProperties {
    pub boost_accel_low: f32,
    pub awesome_boost_accel_low: f32,
    pub boost_accel_high: f32,
    pub awesome_boost_accel_high: f32,
    pub boost_drain_rate: f32,
    pub awesome_boost_drain_rate: f32,
    pub max_boost_speed: f32,
    pub awesome_max_boost_speed: f32,
    pub unstuck_accel_low: f32,
    pub unstuck_accel_high: f32,
    pub max_unstuck_speed: f32,
    pub htv_ground: f32,
    pub htv_air: f32,
    pub vtv_ground: f32,
    pub vtv_air: f32,
    pub htv_wingsuit: f32,
    pub vtv_wingsuit: f32,
    pub htv_falloff_wingsuit: f32,
    pub vtv_falloff_wingsuit: f32,
    pub gravity_multiplier_time_wingsuit: f32,
    pub wingsuit_durability_modifier: f32,
    pub tucked_multiplier: f32,
    pub gravity_ground: f32,
    pub gravity_air: f32,
    pub gravity_wingsuit: f32,
    pub ice_axe_dig_modifier: f32,
    pub k_h_ground: f32,
    pub k_h_ground_tuck: f32,
    pub k_v_ground: f32,
    pub k_v_ground_tuck: f32,
    pub k_h_air: f32,
    pub k_v_air: f32,
    pub g_ground: Vec3,
    pub g_air: Vec3,
    pub g_wingsuit: Vec3,
    pub flip_actuator_strength: f32,
    pub spin_actuator_strength: f32,
    pub flip_max_velocity: f32,
    pub flip_max_prewind_velocity: f32,
    pub flip_max_launch_velocity: f32,
    pub spin_max_velocity: f32,
    pub spin_max_prewind_velocity: f32,
    pub spin_max_launch_velocity: f32,
    pub flip_min_assist_factor: f32,
    pub flip_max_assist_factor: f32,
    pub spin_min_assist_factor: f32,
    pub spin_max_assist_factor: f32,
    pub prewind_accum_rate: f32,
    pub awesome_prewind_accum_rate: f32,
    pub prewind_decay_rate_natural: f32,
    pub awesome_prewind_decay_rate_natural: f32,
    pub prewind_decay_rate_reverse: f32,
    pub boost_meter_increase_modifier: f32,
    pub awesome_boost_meter_increase_modifier: f32,
    pub super_awesome_boost_meter_increase_modifier: f32,
    pub tricky_timer_modifier: f32,
    pub trick_point_bonus: f32,
    pub boost_bonus: f32,
    pub tricky_bonus: f32,
    pub physical_damage_modifier: f32,
    pub health_regen_modifier: f32,
    pub ice_axe_durability_modifier: f32,
    pub temperature_loss_modifier: f32,
    pub temperature_gain_modifier: f32,
    pub oxygen_loss_threshold_modifier: f32,
    pub oxygen_gain_threshold_modifier: f32,
    pub oxygen_tank_usage_modifier: f32,
    pub gain_from_tank_rate_modifier: f32,
    pub head_lamp_battery_modifier: f32,
    pub pulse_goggles_battery_modifier: f32,
}

impl ResolvedCharacterProperties {
    pub fn resolve(cp: &CharacterProperties) -> Result<Self> {
        let speed = |f: &str| cp.get(f).map(|v| v / KMH_PER_M_PER_TICK);
        let accel = |f: &str| cp.get(f).map(|v| v / MS2_PER_M_PER_TICK2);
        let mut r = Self {
            boost_accel_low: accel("BoostAccelLow")?,
            awesome_boost_accel_low: accel("AwesomeBoostAccelLow")?,
            boost_accel_high: accel("BoostAccelHigh")?,
            awesome_boost_accel_high: accel("AwesomeBoostAccelHigh")?,
            boost_drain_rate: cp.get("BoostDrainRate")? / TICKS_PER_SECOND,
            awesome_boost_drain_rate: cp.get("AwesomeBoostDrainRate")? / TICKS_PER_SECOND,
            max_boost_speed: speed("MaxBoostSpeed")?,
            awesome_max_boost_speed: speed("AwesomeMaxBoostSpeed")?,
            unstuck_accel_low: accel("UnstuckAssistAccelLow")?,
            unstuck_accel_high: accel("UnstuckAssistAccelHigh")?,
            max_unstuck_speed: speed("MaxUnstuckAssistSpeed")?,
            htv_ground: speed("HorizontalTerminalVelocity_Ground")?,
            htv_air: speed("HorizontalTerminalVelocity_Air")?,
            vtv_ground: speed("VerticalTerminalVelocity_Ground")?,
            vtv_air: speed("VerticalTerminalVelocity_Air")?,
            htv_wingsuit: speed("HorizontalTerminalVelocity_Wingsuit")?,
            vtv_wingsuit: speed("VerticalTerminalVelocity_Wingsuit")?,
            htv_falloff_wingsuit: cp.get("HorizontalTerminalVelocityFalloffTime_Wingsuit")?,
            vtv_falloff_wingsuit: cp.get("VerticalTerminalVelocityFalloffTime_Wingsuit")?,
            gravity_multiplier_time_wingsuit: cp.get("GravityAccelerationMultiplierTime_Wingsuit")?,
            wingsuit_durability_modifier: cp.get("WingsuitDurabilityModifier")?,
            tucked_multiplier: cp.get("TuckedVelocityMultiplier")?,
            gravity_ground: accel("GravityAcceleration_Ground")?,
            gravity_air: accel("GravityAcceleration_Air")?,
            gravity_wingsuit: accel("GravityAcceleration_Wingsuit")?,
            ice_axe_dig_modifier: cp.get("IceAxeDigModifier")?,
            flip_actuator_strength: cp.get("FlipActuatorStrength")?,
            spin_actuator_strength: cp.get("SpinActuatorStrength")?,
            flip_max_velocity: cp.get("FlipMaxVelocity")?.to_radians(),
            flip_max_prewind_velocity: cp.get("FlipMaxPrewindVelocity")?.to_radians(),
            flip_max_launch_velocity: cp.get("FlipMaxLaunchVelocity")?.to_radians(),
            spin_max_velocity: cp.get("SpinMaxVelocity")?.to_radians(),
            spin_max_prewind_velocity: cp.get("SpinMaxPrewindVelocity")?.to_radians(),
            spin_max_launch_velocity: cp.get("SpinMaxLaunchVelocity")?.to_radians(),
            flip_min_assist_factor: cp.get("FlipMinAssistFactor")?,
            flip_max_assist_factor: cp.get("FlipMaxAssistFactor")?,
            spin_min_assist_factor: cp.get("SpinMinAssistFactor")?,
            spin_max_assist_factor: cp.get("SpinMaxAssistFactor")?,
            prewind_accum_rate: cp.get("PrewindAccumRate")?,
            awesome_prewind_accum_rate: cp.get("AwesomePrewindAccumRate")?,
            prewind_decay_rate_natural: cp.get("PrewindDecayRateNatural")?,
            awesome_prewind_decay_rate_natural: cp.get("AwesomePrewindDecayRateNatural")?,
            prewind_decay_rate_reverse: cp.get("PrewindDecayRateReverse")?,
            boost_meter_increase_modifier: cp.get("BoostMeterIncreaseModifier")?,
            awesome_boost_meter_increase_modifier: cp.get("AwesomeBoostMeterIncreaseModifier")?,
            super_awesome_boost_meter_increase_modifier: cp.get("SuperAwesomeBoostMeterIncreaseModifier")?,
            tricky_timer_modifier: cp.get("TrickyTimerModifier")?,
            trick_point_bonus: cp.get("TrickPointBonus")?,
            boost_bonus: cp.get("BoostBonus")?,
            tricky_bonus: cp.get("TrickyBonus")?,
            physical_damage_modifier: cp.get("PhysicalDamageModifier")?,
            health_regen_modifier: cp.get("HealthRegenModifier")?,
            ice_axe_durability_modifier: cp.get("IceAxeDurabilityModifier")?,
            temperature_loss_modifier: cp.get("TemperatureLossModifier")?,
            temperature_gain_modifier: cp.get("TemperatureGainModifier")?,
            oxygen_loss_threshold_modifier: cp.get("OxygenLossThresholdModifier")?,
            oxygen_gain_threshold_modifier: cp.get("OxygenGainThresholdModifier")?,
            oxygen_tank_usage_modifier: cp.get("OxygenTankUsageModifier")?,
            gain_from_tank_rate_modifier: cp.get("GainFromTankRateModifier")?,
            head_lamp_battery_modifier: cp.get("HeadLampBatteryModifier")?,
            pulse_goggles_battery_modifier: cp.get("PulseGogglesBatteryModifier")?,
            ..Self::default()
        };
        let k = |g: f32, vt: f32| g.abs() / (vt * vt);
        let (gg, ga, tuck) = (r.gravity_ground, r.gravity_air, r.tucked_multiplier);
        r.k_h_ground = k(gg, r.htv_ground);
        r.k_h_ground_tuck = k(gg, r.htv_ground * tuck);
        r.k_v_ground = k(gg, r.vtv_ground);
        r.k_v_ground_tuck = k(gg, r.vtv_ground * tuck);
        r.k_h_air = k(ga, r.htv_air);
        r.k_v_air = k(ga, r.vtv_air);
        r.g_ground = Vec3::new(0.0, gg, 0.0);
        r.g_air = Vec3::new(0.0, ga, 0.0);
        r.g_wingsuit = Vec3::new(0.0, r.gravity_wingsuit, 0.0);
        Ok(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
