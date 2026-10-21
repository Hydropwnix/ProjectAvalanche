use ssx_formats::attrib::fnv1;
use ssx_formats::rnf::CampaignGraph;

use super::data::DropType;
use super::profile::*;

const GRAPH: &str = "\
*startstate\nid: 1\ntype: drop\nattrib: a\n*endstate\n\
*startstate\nid: 2\ntype: drop\nattrib: b\n*endstate\n\
*startstate\nid: 3\ntype: drop\nattrib: c\n*endstate\n\
*startstate\nid: 4\ntype: unlock_char\nattrib: elise\n*endstate\n\
*startstate\nid: 5\ntype: open_range\nattrib: patagonia\n*endstate\n\
*startstate\nid: 6\ntype: unlock_chapter\nattrib: 2\n*endstate\n\
*starttransition\nsource: 1\ntarget: 2\n*endtransition\n\
*starttransition\nsource: 1\ntarget: 3\n*endtransition\n\
*starttransition\nsource: 2\ntarget: 3\n*endtransition\n\
*starttransition\nsource: 2\ntarget: 4\n*endtransition\n\
*starttransition\nsource: 1\ntarget: 5\n*endtransition\n\
*starttransition\nsource: 2\ntarget: 5\n*endtransition\n\
*starttransition\nsource: 3\ntarget: 6\n*endtransition\n";

fn rule(name: &str, drop_type: DropType, event_type: i64, reward: i64) -> EventRule {
    EventRule {
        key: fnv1(name.as_bytes()),
        name: name.to_string(),
        drop_type,
        event_type,
        reward,
        places: None,
        goal: None,
        cost: 0,
        peak: 0x10,
        range: 0x20,
    }
}

fn rules() -> Rules {
    let mut r = Rules {
        graph: CampaignGraph::parse(GRAPH.as_bytes()).unwrap(),
        explore_rewards: vec![[5000, 10000, 20000, 2500], [7500, 15000, 30000, 2500]],
        level_thresholds: vec![6000, 12000, 24000, 100000],
        level_prices: vec![0; 4],
        track_bonuses: [
            ("TrackBonus_1".to_string(), 1500),
            ("TrackBonus_2".to_string(), 1100),
            ("TrackBonus_6".to_string(), 2000),
            ("TrackBonus_7".to_string(), 1000),
        ]
        .into_iter()
        .collect(),
        riders: [("elise".to_string(), fnv1(b"elise"))].into_iter().collect(),
        rider_names: [(fnv1(b"elise"), "Elise".to_string())].into_iter().collect(),
        ..Default::default()
    };
    for e in [
        rule("a", DropType::Campaign, 7, 2000),
        rule("b", DropType::Campaign, 1, 3000),
        rule("c", DropType::Campaign, 3, 20000),
        rule("chapter_2_drop_h2h", DropType::Campaign, 7, 6500),
        rule("x", DropType::Explore, 6, 1),
        rule("t", DropType::Explore, 2, 0),
    ] {
        r.events.insert(e.key, e);
    }
    r
}

fn key(n: &str) -> u32 {
    fnv1(n.as_bytes())
}

#[test]
fn a_new_profile_has_the_game_defaults() {
    let p = Profile::new();
    assert_eq!(p.credits(), 4500);
    assert_eq!(p.attribs[&ATTRIB_LAST_RIDER], i64::from(key("zoe")));
    assert_eq!(p.attribs[&ATTRIB_FLAGS], 8);
    let (g, notes) = Profile::new_game(&rules());
    assert_eq!(g.drop_state(key("chapter_2_drop_h2h")), DropState::Unlocked);
    assert!(g.owned_items.is_empty());
    assert_eq!(g.attribs[&ATTRIB_FLAGS], 8);
    assert_eq!(notes[0], Notice::Movie("mainintro".into()));
    assert_eq!(g.peak_modes[&0x10], [0, 1, 0, 0, 0]);
}

#[test]
fn credits_clamp_and_earnings() {
    let mut p = Profile::new();
    p.add_credits(-10_000, true);
    assert_eq!((p.credits(), p.earned()), (0, 0));
    p.add_credits(i64::from(i32::MAX), true);
    p.add_credits(5, true);
    assert_eq!((p.credits(), p.earned()), (i64::from(i32::MAX), i64::from(i32::MAX)));
}

#[test]
fn completing_drops_unlocks_children_whose_parents_are_complete() {
    let r = rules();
    let mut p = Profile::new();
    p.unlock_drop(&r, key("a"));
    let n = p.complete_campaign_drop(&r, key("a"));
    assert_eq!(n, vec![Notice::DropUnlocked(key("b"))]);
    assert_eq!(p.drop_state(key("c")), DropState::Locked);
    p.unlock_drop(&r, key("b"));
    let n = p.complete_campaign_drop(&r, key("b"));
    assert!(n.contains(&Notice::DropUnlocked(key("c"))));
    assert!(n.contains(&Notice::RiderUnlocked("elise".into())));
    assert!(p.owned_items.contains(&key("elise")) && p.unlocked_riders.is_empty());
    assert_eq!(p.stats.records[&key("RiderUnlock_Elise")].value, 1);
    assert!(n.contains(&Notice::RangeOpened("patagonia".into())));
    let n = p.complete_campaign_drop(&r, key("c"));
    assert_eq!(n, vec![Notice::ChapterUnlocked(2)]);
    assert_eq!(p.chapter_drops.len(), 6);
    assert_eq!(p.range_modes[&0x20][1], 2);
}

#[test]
fn an_older_port_save_owns_its_campaign_riders() {
    let r = rules();
    let mut p = Profile::new();
    p.unlocked_riders.insert(key("elise"));
    p.own_campaign_riders(&r);
    assert!(!p.owned_items.contains(&key("elise")), "its unlock_char node is not reached yet");
    p.drop_states.insert(key("b"), DropState::Completed);
    p.own_campaign_riders(&r);
    assert!(p.owned_items.contains(&key("elise")) && p.unlocked_riders.is_empty());
}

#[test]
fn rider_levels_follow_the_thresholds_and_the_purchase_gate() {
    let thr = [6000, 12000, 24000, 100000];
    let level = |money: i64, prices: &[i64], bought: i64| {
        let mut s = RiderStats { money_earned: money, max_purchased_level: bought, ..Default::default() };
        update_rider_level(&mut s, &thr, prices)
    };
    assert_eq!(level(0, &[0; 4], 0), (0, 0));
    assert_eq!(level(6000, &[0; 4], 0), (0, 100));
    assert_eq!(level(9000, &[0; 4], 0), (1, 50));
    assert_eq!(level(10_000_000, &[0; 4], 0).0, 3);
    assert_eq!(level(30000, &[0, 0, 100, 0], 0).0, 1);
    assert_eq!(level(30000, &[0, 0, 100, 0], 2).0, 3);
}

#[test]
fn campaign_results_pay_once_in_full_then_capped() {
    let r = rules();
    let mut p = Profile::new();
    p.unlock_drop(&r, key("a"));
    let o = EventOutcome { event: key("a"), rider: key("zoe"), place: 0, ..Default::default() };
    let rep = p.apply_result(&r, &o);
    assert!(rep.success);
    assert_eq!((rep.reward, rep.bonus, rep.total), (2000, 500, 2500));
    assert_eq!(p.credits(), 4500 + 2500);
    assert_eq!(p.drop_state(key("a")), DropState::Completed);
    assert_eq!(p.riders[&key("zoe")].money_earned, 2500);
    let rep = p.apply_result(&r, &EventOutcome { track_bonuses: vec!["TrackBonus_7".into()], ..o.clone() });
    assert_eq!((rep.reward, rep.bonus), (2000, 1000));
    let rep = p.apply_result(&r, &EventOutcome { place: 3, ..o });
    assert!(!rep.success && rep.reward == 0);
    assert_eq!(p.drop_counts[&key("a")], DropCounts { completions: 2, failures: 1, unk_flag: 0 });
}

#[test]
fn success_rules_by_event_type() {
    let mut r = rule("q", DropType::Campaign, 0, 0);
    r.goal = Some(13000);
    let o = |value| EventOutcome { value, ..Default::default() };
    assert!(!campaign_success(&r, &o(13000)) && campaign_success(&r, &o(13001)));
    r.event_type = 1;
    r.places = Some(3);
    let bots = EventOutcome { value: 100, bot_scores: vec![200, 300, 50], ..Default::default() };
    assert!(campaign_success(&r, &bots));
    r.places = Some(1);
    assert!(!campaign_success(&r, &bots));
    r.event_type = 4;
    assert!(campaign_success(&r, &EventOutcome { failed: true, ..Default::default() }));
    r.event_type = 3;
    assert!(!campaign_success(&r, &EventOutcome { failed: true, ..Default::default() }));
}

#[test]
fn explore_medals_pay_the_tier_and_rank_ties_to_the_player() {
    assert_eq!(leaderboard_rank(true, 90_000, &[77_532, 92_631, 120_965]), 1);
    assert_eq!(leaderboard_rank(true, 92_631, &[77_532, 92_631, 120_965]), 1);
    assert_eq!(leaderboard_rank(false, 5, &[10, 7, u32::MAX]), 2);
    assert_eq!(leaderboard_rank(true, u32::MAX, &[1, 2, 3]), 3);
    let r = rules();
    let mut p = Profile::new();
    let o = EventOutcome {
        event: key("x"),
        value: 90_000,
        leaderboard: vec![77_532, 92_631, 120_965],
        ..Default::default()
    };
    let rep = p.apply_result(&r, &o);
    assert_eq!((rep.medal, rep.reward, rep.bonus), (Some(Medal::Silver), 15000, 500));
    assert_eq!(p.medals[&key("x")], [1, 1, 0]);
    assert_eq!(p.explore_scores[&key("x")].score, 90_000);
    let rep = p.apply_result(&r, &EventOutcome { value: 130_000, ..o });
    assert_eq!((rep.medal, rep.total), (None, 0));
    assert_eq!(p.explore_scores[&key("x")].attempts, 2);
    assert_eq!(p.explore_scores[&key("x")].score, 90_000);
}

#[test]
fn only_three_track_bonuses_pay() {
    let r = rules();
    let mut p = Profile::new();
    let names = ["TrackBonus_1", "TrackBonus_2", "TrackBonus_6", "TrackBonus_7"].map(String::from).to_vec();
    let rep = p.apply_result(
        &r,
        &EventOutcome { event: key("t"), leaderboard: vec![], track_bonuses: names, ..Default::default() },
    );
    assert_eq!(rep.bonus, 2000 + 1500 + 1100 + 500);
    assert_eq!((rep.medal, rep.reward), (Some(Medal::Gold), 20000));
}

#[test]
fn geotag_credits_stay_paid_after_a_failed_run() {
    let r = rules();
    let mut p = Profile::new();
    p.add_credits(700, true);
    let failed = EventOutcome { event: key("t"), failed: true, geotag_credits: 700, ..Default::default() };
    let rep = p.apply_result(&r, &failed);
    assert_eq!((rep.bonus, rep.total), (0, 0));
    assert_eq!(p.credits(), 4500 + 700);
    p.add_credits(300, true);
    let ok = EventOutcome { event: key("t"), geotag_credits: 300, ..Default::default() };
    let rep = p.apply_result(&r, &ok);
    assert_eq!(rep.bonus, 300);
    assert_eq!(p.credits(), 4500 + 700 + rep.total);
}

#[test]
fn a_skipped_drop_completes_with_two_completions() {
    let r = rules();
    let mut p = Profile::new();
    p.unlock_drop(&r, key("a"));
    let failed = EventOutcome { event: key("a"), failed: true, ..Default::default() };
    for _ in 0..3 {
        let _ = p.apply_result(&r, &failed);
    }
    assert_eq!(p.drop_counts[&key("a")].failures, 3);
    let _ = p.skip_campaign_drop(&r, key("a"));
    assert_eq!(p.drop_state(key("a")), DropState::Completed);
    assert_eq!(p.drop_counts[&key("a")].completions, 2);
}

#[test]
fn explore_drops_are_bought_once() {
    let mut r = rules();
    r.events.get_mut(&key("x")).unwrap().cost = 5000;
    let mut p = Profile::new();
    assert_eq!(p.buy_drop(&r, key("x")), Err(BuyError::NotEnoughCredits { cost: 5000, credits: 4500 }));
    p.add_credits(1000, false);
    assert_eq!(p.buy_drop(&r, key("x")), Ok(5000));
    assert_eq!(p.credits(), 500);
    assert_eq!(p.buy_drop(&r, key("x")), Ok(0));
}

#[test]
fn the_new_game_flow_runs_from_the_menu_to_the_globe() {
    use super::new_game::{FlowAction, FrontEnd, NEW_GAME_DONE, run_new_game};
    let r = rules();
    let mut p = Profile::new();
    assert_eq!(p.new_game_state, 0);
    assert!(p.new_game_active());
    assert!(p.new_game_update(&r, &FrontEnd::default()).is_empty());
    let mut launched = Vec::new();
    let log = run_new_game(&mut p, &r, |_, d| launched.push(d));
    assert_eq!(launched, vec![key("chapter_2_drop_h2h")]);
    assert_eq!(p.new_game_state, NEW_GAME_DONE);
    assert!(!p.new_game_active() && !p.start_new_game());
    assert_eq!(p.drop_state(key("chapter_2_drop_h2h")), DropState::Completed);
    assert_eq!(p.globe_last_drop, key("chapter_2_drop_gen_1"));
    let movies: Vec<&str> =
        log.iter().filter_map(|(_, a)| if let FlowAction::PlayMovie(m) = a { Some(*m) } else { None }).collect();
    assert_eq!(movies, ["mainintro", "rockies"]);
    assert!(log.iter().any(|(s, a)| *s == 7
        && *a
            == FlowAction::GlobeProgression {
                from: key("chapter_1_drop_h2h"),
                to: vec![key("chapter_2_drop_h2h")],
                flag: false
            }));
    assert_eq!(p.drop_state(key("chapter_1_drop_h2h")), DropState::Locked);
    p.load_new_game_state(0xC);
    assert_eq!(p.new_game_state, NEW_GAME_DONE);
    p.load_new_game_state(9);
    assert_eq!(p.new_game_state, 0);
}

#[test]
fn tutorial_drops_and_range_visits() {
    let mut p = Profile::new();
    assert!(p.is_tutorial_drop(key("chapter_1_drop_h2h")));
    assert!(p.is_tutorial_drop(key("chapter_2_drop_h2h")));
    assert!(!p.is_tutorial_drop(key("chapter_2_drop_gen_2")));
    p.drop_states.insert(key("chapter_2_drop_gen_1"), DropState::Completed);
    assert!(!p.is_tutorial_drop(key("chapter_2_drop_h2h")));
    assert_eq!(p.range_visits_by(7, "Zoe"), 0);
    assert_eq!(p.visit_range_by(7, "Zoe"), 1);
    assert_eq!(p.visit_range_by(7, "Zoe"), 2);
    assert_eq!(p.range_visits_by(7, "Elise"), 0);
    assert_eq!(p.visit_range_by(7, "Elise"), 1);
    assert_eq!(p.visit_range_by(8, "Zoe"), 1);
    let key = fnv1(b"NumTimesVisitedRange_7Zoe");
    let r = p.stats.records[&key];
    assert_eq!((r.value, r.key_again, r.scope, r.kind, r.initial), (2, key, 2, 0, 0));
}
