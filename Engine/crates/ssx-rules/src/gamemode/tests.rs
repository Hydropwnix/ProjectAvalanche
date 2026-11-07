use super::*;

#[test]
fn the_targets_are_the_board() {
    let r = Rules::plain(EventType::RaceGhosts, None).unwrap();
    assert_eq!(r.board_values(), [120_000, 180_000, 240_000], "no targets: the padding (k + 2) × 60 000");
    let r = Rules::new(EventType::TrickGhosts, &[90_000, 60_000, 30_000], None, None).unwrap();
    assert_eq!(r.board_values(), [90_000, 60_000, 30_000]);
    let none = Rules::plain(EventType::SurviveToBottom, None).unwrap();
    assert!(none.board().is_empty());
    assert_eq!(none.rivals(), Rivals::None);
    assert_eq!(Rules::plain(EventType::RaceBots, None).unwrap().rivals(), Rivals::Racers);
    assert_eq!(Rules::plain(EventType::RaceClock, None).unwrap().rivals(), Rivals::Board);
}

#[test]
fn the_medal_is_the_walk_over_the_targets() {
    let r = Rules::new(EventType::RaceGhosts, &[75_000, 90_000, 120_000], None, None).unwrap();
    assert_eq!(r.outcome(false, 75_000), (Some(0), true), "a tie is the player's");
    assert_eq!(r.outcome(false, 80_000), (Some(1), true));
    assert_eq!(r.outcome(false, 120_001), (Some(3), false));
    assert_eq!(r.outcome(true, 1), (None, false));
    let e = r.run_end(false, 100_000);
    assert!(e.bronze && e.won && e.code == 3);
    let t = Rules::new(EventType::TrickScore, &[50_000, 40_000, 30_000], None, None).unwrap();
    assert_eq!(t.outcome(false, 50_000), (Some(0), true));
    assert_eq!(t.outcome(false, 49_999), (Some(1), true));
}

#[test]
fn targets_that_cannot_hold_are_refused() {
    use RulesError::*;
    assert_eq!(check_targets(EventType::RaceClock, &[1, 2, 3, 4]), Err(TargetCount { count: 4 }));
    assert_eq!(check_targets(EventType::RaceClock, &[1, 2]), Err(TargetCount { count: 2 }));
    let free = EventType::FreeRide;
    assert_eq!(check_targets(free, &[3, 2, 1]), Err(NoBoard { event_type: free }));
    let bottom = EventType::SurviveToBottom;
    assert_eq!(check_targets(bottom, &[3, 2, 1]), Err(NoBoard { event_type: bottom }));
    let race = EventType::RaceClock;
    assert_eq!(check_targets(race, &[0, 1, 2]), Err(OutOfRange { index: 0, value: 0, max: MAX_CLOCK_MS }));
    let over = MAX_CLOCK_MS + 1;
    assert_eq!(check_targets(race, &[1, 2, over]), Err(OutOfRange { index: 2, value: over, max: MAX_CLOCK_MS }));
    assert_eq!(check_targets(race, &[90_000, 90_000, 95_000]), Err(Order { index: 1 }));
    assert_eq!(check_targets(race, &[90_000, 95_000, 80_000]), Err(Order { index: 2 }));
    assert_eq!(check_targets(EventType::TrickBots, &[10, 20, 5]), Err(Order { index: 1 }));
    assert!(check_targets(EventType::SurviveToDeath, &[3_000_000, 2_000_000, 1_000_000]).is_ok());
    assert_eq!(Rules::new(EventType::RaceClock, &[], Some(0), None), Err(TimeLimit { ms: 0 }));
    assert_eq!(Rules::new(EventType::RaceClock, &[], None, Some(TimeUp::Result)), Err(RaceResultAtTimeUp));
}

#[test]
fn the_time_up_end_is_the_death_timers() {
    assert_eq!(TimeUp::of(EventType::SurviveToDeath), TimeUp::Result);
    for t in [EventType::RaceClock, EventType::TrickScore, EventType::SurviveToBottom, EventType::FreeRide] {
        assert_eq!(TimeUp::of(t), TimeUp::Fail);
    }
    let r = Rules::new(EventType::TrickScore, &[], Some(60_000), Some(TimeUp::Result)).unwrap();
    assert_eq!((r.time_limit_ms(), r.time_up()), (Some(60_000), TimeUp::Result));
}

#[test]
fn the_event_types_flags() {
    let f = |t| {
        let r = Rules::plain(t, None).unwrap();
        (r.rewinds_infinite(), r.heli_interaction())
    };
    assert_eq!(f(EventType::SurviveToDeath), (Some(false), Some(false)));
    assert_eq!(f(EventType::RaceBots), (Some(true), Some(false)));
    assert_eq!(f(EventType::TrickGhosts), (Some(true), Some(true)));
    assert_eq!(f(EventType::FreeRide), (None, None));
}

#[test]
fn the_record_keeps_what_the_results_keep() {
    let mut r = Record::default();
    let run = |value, failed, rewinds| RunOutcome { value, failed, rewinds };
    assert!(r.add(EventType::RaceClock, run(90_000, false, 2)));
    assert!(!r.add(EventType::RaceClock, run(90_000, false, 1)), "an equal time is not better");
    assert!(!r.add(EventType::RaceClock, run(80_000, true, 5)), "a failed race is not kept");
    assert!(r.add(EventType::RaceClock, run(80_000, false, 3)));
    assert_eq!(r, Record { best: Some(80_000), attempts: 4, deaths: 1, fewest_rewinds: Some(1) });
    let mut s = Record::default();
    assert!(s.add(EventType::SurviveToDeath, run(1_500_000, true, 0)), "a survive run's death is kept");
    assert!(!is_better(EventType::TrickScore, u32::MAX, None));
    let mut full = Record { attempts: MAX_COUNT, deaths: MAX_COUNT, ..Record::default() };
    full.add(EventType::TrickScore, run(1, true, 0));
    assert_eq!((full.attempts, full.deaths), (MAX_COUNT, MAX_COUNT));
}
