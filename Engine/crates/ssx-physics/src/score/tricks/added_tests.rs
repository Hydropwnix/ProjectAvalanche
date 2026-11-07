use super::*;

const AIR_WALK: AddedTrick = AddedTrick { slot: 0, base: 3, hand: TrickHand::Both, points: 1300 };

fn tuning(added: AddedAirTricks) -> TrickTuning {
    TrickTuning { tweak_switch_threshold: FRAC_PI_4, virtual_stick_persist_time: 15.0, added }
}

fn with(slot: usize, t: AddedTrick) -> AddedAirTricks {
    let mut a = AddedAirTricks::default();
    a.by_input[slot] = Some(t);
    a
}

fn air(goofy: bool) -> TrickContext {
    TrickContext { in_air: true, enable_air_tricks: true, ticks_to_land: 1e35, goofy, ..TrickContext::default() }
}

fn press(y: bool, x: bool, b: bool) -> TrickInput {
    TrickInput { y, x, b, ..TrickInput::default() }
}

fn ride(t: &TrickTuning, ctx: &TrickContext, inputs: &[(TrickInput, usize)]) -> Vec<TrickType> {
    let mut s = TrickState::default();
    let mut out = Vec::new();
    for &(i, n) in inputs {
        for _ in 0..n {
            s.update(t, &i, ctx, 2.0);
            out.push(s.trick_type);
        }
    }
    out
}

fn twice(b: TrickInput) -> Vec<(TrickInput, usize)> {
    vec![(TrickInput::default(), 1), (b, 3), (TrickInput::default(), 3), (b, 12)]
}

#[test]
fn an_added_trick_is_chosen_by_its_double_after_the_discs_single() {
    let t = tuning(with(0, AIR_WALK));
    let types = ride(&t, &air(false), &twice(press(true, false, false)));
    let first = types.iter().position(|t| *t != TrickType::None).expect("a trick");
    assert_eq!(types[first], AIR_TRICKS[0].regular, "{types:?}");
    assert_eq!(types.last(), Some(&TrickType::Added(AIR_WALK)), "{types:?}");
    let at = types.iter().position(|t| matches!(t, TrickType::Added(_))).expect("chosen");
    assert_eq!((first, at), (2, 7), "{types:?}");
    let added = TrickType::Added(AIR_WALK);
    assert_eq!(
        (added.value(), added.points(), added.hand(), added.flow_base()),
        (32, 1300.0, TrickHand::Both, TrickType::Indy)
    );
    assert!(!added.is_grind());
}

#[test]
fn without_an_added_trick_the_same_presses_keep_the_discs_choice() {
    let disc = ride(&tuning(AddedAirTricks::default()), &air(false), &twice(press(true, false, false)));
    assert!(disc.iter().all(|t| !matches!(t, TrickType::Added(_))));
    assert_eq!(disc.last(), Some(&AIR_TRICKS[0].regular));
    let other =
        ride(&tuning(with(1, AddedTrick { slot: 1, ..AIR_WALK })), &air(false), &twice(press(true, false, false)));
    assert_eq!(other, disc);
}

#[test]
fn the_goofy_stance_takes_the_mirrored_input() {
    let left_left = AddedTrick { slot: 1, base: 4, hand: TrickHand::Forehand, points: 900 };
    let t = tuning(with(1, left_left));
    assert_eq!(ride(&t, &air(false), &twice(press(false, true, false))).last(), Some(&TrickType::Added(left_left)));
    assert_eq!(ride(&t, &air(true), &twice(press(false, false, true))).last(), Some(&TrickType::Added(left_left)));
    assert!(ride(&t, &air(true), &twice(press(false, true, false))).iter().all(|t| !matches!(t, TrickType::Added(_))));
    let t = tuning(with(0, AIR_WALK));
    assert_eq!(ride(&t, &air(true), &twice(press(true, false, false))).last(), Some(&TrickType::Added(AIR_WALK)));
}

#[test]
fn the_double_keeps_the_discs_windows() {
    let t = tuning(with(0, AIR_WALK));
    let y = press(true, false, false);
    let chosen =
        |runs: &[(TrickInput, usize)]| ride(&t, &air(false), runs).iter().any(|t| matches!(t, TrickType::Added(_)));
    let off = TrickInput::default();
    assert!(chosen(&[(off, 1), (y, 10), (off, 10), (y, 3)]));
    assert!(!chosen(&[(off, 1), (y, 11), (off, 3), (y, 3)]));
    assert!(!chosen(&[(off, 1), (y, 3), (off, 11), (y, 3)]));
    assert!(!chosen(&[(off, 1), (y, 30)]));
}

#[test]
fn the_values_are_the_discs_from_0_to_31_and_32_on() {
    for v in 0..32u8 {
        assert_eq!(TrickType::disc(v).map(TrickType::value), Some(v));
    }
    assert_eq!(TrickType::disc(32), None);
    for slot in 0..3u8 {
        assert_eq!(TrickType::Added(AddedTrick { slot, ..AIR_WALK }).value(), 32 + slot);
    }
    assert_eq!(TrickType::grind(14), Some(TrickType::Grind1));
    assert_eq!((TrickType::grind(13), TrickType::grind(32)), (None, None));
}

#[test]
fn the_points_cap_is_the_tables_largest() {
    assert_eq!(f32::from(ssx_content::trick::MAX_POINTS), ssx_rules::scoring::TRICK_POINTS_MAX);
    assert_eq!(ssx_rules::scoring::TRICK_POINTS_MAX, ssx_rules::scoring::TRICK_POINTS_SIGNATURE);
    let most = TrickType::Added(AddedTrick { points: ssx_content::trick::MAX_POINTS, ..AIR_WALK });
    assert!(most.points() <= TrickType::Signature.points());
}
