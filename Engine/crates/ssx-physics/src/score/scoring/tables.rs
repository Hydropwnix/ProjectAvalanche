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

pub const UPHILL_TIME_NO_AIR_TRICKS: f32 = 180.0;
pub const TIME_SINCE_REWIND_DEFAULT: f32 = 1.0e6;
pub const TRAILING_DESCENT_DEFAULT: f32 = -10_000.0;
pub(super) const SPEED_EPS: f32 = 1.525_9e-5;
pub(super) const COMBO_ROTATION_SPEED: f32 = 0.025;
pub const BOARDPRESS_MIN_DIST: f32 = 10.0;
pub const BOARDPRESS_MIN_SPEED: f32 = 2.0;
pub const BOARDPRESS_DIST_POINTS: f32 = 5.0;
pub(super) const FLOW_LEVEL_UP_POINTS: [f32; 3] = [1500.0, 2250.0, 3000.0];

pub fn trick_time_level(ticks: f32) -> f32 {
    [(300.0, 2.5), (240.0, 2.0), (180.0, 1.75), (120.0, 1.5), (60.0, 1.25), (30.0, 1.0), (15.0, 0.5)]
        .into_iter()
        .find(|&(t, _)| ticks >= t)
        .map_or(0.0, |(_, l)| l)
}

pub fn grind_trick_time_level(ticks: f32) -> f32 {
    [
        (1200.0, 0.0),
        (540.0, 2.0),
        (480.0, 1.8),
        (420.0, 1.4),
        (360.0, 1.0),
        (240.0, 0.8),
        (120.0, 0.5),
        (60.0, 0.3),
        (30.0, 0.2),
        (15.0, 0.1),
    ]
    .into_iter()
    .find(|&(t, _)| ticks >= t)
    .map_or(0.1, |(_, l)| l)
}

pub fn grind_spin_points(deg: f32) -> f32 {
    if deg < 180.0 {
        return 0.0;
    }
    let n = (deg / 180.0).floor();
    n * (150.0 + 37.5 * (n - 1.0))
}

pub fn spin_points(deg: f32) -> f32 {
    if deg < 180.0 {
        return 0.0;
    }
    let n = (deg / 180.0).floor();
    n * 150.0 + 30.0 * n * (n - 1.0) / 2.0
}

pub fn flip_points(deg: f32) -> f32 {
    if deg < 180.0 {
        return 0.0;
    }
    let n = (1.0 + (deg - 180.0) / 360.0).floor();
    n * (600.0 + (n - 1.0) * 300.0)
}

pub fn combo_bonus(count: u32) -> f32 {
    const TABLE: [(u32, f32); 26] = [
        (2, 2_000.0),
        (5, 10_000.0),
        (7, 15_000.0),
        (10, 25_000.0),
        (20, 50_000.0),
        (25, 75_000.0),
        (30, 100_000.0),
        (40, 125_000.0),
        (50, 150_000.0),
        (60, 175_000.0),
        (70, 200_000.0),
        (80, 225_000.0),
        (90, 250_000.0),
        (100, 300_000.0),
        (125, 350_000.0),
        (150, 400_000.0),
        (175, 450_000.0),
        (200, 500_000.0),
        (225, 550_000.0),
        (250, 600_000.0),
        (275, 650_000.0),
        (300, 700_000.0),
        (350, 800_000.0),
        (400, 900_000.0),
        (450, 950_000.0),
        (500, 1_000_000.0),
    ];
    TABLE.iter().rev().find(|&&(c, _)| count >= c).map_or(0.0, |&(_, b)| b)
}

pub fn rewind_time_level(ticks: f32) -> f32 {
    [(300.0, 1.5), (240.0, 1.25), (180.0, 1.0), (120.0, 0.75), (60.0, 0.5), (30.0, 0.35)]
        .into_iter()
        .find(|&(t, _)| ticks >= t)
        .map_or(0.10, |(_, l)| l)
}

pub(super) const UPHILL_TIME_LEVEL: [(f32, f32); 6] =
    [(180.0, 1.0), (240.0, 1.25), (300.0, 1.5), (360.0, 1.75), (420.0, 2.0), (640.0, 0.0)];

pub fn uphill_time_level(ticks: f32) -> f32 {
    UPHILL_TIME_LEVEL.iter().rev().find(|&&(t, _)| ticks >= t).map_or(UPHILL_TIME_LEVEL[0].1, |&(_, l)| l)
}

pub fn rewind_ramp(ticks: f32) -> f32 {
    [(240.0, 1.0), (180.0, 0.5), (120.0, 0.25), (60.0, 0.10)]
        .into_iter()
        .find(|&(t, _)| ticks >= t)
        .map_or(0.05, |(_, m)| m)
}
