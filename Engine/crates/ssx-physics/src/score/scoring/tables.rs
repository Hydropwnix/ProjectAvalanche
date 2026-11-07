use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScoringTuning {
    pub max_boost: f32,
    pub points_per_boost: f32,
    pub boost_wipeout_multiplier: f32,
    pub boost_percent_after_awesome: f32,
    pub awesome_time: f32,
    pub super_awesome_time: f32,
    pub unlimited_boost_while_awesome: bool,
    pub super_awesome_points_activation: f32,
}

impl ScoringTuning {
    pub fn load(bundle: &Bundle) -> Result<Self> {
        let a = bundle.view("ScoringSceneOp", "ScoringSceneOpAsset")?;
        Ok(Self {
            max_boost: a.f32("MaxBoost")?,
            points_per_boost: a.f32("PointsPerBoost")?,
            boost_wipeout_multiplier: a.f32("BoostWipeoutMultipler")?,
            boost_percent_after_awesome: a.f32("BoostPercentAfterAwesome")?,
            awesome_time: a.f32("AwesomeTime")?,
            super_awesome_time: a.f32("SuperAwesomeTime")?,
            unlimited_boost_while_awesome: a.int("UnlimitedBoostWhileAwesome")? != 0,
            super_awesome_points_activation: a.f32("SuperAwesomePointsActivation")?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ComboFlowTuning {
    pub combo_link_time: f32,
    pub combo_time_medium_percentage: f32,
    pub combo_time_small_percentage: f32,
    pub combo_time_board_press_percentage: f32,
    pub combo_time_dupe_percentage: f32,
    pub combo_max_count: u32,
    pub air_trick_tiers: [f32; 3],
    pub grind_trick_tiers: [f32; 3],
    pub flow_min_speed: f32,
    pub time_uphill_for_no_flow: f32,
    pub time_uphill_for_neg_flow: f32,
    pub time_too_slow_for_no_flow: f32,
    pub time_downhill_for_flow: f32,
    pub flow_stagnation_time: f32,
    pub descent_trail_distance: f32,
    pub descent_rate: f32,
}

impl Default for ComboFlowTuning {
    fn default() -> Self {
        Self {
            combo_link_time: 240.0,
            combo_time_medium_percentage: 0.75,
            combo_time_small_percentage: 0.45,
            combo_time_board_press_percentage: 0.65,
            combo_time_dupe_percentage: 0.25,
            combo_max_count: 500,
            air_trick_tiers: [0.0, 60.0, 120.0],
            grind_trick_tiers: [0.0, 120.0, 240.0],
            flow_min_speed: 0.324_074_1,
            time_uphill_for_no_flow: 120.0,
            time_uphill_for_neg_flow: 30.0,
            time_too_slow_for_no_flow: 60.0,
            time_downhill_for_flow: 15.0,
            flow_stagnation_time: 1.0,
            descent_trail_distance: 30.0,
            descent_rate: 0.1,
        }
    }
}

pub(crate) fn number(a: &AssetView, field: &str) -> Result<f32> {
    a.f32(field).or_else(|_| a.int(field).map(|v| v as f32))
}

impl ComboFlowTuning {
    pub fn load(bundle: &Bundle) -> Result<Self> {
        let a = bundle.view("ScoringSceneOp", "ScoringSceneOpAsset")?;
        let n = |f: &str| number(&a, f);
        Ok(Self {
            combo_link_time: n("TrickComboLinkTime")?,
            combo_time_medium_percentage: n("TrickComboTimeMediumPercentage")?,
            combo_time_small_percentage: n("TrickComboTimeSmallPercentage")?,
            combo_time_board_press_percentage: n("TrickComboTimeBoardPressPercentage")?,
            combo_time_dupe_percentage: n("TrickComboTimeDupePercentage")?,
            combo_max_count: n("TrickComboMaxCount")? as u32,
            air_trick_tiers: [n("AirTrickTier1")?, n("AirTrickTier2")?, n("AirTrickTier3")?],
            grind_trick_tiers: [n("GrindTrickTier1")?, n("GrindTrickTier2")?, n("GrindTrickTier3")?],
            flow_min_speed: n("FlowMinSpeed")?,
            time_uphill_for_no_flow: n("TimeUphillForNoFlow")?,
            time_uphill_for_neg_flow: n("TimeUphillForNegFlow")?,
            time_too_slow_for_no_flow: n("TimeTooSlowForNoFlow")?,
            time_downhill_for_flow: n("TimeDownhillForFlow")?,
            flow_stagnation_time: n("FlowStagnationTime")?,
            descent_trail_distance: n("DescentTrailDistance")?,
            descent_rate: n("DescentRate")?,
        })
    }
}

pub use ssx_rules::scoring::*;
