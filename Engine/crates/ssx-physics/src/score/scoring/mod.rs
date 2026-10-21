use anyhow::Result;
use glam::Vec3;

use crate::character::ResolvedCharacterProperties;
use crate::tricks::TrickType;
use crate::tuning::{AssetView, Bundle};

mod banks;
mod lists;
mod tables;
mod update;

pub use tables::*;

#[derive(Clone, Copy, Debug, Default)]
pub struct ScoringInputs {
    pub in_air: bool,
    pub wipeout: bool,
    pub spin_speed: f32,
    pub flip_speed: f32,
    pub flip_accel: f32,
    pub goofy: bool,
    pub trick_type: TrickType,
    pub tricking: bool,
    pub trick_ending: bool,
    pub trick_locked: bool,
    pub tweaking: bool,
    pub is_grinding: bool,
    pub is_grind_trick: bool,
    pub super_grind: bool,
    pub grind_spin_vel: f32,
    pub grind_group: i32,
    pub is_board_press: bool,
    pub press_value: f32,
    pub lua_wipeout: bool,
    pub helicopter_interaction: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScoringOverrides {
    pub force_reset: bool,
    pub force_awesome_on: bool,
    pub force_awesome_off: bool,
    pub force_super_awesome_on: bool,
    pub force_super_awesome_off: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct ScoringContext {
    pub track_started: bool,
    pub trigger_reset: bool,
    pub track_finished: bool,
    pub is_uphill: bool,
    pub in_wingsuit: bool,
    pub is_boosting: bool,
    pub on_vertical_surface_for_scoring: bool,
    pub player_speed: f32,
    pub velocity_y: f32,
    pub position: Vec3,
    pub prev_position: Vec3,
    pub level_start_y: f32,
    pub on_ground: bool,
    pub force_descent_in_air: bool,
    pub death_by_death_plane: bool,
    pub rewind_active: bool,
    pub rewind_speed: f32,
    pub rewind_terminating: bool,
    pub tuning: ComboFlowTuning,
}

impl Default for ScoringContext {
    fn default() -> Self {
        Self {
            track_started: true,
            trigger_reset: false,
            track_finished: false,
            is_uphill: false,
            in_wingsuit: false,
            is_boosting: false,
            force_descent_in_air: false,
            on_vertical_surface_for_scoring: false,
            player_speed: 0.0,
            velocity_y: 0.0,
            position: Vec3::ZERO,
            prev_position: Vec3::ZERO,
            level_start_y: 0.0,
            on_ground: false,
            death_by_death_plane: false,
            rewind_active: false,
            rewind_speed: 0.0,
            rewind_terminating: false,
            tuning: ComboFlowTuning::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RecordKind {
    #[default]
    None = -1,
    Air = 0,
    Grind = 1,
    BoardPress = 2,
    Interaction = 3,
    Rewind = 4,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrickRecord {
    pub kind: RecordKind,
    pub trick_id: TrickType,
    pub flips_fwd: u32,
    pub flips_bwd: u32,
    pub interaction: i32,
    pub tier: u8,
    pub grind_group: i32,
    pub tweaked: bool,
    pub awesome: bool,
    pub super_awesome: bool,
    pub nose_press: bool,
    pub goofy: bool,
    pub duplicate: bool,
    pub spin_fs_deg: f32,
    pub spin_bs_deg: f32,
    pub distance: f32,
    pub trick_time: f32,
}

impl Default for TrickRecord {
    fn default() -> Self {
        Self {
            kind: RecordKind::None,
            trick_id: TrickType::None,
            flips_fwd: 0,
            flips_bwd: 0,
            interaction: -1,
            tier: 0,
            grind_group: -1,
            tweaked: false,
            awesome: false,
            super_awesome: false,
            nose_press: false,
            goofy: false,
            duplicate: false,
            spin_fs_deg: 0.0,
            spin_bs_deg: 0.0,
            distance: 0.0,
            trick_time: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BanksFlushed {
    pub is_points_captured: bool,
    pub is_bad_landing: bool,
    pub spin_total: f32,
    pub flip_total: f32,
    pub air_time: f32,
    pub flight_distance: f32,
    pub grind_time: f32,
    pub grind_distance: f32,
    pub air_bank: f32,
    pub flow_bank: f32,
    pub grind_bank: f32,
    pub board_press_bank: f32,
    pub interactions_bank: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ComboEnded {
    pub success: bool,
    pub is_wingsuit_deployed: bool,
    pub trick_points: f32,
    pub bonus_points: f32,
    pub total_points: f32,
    pub combo_level: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TrackEnded {
    pub tricky: bool,
    pub uber_tricky: bool,
    pub wingsuit: bool,
    pub boosting: bool,
}

pub fn flip_count(rad: f32) -> u32 {
    let deg = rad.to_degrees();
    if deg > 180.0 { ((deg - 180.0) / 360.0).floor() as u32 + 1 } else { 0 }
}

pub fn spin_degrees_rounded(rad: f32) -> f32 {
    (rad.to_degrees() / 180.0).floor() * 180.0
}

pub fn trick_time_tier(ticks: f32, tiers: [f32; 3]) -> u8 {
    tiers.iter().take_while(|&&t| ticks >= t).count() as u8
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FlowState {
    #[default]
    None = 0,
    Positive = 1,
    Negative = 2,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct OnEndAsset {
    met: bool,
    paid: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScoreState {
    pub score: f32,
    pub air_bank: f32,
    pub rewind_bank: f32,
    pub grind_bank: f32,
    pub grind_time: f32,
    pub grind_distance: f32,
    pub grind_spin_fs: f32,
    pub grind_spin_bs: f32,
    pub grind_spin_total: f32,
    pub scoring_grind_group: i32,
    pub buffered_super_grind: i32,
    pub grind_goofy: bool,
    pub prev_is_grinding: bool,
    pub just_grinding: bool,
    pub just_stopped_grinding: bool,
    pub is_scoring_grind: bool,
    pub grind_capture: f32,
    pub press_bank: f32,
    pub press_time: f32,
    pub press_distance: f32,
    pub board_press_count: i32,
    pub prev_is_board_pressing: bool,
    pub just_board_pressing: bool,
    pub just_stopped_board_pressing: bool,
    pub is_scoring_board_press: bool,
    pub is_board_press: bool,
    pub press_capture: f32,
    pub interaction_bank: f32,
    pub is_scoring_interactions: bool,
    pub prev_is_scoring_interactions: bool,
    pub just_interacting: bool,
    pub just_stopped_interacting: bool,
    pub interaction_capture: f32,
    pub pushed_object: bool,
    pub boost: f32,
    pub awesome_timer: f32,
    pub super_timer: f32,
    pub super_activation: f32,
    pub is_awesome: bool,
    pub is_super_awesome: bool,
    pub spin_fs: f32,
    pub spin_bs: f32,
    pub flip_fwd: f32,
    pub flip_bwd: f32,
    pub flip_dir: u8,
    pub air_time: f32,
    pub flight_distance: f32,
    pub trick_id: TrickType,
    pub trick_time: f32,
    pub tweak_time: f32,
    pub combo_count: u32,
    pub combo_points: f32,
    pub combo_timer: f32,
    pub combo_timer_max: f32,
    pub combo_pct: f32,
    pub flow_state: FlowState,
    pub flow_level: i32,
    pub flow_state_timer: f32,
    pub flow_uphill_timer: f32,
    pub flow_too_slow_timer: f32,
    pub flow_downhill_timer: f32,
    pub descent: f32,
    pub trailing_descent: f32,
    pub time_uphill: f32,
    pub time_since_rewind: f32,
    pub time_rewinding_backwards: f32,
    pub is_rewinding_backwards: bool,
    pub duplicate: bool,
    pub wipeout: bool,
    pub is_scoring_air: bool,
    pub is_scoring_rewind: bool,
    pub is_big_landing: bool,
    pub has_touched_ground: bool,
    pub just_landed: bool,
    pub just_airborne: bool,
    pub just_wipeout: bool,
    pub just_changed_tricks: bool,
    pub just_ended_trick: bool,
    pub just_locked_trick: bool,
    pub just_finished_track: bool,
    pub just_stopped_rewinding: bool,
    pub record: TrickRecord,
    pub last_record: TrickRecord,
    pub last_pushed: Option<TrickRecord>,
    pub last_capture: f32,
    pub last_lost: bool,
    pub trick_update: Option<TrickRecord>,
    pub flushed: [Option<BanksFlushed>; 5],
    pub combo_ended: Option<ComboEnded>,
    pub base_points: Option<f32>,
    pub track_ended: Option<TrackEnded>,
    pub prev_is_boosting: bool,
    prev_in_wingsuit: bool,
    pub just_stopped_wingsuit: bool,
    prev_in_air: bool,
    prev_is_landed: bool,
    prev_wipeout: bool,
    prev_trick_locked: bool,
    prev_trick_ending: bool,
    prev_track_finished: bool,
    prev_is_scoring_rewind: bool,
    prev_flow_state: FlowState,
    pub force_capture: bool,
    pub overrides: ScoringOverrides,
    on_end: [OnEndAsset; 4],
    grind_on_end: [OnEndAsset; 3],
    press_on_end: OnEndAsset,
    interaction_triggers: [u32; 4],
    pub award: f32,
}

impl Default for ScoreState {
    fn default() -> Self {
        Self {
            award: 0.0,
            score: 0.0,
            air_bank: 0.0,
            rewind_bank: 0.0,
            grind_bank: 0.0,
            grind_time: 0.0,
            grind_distance: 0.0,
            grind_spin_fs: 0.0,
            grind_spin_bs: 0.0,
            grind_spin_total: 0.0,
            scoring_grind_group: 0,
            buffered_super_grind: 0,
            grind_goofy: false,
            prev_is_grinding: false,
            just_grinding: false,
            just_stopped_grinding: false,
            is_scoring_grind: false,
            grind_capture: 0.0,
            press_bank: 0.0,
            press_time: 0.0,
            press_distance: 0.0,
            board_press_count: 0,
            prev_is_board_pressing: false,
            just_board_pressing: false,
            just_stopped_board_pressing: false,
            is_scoring_board_press: false,
            is_board_press: false,
            press_capture: 0.0,
            interaction_bank: 0.0,
            is_scoring_interactions: false,
            prev_is_scoring_interactions: false,
            just_interacting: false,
            just_stopped_interacting: false,
            interaction_capture: 0.0,
            pushed_object: false,
            boost: 0.0,
            awesome_timer: 0.0,
            super_timer: 0.0,
            super_activation: 0.0,
            is_awesome: false,
            is_super_awesome: false,
            spin_fs: 0.0,
            spin_bs: 0.0,
            flip_fwd: 0.0,
            flip_bwd: 0.0,
            flip_dir: 0,
            air_time: 0.0,
            flight_distance: 0.0,
            trick_id: TrickType::None,
            trick_time: 0.0,
            tweak_time: 0.0,
            combo_count: 0,
            combo_points: 0.0,
            combo_timer: 0.0,
            combo_timer_max: 0.0,
            combo_pct: 0.0,
            flow_state: FlowState::None,
            flow_level: 0,
            flow_state_timer: 0.0,
            flow_uphill_timer: 0.0,
            flow_too_slow_timer: 0.0,
            flow_downhill_timer: 0.0,
            descent: 0.0,
            trailing_descent: TRAILING_DESCENT_DEFAULT,
            time_uphill: 0.0,
            time_since_rewind: TIME_SINCE_REWIND_DEFAULT,
            time_rewinding_backwards: 0.0,
            is_rewinding_backwards: false,
            duplicate: false,
            wipeout: false,
            is_scoring_air: false,
            is_scoring_rewind: false,
            is_big_landing: false,
            has_touched_ground: false,
            just_landed: false,
            just_airborne: false,
            just_wipeout: false,
            just_changed_tricks: false,
            just_ended_trick: false,
            just_locked_trick: false,
            just_finished_track: false,
            just_stopped_rewinding: false,
            record: TrickRecord::default(),
            last_record: TrickRecord::default(),
            last_pushed: None,
            last_capture: 0.0,
            last_lost: false,
            trick_update: None,
            flushed: [None; 5],
            combo_ended: None,
            base_points: None,
            track_ended: None,
            prev_is_boosting: false,
            prev_in_wingsuit: false,
            just_stopped_wingsuit: false,
            prev_in_air: false,
            prev_is_landed: false,
            prev_wipeout: false,
            prev_trick_locked: false,
            prev_trick_ending: false,
            prev_track_finished: false,
            prev_is_scoring_rewind: false,
            prev_flow_state: FlowState::None,
            force_capture: false,
            overrides: ScoringOverrides::default(),
            on_end: [OnEndAsset::default(); 4],
            grind_on_end: [OnEndAsset::default(); 3],
            press_on_end: OnEndAsset::default(),
            interaction_triggers: [0; 4],
        }
    }
}

impl ScoreState {
    pub fn update(&mut self, t: &ScoringTuning, r: &ResolvedCharacterProperties, i: &ScoringInputs, dt: f32) {
        self.update_with(t, r, i, &ScoringContext::default(), dt);
    }

    pub fn update_with(
        &mut self,
        t: &ScoringTuning,
        r: &ResolvedCharacterProperties,
        i: &ScoringInputs,
        c: &ScoringContext,
        dt: f32,
    ) {
        self.update_descent(c);
        (self.trick_update, self.flushed, self.combo_ended) = (None, [None; 5], None);
        (self.base_points, self.track_ended) = (None, None);
        let reset = c.trigger_reset || !c.track_started;
        let rewound_back = c.rewind_active && c.rewind_speed < 0.0;
        self.reset_from_defaults(rewound_back, reset);
        if !reset {
            self.edges_and_accumulators(i, c, dt);
        }
        self.update_flow_state(i, c, dt);
        if !(rewound_back || reset) {
            self.flow_state_timer += dt;
        } else if self.flow_state != FlowState::Negative {
            self.flow_state_timer = 0.0;
        }
        self.award = 0.0;
        if !reset {
            self.evaluate_and_award(t, r, i, c, dt);
        }
    }

    fn update_descent(&mut self, c: &ScoringContext) {
        let d = c.level_start_y - c.position.y;
        if c.track_started && d > self.descent && (c.on_ground || c.force_descent_in_air) && !c.death_by_death_plane {
            self.descent = d;
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BoostState {
    pub is_boosting: bool,
    pub blend: f32,
    pub accel_low: f32,
    pub accel_high: f32,
    pub max_speed: f32,
    pub drain_rate: f32,
    pub modifier: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BoostRequest {
    pub requested: bool,
    pub in_air: bool,
    pub player_speed: f32,
    pub crashed: bool,
    pub awesome: bool,
}

pub const BOOST_MIN_PLAYER_SPEED: f32 = 1.0;
pub const BOOST_BLEND_RATE: f32 = 0.1;

impl BoostState {
    pub fn update(&mut self, meter: &mut f32, r: &ResolvedCharacterProperties, q: &BoostRequest, dt: f32) {
        self.drain_rate = if q.awesome { r.awesome_boost_drain_rate } else { r.boost_drain_rate };
        let can = *meter > 0.0 && !q.in_air && q.player_speed > BOOST_MIN_PLAYER_SPEED && !q.crashed;
        self.is_boosting = can && q.requested;
        if self.is_boosting {
            *meter -= dt * self.drain_rate;
        }
        *meter = meter.max(0.0);
        let target = if self.is_boosting { 1.0 } else { 0.0 };
        self.blend = (target + (self.blend - target) * (-BOOST_BLEND_RATE * dt).exp()).clamp(0.0, 1.0);
        (self.accel_low, self.accel_high, self.max_speed) = if q.awesome {
            (r.awesome_boost_accel_low, r.awesome_boost_accel_high, r.awesome_max_boost_speed)
        } else {
            (r.boost_accel_low, r.boost_accel_high, r.max_boost_speed)
        };
    }
}

#[cfg(test)]
mod tests;
