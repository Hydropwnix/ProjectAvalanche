use super::*;

const DT: f32 = 2.0;

struct Rig {
    s: ScoreState,
    t: ScoringTuning,
    r: ResolvedCharacterProperties,
    c: ScoringContext,
}

impl Rig {
    fn new() -> Self {
        let t = ScoringTuning {
            max_boost: 150.0,
            points_per_boost: 100.0,
            boost_wipeout_multiplier: 0.5,
            boost_percent_after_awesome: 0.25,
            awesome_time: 840.0,
            super_awesome_time: 840.0,
            unlimited_boost_while_awesome: true,
            super_awesome_points_activation: 10_000.0,
        };
        let r = ResolvedCharacterProperties {
            trick_point_bonus: 1.0,
            boost_bonus: 1.0,
            tricky_bonus: 1.0,
            tricky_timer_modifier: 1.0,
            boost_meter_increase_modifier: 1.0,
            awesome_boost_meter_increase_modifier: 1.0,
            super_awesome_boost_meter_increase_modifier: 1.0,
            ..Default::default()
        };
        Self { s: ScoreState::default(), t, r, c: ScoringContext::default() }
    }

    fn step(&mut self, i: ScoringInputs) {
        self.s.update_with(&self.t, &self.r, &i, &self.c, DT);
    }

    fn steps(&mut self, n: usize, i: ScoringInputs) {
        (0..n).for_each(|_| self.step(i));
    }
}

fn ground() -> ScoringInputs {
    ScoringInputs::default()
}

fn air() -> ScoringInputs {
    ScoringInputs { in_air: true, ..ScoringInputs::default() }
}

fn grab(t: TrickType) -> ScoringInputs {
    ScoringInputs { trick_type: t, tricking: true, trick_locked: true, ..air() }
}

fn hold_points(t: TrickType, tt: f32) -> f32 {
    t.points() * trick_time_level(tt) / 60.0 * DT
}

#[test]
fn lua_tables_and_record_helpers() {
    assert_eq!(spin_points(179.0), 0.0);
    assert_eq!(spin_points(180.0), 150.0);
    assert_eq!(spin_points(360.0), 330.0);
    assert_eq!(spin_points(540.0), 540.0);
    assert_eq!(spin_points(720.0), 780.0);
    assert_eq!(spin_points(1080.0), 1350.0);
    assert_eq!(flip_points(360.0), 600.0);
    assert_eq!(flip_points(720.0), 1800.0);
    assert_eq!(trick_time_level(14.0), 0.0);
    assert_eq!(trick_time_level(60.0), 1.25);
    assert_eq!(trick_time_level(400.0), 2.5);
    assert_eq!((combo_bonus(1), combo_bonus(2), combo_bonus(6), combo_bonus(499)), (0.0, 2000.0, 10_000.0, 950_000.0));
    assert_eq!(combo_bonus(10_000), 1_000_000.0);
    assert_eq!((rewind_time_level(2.0), rewind_time_level(30.0), rewind_time_level(400.0)), (0.10, 0.35, 1.5));
    assert_eq!((flip_count(180f32.to_radians()), flip_count(181f32.to_radians())), (0, 1));
    assert_eq!((flip_count(539f32.to_radians()), flip_count(541f32.to_radians())), (1, 2));
    assert_eq!(spin_degrees_rounded(539f32.to_radians()), 360.0);
    let tiers = ComboFlowTuning::default().air_trick_tiers;
    assert_eq!([0.0, 59.0, 60.0, 120.0].map(|t| trick_time_tier(t, tiers)), [1, 1, 2, 3]);
    assert_eq!(trick_time_tier(-1.0, tiers), 0);
}

#[test]
fn a_grind_pays_its_distance_when_it_stops() {
    assert_eq!((grind_spin_points(179.0), grind_spin_points(180.0), grind_spin_points(360.0)), (0.0, 150.0, 375.0));
    let mut g = Rig::new();
    g.steps(2, ground());
    let grind = ScoringInputs { is_grinding: true, ..ground() };
    for _ in 0..60 {
        g.c.prev_position = g.c.position;
        g.c.position += Vec3::Z * 0.5;
        g.step(grind);
        assert_eq!(g.s.grind_capture, 0.0);
    }
    assert!((g.s.grind_distance - 30.0).abs() < 1e-3 && g.s.grind_time == 120.0);
    g.c.prev_position = g.c.position;
    g.step(ground());
    assert!(g.s.just_stopped_grinding);
    let want = 30.0 * 25.0 * 1.2;
    assert!((g.s.grind_capture - want).abs() < 0.1, "{}", g.s.grind_capture);
    assert_eq!((g.s.grind_bank, g.s.record.kind), (0.0, RecordKind::None));
    assert_eq!(g.s.last_record.kind, RecordKind::Grind);
}

#[test]
fn a_board_press_pays_its_distance_when_it_stops() {
    let mut g = Rig::new();
    g.c.player_speed = 54.0 / 3.6 / 60.0;
    g.steps(2, ground());
    let press = ScoringInputs { is_board_press: true, press_value: -1.0, ..ground() };
    for i in 0..30 {
        g.c.prev_position = g.c.position;
        g.c.position += Vec3::Z * 0.5;
        g.step(press);
        assert_eq!(g.s.board_press_count, 1);
        assert_eq!(g.s.is_scoring_board_press, g.s.press_distance > BOARDPRESS_MIN_DIST, "execution {i}");
    }
    assert!((g.s.press_distance - 15.0).abs() < 1e-3 && g.s.press_time == 60.0);
    assert!((g.s.combo_timer_max - 240.0 * 0.65).abs() < 1e-3, "{}", g.s.combo_timer_max);
    g.c.prev_position = g.c.position;
    g.step(ground());
    assert!(g.s.just_stopped_board_pressing);
    assert!((g.s.press_capture - 15.0 * 5.0).abs() < 0.01, "{}", g.s.press_capture);
    let rec = g.s.last_pushed.expect("pushed");
    assert!(rec.kind == RecordKind::BoardPress && rec.nose_press && (rec.distance - 15.0).abs() < 1e-3);
    assert_eq!(g.s.last_record.kind, RecordKind::None, "a press is never the duplicate reference");
    g.step(ScoringInputs { in_air: true, ..press });
    assert!(!g.s.is_board_press);
}

#[test]
fn the_take_off_step_rotation_counts() {
    let mut g = Rig::new();
    g.steps(3, ground());
    g.s.spin_fs = 5.0;
    let spin = ScoringInputs { spin_speed: 0.1, ..air() };
    g.step(spin);
    assert!(g.s.just_airborne);
    assert_eq!(g.s.spin_fs, 0.0);
    assert!((g.s.spin_bs - 0.2).abs() < 1e-6, "{}", g.s.spin_bs);
    g.step(spin);
    assert!((g.s.spin_bs - 0.4).abs() < 1e-6);
    assert_eq!((g.s.combo_timer, g.s.combo_timer_max), (106.0, 108.0));
}

#[test]
fn a_grab_change_captures_the_old_trick() {
    use TrickType::{Indy, Melon};
    let mut g = Rig::new();
    g.steps(2, ground());
    g.steps(6, grab(Melon));
    assert!(!g.s.just_changed_tricks && g.s.record.trick_id == TrickType::None);
    g.steps(4, grab(Indy));
    assert_eq!((g.s.trick_id, g.s.trick_time), (Indy, 20.0));
    assert_eq!((g.s.record.kind, g.s.record.trick_id), (RecordKind::Air, Indy));
    let bank = g.s.air_bank;
    assert!((bank - [16.0, 18.0, 20.0].map(|t| hold_points(Indy, t)).iter().sum::<f32>()).abs() < 1e-3);
    g.step(grab(Melon));
    assert!(g.s.just_changed_tricks);
    assert_eq!((g.s.trick_id, g.s.trick_time), (Indy, 22.0));
    let captured = bank + hold_points(Indy, 22.0);
    assert!((g.s.score - captured).abs() < 1e-3, "{} vs {captured}", g.s.score);
    assert_eq!((g.s.air_bank, g.s.combo_count), (0.0, 1));
    let pushed = g.s.last_pushed.expect("pushed");
    assert_eq!((pushed.kind, pushed.trick_id, pushed.trick_time), (RecordKind::Air, Indy, 22.0));
    assert_eq!(g.s.last_record, pushed);
    assert_eq!(g.s.record.kind, RecordKind::None);
    g.step(grab(Melon));
    assert!(!g.s.just_changed_tricks);
    assert_eq!((g.s.trick_id, g.s.trick_time), (Melon, DT));
}

#[test]
fn a_repeated_trick_pays_a_quarter_and_shortens_the_combo() {
    use TrickType::Indy;
    let mut g = Rig::new();
    g.steps(2, ground());
    g.steps(10, grab(Indy));
    let first = g.s.air_bank;
    g.step(ground());
    assert!(!g.s.duplicate && g.s.last_record.trick_id == Indy && g.s.combo_count == 1);
    g.steps(10, grab(Indy));
    assert!(g.s.duplicate && g.s.record.duplicate);
    let expect = hold_points(Indy, 16.0) + 0.25 * (hold_points(Indy, 18.0) + hold_points(Indy, 20.0));
    assert!((g.s.air_bank - expect).abs() < 1e-3 && g.s.air_bank < first, "{}", g.s.air_bank);
    let mut goofy = g.s;
    goofy.record.goofy = true;
    goofy.duplicate_trick_check();
    assert!(!goofy.duplicate);
    g.step(ground());
    assert_eq!((g.s.combo_timer_max, g.s.combo_count), (60.0, 2));
}

#[test]
fn three_seconds_uphill_block_air_points_until_the_next_landing() {
    use TrickType::Indy;
    let mut g = Rig::new();
    g.c.is_uphill = true;
    g.steps(89, ground());
    assert_eq!(g.s.time_uphill, 178.0);
    let mut ok = g.s;
    g.step(ground());
    assert_eq!(g.s.time_uphill, 180.0);
    ok.update_with(&g.t, &g.r, &grab(Indy), &g.c, DT);
    ok.update_with(&g.t, &g.r, &ScoringInputs { tweaking: true, ..grab(Indy) }, &g.c, DT);
    assert!(ok.air_bank > 0.0);
    g.steps(10, ScoringInputs { tweaking: true, ..grab(Indy) });
    assert_eq!((g.s.air_bank, g.s.time_uphill), (0.0, 180.0));
    assert!(!g.s.is_scoring_air);
    g.c.is_uphill = false;
    g.step(ground());
    assert_eq!(g.s.time_uphill, 0.0);
}

#[test]
fn rewind_multipliers() {
    let mut s = ScoreState::default();
    assert_eq!((s.duplicate_or_rewind(false), s.duplicate_or_rewind(true)), (1.0, 0.10));
    let ramp = [0.0, 2.0, 60.0, 130.0, 180.0, 240.0].map(|t| {
        s.time_since_rewind = t;
        s.duplicate_or_rewind(false)
    });
    assert_eq!(ramp, [1.0, 0.05, 0.10, 0.25, 0.5, 1.0]);
    s.duplicate = true;
    assert_eq!(s.duplicate_or_rewind(false), 0.25);
    s.time_rewinding_backwards = 2.0;
    assert_eq!(s.duplicate_or_rewind(false), 0.10);
    (s.is_awesome, s.time_since_rewind) = (true, 240.0);
    assert_eq!(s.tricky(false), 1.0);
    s.time_since_rewind = 242.0;
    assert_eq!((s.tricky(false), s.tricky(true)), (1.5, 1.0));
    s.is_super_awesome = true;
    assert_eq!(s.tricky(false), 2.0);
}

#[test]
fn combo_timer_factor_by_the_last_pushed_trick() {
    let k = ComboFlowTuning::default();
    let mut s = ScoreState::default();
    s.restart_combo_timer(&k, false);
    assert_eq!(s.combo_timer_max, 108.0);
    let max = [50.0, 89.0, 90.0, 179.0, 180.0, 400.0].map(|tt| {
        s.last_pushed = Some(TrickRecord { trick_time: tt, ..TrickRecord::default() });
        s.restart_combo_timer(&k, false);
        assert_eq!(s.combo_timer, s.combo_timer_max);
        s.combo_timer_max
    });
    assert_eq!(max, [108.0, 108.0, 180.0, 180.0, 240.0, 240.0]);
}

#[test]
fn the_combo_pays_again_when_it_ends_on_the_ground() {
    let mut g = Rig::new();
    g.steps(2, ground());
    g.step(air());
    g.s.air_bank = 1000.0;
    g.step(air());
    g.step(ground());
    assert_eq!((g.s.score, g.s.combo_points, g.s.combo_count), (1000.0, 1000.0, 1));
    assert_eq!(g.s.combo_timer, 108.0);
    let f = g.s.flushed[0].expect("a flush");
    assert!(f.is_points_captured && !f.is_bad_landing && f.air_bank == 1000.0 && g.s.flushed[1].is_none());
    assert!(g.s.combo_ended.is_none());
    g.steps(53, ground());
    assert_eq!((g.s.score, g.s.combo_timer), (1000.0, 2.0));
    g.step(ground());
    assert_eq!((g.s.score, g.s.combo_points, g.s.combo_count), (2000.0, 0.0, 0));
    let e = g.s.combo_ended.expect("the combo's end message");
    assert_eq!(
        (e.success, e.trick_points, e.bonus_points, e.total_points, e.combo_level),
        (true, 1000.0, 0.0, 1000.0, 1)
    );
    g.step(ground());
    assert!(g.s.combo_ended.is_none() && g.s.flushed[0].is_none(), "messages last one execution");
}

#[test]
fn the_combo_cannot_end_in_the_air() {
    let mut g = Rig::new();
    g.step(ground());
    g.s.combo_timer = 2.0;
    g.s.combo_points = 500.0;
    g.steps(5, air());
    assert_eq!((g.s.combo_timer, g.s.score), (2.0, 0.0));
    g.step(ground());
    assert_eq!((g.s.combo_timer, g.s.score, g.s.combo_points), (0.0, 500.0, 0.0));
}

fn flowing(c: &mut ScoringContext) {
    (c.player_speed, c.velocity_y, c.position, c.on_ground) = (0.4, -0.1, Vec3::new(0.0, -50.0, 0.0), true);
}

#[test]
fn positive_flow_and_the_flow_level() {
    let mut g = Rig::new();
    flowing(&mut g.c);
    g.step(ground());
    assert_eq!(g.s.descent, 50.0);
    assert_eq!(g.s.trailing_descent, 20.0);
    assert_eq!(g.s.flow_state, FlowState::Positive);
    g.step(air());
    g.s.air_bank = 1600.0;
    g.step(air());
    assert_eq!(g.s.flow_state, FlowState::Positive);
    g.step(ground());
    assert_eq!((g.s.flow_level, g.s.score, g.s.combo_points), (1, 3200.0, 1600.0));
    g.c.player_speed = 0.1;
    g.steps(29, ground());
    assert_eq!((g.s.flow_state, g.s.flow_level), (FlowState::Positive, 1));
    g.step(ground());
    assert_eq!((g.s.flow_state, g.s.flow_level), (FlowState::None, 0));
    assert_eq!(g.s.flow_state_timer, DT);
}

#[test]
fn negative_flow_pays_nothing() {
    let mut g = Rig::new();
    g.c.velocity_y = 0.1;
    g.steps(14, ground());
    assert_eq!(g.s.flow_state, FlowState::None);
    g.step(ground());
    assert_eq!((g.s.flow_state, g.s.flow_level), (FlowState::Negative, -1));
    g.step(air());
    g.s.air_bank = 1000.0;
    g.step(air());
    g.step(ground());
    assert_eq!((g.s.score, g.s.combo_points, g.s.combo_count), (0.0, 0.0, 1));
    assert!(g.s.boost > 0.0, "the boost still fills");
    g.c.velocity_y = -0.1;
    g.c.player_speed = 0.4;
    g.steps(7, ground());
    assert_eq!(g.s.flow_state, FlowState::Negative);
    g.step(ground());
    assert_eq!((g.s.flow_state, g.s.flow_level), (FlowState::None, 0));
}

#[test]
fn rewinding_backwards_costs_points_and_holds_the_combo() {
    let mut g = Rig::new();
    g.step(ground());
    (g.s.combo_timer, g.s.combo_points, g.s.score) = (50.0, 2000.0, 5000.0);
    (g.c.rewind_active, g.c.rewind_speed) = (true, -1.0);
    g.steps(20, ground());
    assert!(g.s.is_scoring_rewind && g.s.is_rewinding_backwards);
    assert_eq!((g.s.time_rewinding_backwards, g.s.time_since_rewind), (40.0, 0.0));
    let expect = -1000.0 / 60.0 * DT * (14.0 * 0.10 + 6.0 * 0.35);
    assert!((g.s.rewind_bank - expect).abs() < 1e-3, "{}", g.s.rewind_bank);
    assert_eq!(g.s.combo_timer, 48.0, "held from the second execution on (scoring the rewind)");
    g.c.rewind_speed = 1.0;
    g.step(ground());
    let bank = expect;
    assert_eq!(g.s.rewind_bank, 0.0);
    assert!((g.s.score - (5000.0 + bank)).abs() < 1e-3 && (g.s.combo_points - (2000.0 + bank)).abs() < 1e-3);
    let pushed = g.s.last_pushed.expect("rewind record");
    assert_eq!(pushed.kind, RecordKind::Rewind);
    assert_eq!(g.s.combo_timer_max, 108.0);
    g.c.rewind_active = false;
    g.steps(19, ground());
    assert_eq!((g.s.time_rewinding_backwards, g.s.time_since_rewind), (0.0, 0.0));
    g.step(ground());
    assert_eq!(g.s.time_since_rewind, DT);
}

#[test]
fn a_scoring_wipeout_ends_the_combo() {
    let mut g = Rig::new();
    g.step(ground());
    (g.s.combo_count, g.s.combo_points, g.s.combo_timer, g.s.boost) = (3, 900.0, 100.0, 40.0);
    g.step(air());
    g.s.air_bank = 700.0;
    g.step(ScoringInputs { wipeout: true, ..ground() });
    assert_eq!((g.s.air_bank, g.s.boost, g.s.score), (0.0, 20.0, 0.0));
    assert_eq!((g.s.combo_count, g.s.combo_points, g.s.combo_timer), (0, 0.0, 0.0));
    assert!(g.s.last_lost && g.s.last_capture == 700.0 && g.s.last_pushed.is_none());
    let f = g.s.flushed[0].expect("the wipeout's flush");
    assert!(!f.is_points_captured && f.is_bad_landing);
    let e = g.s.combo_ended.expect("the failed combo");
    assert_eq!(
        (e.success, e.trick_points, e.bonus_points, e.total_points, e.combo_level),
        (false, 900.0, 0.0, 900.0, 3)
    );
}

#[test]
fn a_forced_reset_clears_the_accumulators_once() {
    let mut g = Rig::new();
    g.steps(3, ground());
    let spin = ScoringInputs { spin_speed: 0.1, ..air() };
    g.steps(10, spin);
    assert!((g.s.spin_bs - 2.0).abs() < 1e-5, "{}", g.s.spin_bs);
    let (points, boost) = (g.s.score, g.s.boost);
    g.s.overrides.force_reset = true;
    g.step(spin);
    assert!((g.s.spin_bs - 0.2).abs() < 1e-6, "{}", g.s.spin_bs);
    assert!(!g.s.overrides.force_reset);
    assert_eq!((g.s.score, g.s.boost), (points, boost), "not the full reset");
    g.step(spin);
    assert!((g.s.spin_bs - 0.4).abs() < 1e-6, "one execution only");
}

#[test]
fn the_tricky_overrides_in_their_order() {
    let mut g = Rig::new();
    g.steps(3, ground());
    g.s.boost = 60.0;
    g.s.overrides.force_awesome_off = true;
    g.step(air());
    assert_eq!((g.s.boost, g.s.is_awesome, g.s.is_super_awesome), (0.0, false, false));
    g.s.overrides = ScoringOverrides { force_awesome_on: true, force_super_awesome_off: true, ..Default::default() };
    g.step(air());
    assert_eq!((g.s.awesome_timer, g.s.super_timer, g.s.super_activation), (840.0, 0.0, 0.0));
    assert!(g.s.is_awesome && !g.s.is_super_awesome);
    g.s.overrides.force_super_awesome_on = true;
    g.step(air());
    assert_eq!((g.s.awesome_timer, g.s.super_timer, g.s.super_activation), (840.0, 840.0, 1.0));
    assert!(g.s.is_awesome && g.s.is_super_awesome);
    g.s.overrides = ScoringOverrides { force_super_awesome_off: true, ..Default::default() };
    g.step(air());
    assert_eq!((g.s.awesome_timer, g.s.super_timer, g.s.super_activation, g.s.boost), (0.0, 0.0, 0.0, 0.0));
    assert!(!g.s.is_awesome && !g.s.is_super_awesome);
    g.c.trigger_reset = true;
    g.step(ground());
    assert!(g.s.overrides.force_super_awesome_off);
}

#[test]
fn a_finished_track_ends_tricky_and_empties_the_meter() {
    let mut g = Rig::new();
    g.steps(3, ground());
    (g.s.awesome_timer, g.s.is_awesome, g.s.boost) = (500.0, true, 150.0);
    g.step(ground());
    assert!(g.s.is_awesome && g.s.boost == 150.0);
    g.c.track_finished = true;
    g.step(ground());
    assert_eq!((g.s.awesome_timer, g.s.boost, g.s.is_awesome), (0.0, 0.0, false));
    assert!(g.s.track_ended.is_some_and(|m| m.tricky));
    g.step(ground());
    assert!(g.s.track_ended.is_none());
}

#[test]
fn boost_spends_the_meter_on_the_ground_only() {
    let r = ResolvedCharacterProperties {
        boost_accel_low: 0.0,
        awesome_boost_accel_low: 0.0,
        boost_accel_high: 30.0 / 3600.0,
        awesome_boost_accel_high: 33.0 / 3600.0,
        boost_drain_rate: 25.0 / 60.0,
        awesome_boost_drain_rate: 0.0,
        max_boost_speed: 145.0 / 216.0,
        awesome_max_boost_speed: 155.0 / 216.0,
        ..Default::default()
    };
    let go = BoostRequest { requested: true, player_speed: 20.0, ..BoostRequest::default() };
    let (mut b, mut meter) = (BoostState::default(), 10.5);
    b.update(&mut meter, &r, &go, DT);
    assert!(b.is_boosting && (meter - (10.5 - 2.0 * 25.0 / 60.0)).abs() < 1e-5);
    assert!((b.blend - (1.0 - (-0.2f32).exp())).abs() < 1e-6 && b.accel_high == r.boost_accel_high);
    assert_eq!((b.max_speed, b.drain_rate), (r.max_boost_speed, r.boost_drain_rate));
    let mut updates = 1;
    while b.is_boosting && updates < 1000 {
        b.update(&mut meter, &r, &go, DT);
        updates += 1;
    }
    assert_eq!((meter, updates), (0.0, 14));
    assert!(!b.is_boosting && b.blend > 0.5, "the blend decays after the meter ran out");
    for q in [
        BoostRequest { requested: false, ..go },
        BoostRequest { in_air: true, ..go },
        BoostRequest { crashed: true, ..go },
        BoostRequest { player_speed: 1.0, ..go },
    ] {
        let (mut b, mut meter) = (BoostState::default(), 50.0);
        b.update(&mut meter, &r, &q, DT);
        assert!(!b.is_boosting && meter == 50.0 && b.blend == 0.0, "{q:?}");
    }
    let (mut b, mut meter) = (BoostState::default(), 150.0);
    b.update(&mut meter, &r, &BoostRequest { awesome: true, ..go }, DT);
    assert!(b.is_boosting && meter == 150.0 && b.accel_high == r.awesome_boost_accel_high);
    assert_eq!(b.max_speed, r.awesome_max_boost_speed);
}
#[test]
fn a_pushed_flare_pays_1000_once_as_an_interaction() {
    let mut g = Rig::new();
    g.steps(3, ground());
    let score0 = g.s.score;
    g.s.pushed_object = true;
    g.step(ground());
    assert!(g.s.is_scoring_interactions);
    assert_eq!(g.s.interaction_bank, 1000.0);
    assert_eq!((g.s.record.kind, g.s.record.interaction), (RecordKind::Interaction, 2));
    g.step(ground());
    assert!(!g.s.pushed_object && !g.s.is_scoring_interactions);
    assert!(g.s.just_interacting);
    assert_eq!(g.s.interaction_capture, 0.0);
    g.step(ground());
    assert!(g.s.just_stopped_interacting);
    assert_eq!((g.s.interaction_capture, g.s.interaction_bank, g.s.combo_count), (1000.0, 0.0, 1));
    assert!(g.s.flushed.iter().flatten().any(|f| f.interactions_bank == 1000.0));
    assert!(g.s.score - score0 >= 1000.0);
    g.steps(3, ground());
    assert_eq!(g.s.interaction_bank, 0.0);
}

#[test]
fn a_scoring_wipeout_drops_the_push() {
    let mut g = Rig::new();
    g.s.pushed_object = true;
    g.step(ScoringInputs { wipeout: true, lua_wipeout: true, ..ground() });
    assert!(!g.s.pushed_object && !g.s.is_scoring_interactions);
    assert_eq!(g.s.interaction_bank, 0.0);
}

#[test]
fn uphill_ground_costs_points_after_180_ticks() {
    let mut g = Rig::new();
    g.c.is_uphill = true;
    g.steps(90, ground());
    assert!(!g.s.is_scoring_interactions);
    g.step(ground());
    assert!(g.s.is_scoring_interactions);
    assert!((g.s.interaction_bank + 5000.0 / 60.0 * DT).abs() < 1e-3);
    assert_eq!((g.s.record.kind, g.s.record.interaction), (RecordKind::Interaction, 3));
    assert_eq!((uphill_time_level(239.0), uphill_time_level(420.0), uphill_time_level(700.0)), (1.0, 2.0, 0.0));
}
