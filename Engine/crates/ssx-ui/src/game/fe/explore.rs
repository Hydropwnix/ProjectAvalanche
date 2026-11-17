use ssx_assets::career::{DropType, Profile};

use super::FrontEnd;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExploreList {
    pub drop: u32,
    pub own_best: Option<u32>,
}

impl ExploreList {
    pub fn build(profile: &Profile, drop: u32) -> Self {
        let own_best = profile.explore_scores.get(&drop).map(|s| s.score).filter(|&s| s != u32::MAX);
        ExploreList { drop, own_best }
    }

    pub fn own_best(&self, drop: u32) -> Option<u32> {
        if self.drop == drop { self.own_best } else { None }
    }
}

pub fn drop_medal(fe: &FrontEnd, drop: u32) -> i32 {
    fe.profile.medals.get(&drop).and_then(|m| (0..3).rev().find(|&k| m[k] > 0)).map_or(-1, |k| k as i32)
}

pub fn medal_percent(medal: i32) -> f32 {
    match medal {
        2 => 100.0,
        1 => f32::from_bits(0x4285_5555),
        0 => f32::from_bits(0x4205_5555),
        _ => 0.0,
    }
}

fn node_drops(fe: &FrontEnd, kind: i32, key: u32) -> Vec<u32> {
    fe.career
        .events
        .iter()
        .filter(|e| e.drop_type == DropType::Explore)
        .filter(|e| {
            let (peak, range) = fe.career.peak_and_range(e);
            match kind {
                0 => true,
                1 => range == key,
                2 => peak == key,
                _ => false,
            }
        })
        .map(|e| e.key)
        .collect()
}

pub fn node_medal(fe: &FrontEnd, kind: i32, key: u32) -> (i32, f32) {
    if kind == 3 {
        let m = drop_medal(fe, key);
        return (m, medal_percent(m));
    }
    if !(0..=2).contains(&kind) {
        return (-1, 0.0);
    }
    let drops = node_drops(fe, kind, key);
    if drops.is_empty() {
        return (-1, 0.0);
    }
    let mut medal = 2;
    let mut sum = 0.0;
    for d in &drops {
        let m = drop_medal(fe, *d);
        sum += medal_percent(m);
        medal = medal.min(m);
    }
    (medal, sum / drops.len() as f32)
}

pub fn node_counts(fe: &FrontEnd, kind: i32, key: u32) -> (i32, i32) {
    if kind == 3 {
        return (i32::from(drop_medal(fe, key) >= 0), 0);
    }
    let drops = node_drops(fe, kind, key);
    let with = drops.iter().filter(|&&d| drop_medal(fe, d) >= 0).count();
    (with as i32, drops.len() as i32)
}

pub fn fe_medal(fe: &FrontEnd, kind: i32, key: u32) -> (i32, f32) {
    if fe.game_mode == 0 { node_medal(fe, kind, key) } else { (-1, 0.0) }
}

#[cfg(test)]
mod tests {
    use ssx_assets::career::DropType;
    use ssx_assets::career::profile::{EventRule, ExploreScore, Profile, Rules};
    use ssx_physics::event::{EventResult, EventType};

    use super::ExploreList;
    use crate::game::leaderboard::{ExploreFrame, ExploreLeaderboard};
    use crate::game::loc::{Language, Localization};
    use crate::game::values::{UiValues, id};

    const DROP: u32 = 0x3640_F3B7;

    #[test]
    fn the_list_holds_the_profile_s_best_of_its_drop() {
        let mut p = Profile::new();
        assert_eq!(ExploreList::build(&p, DROP), ExploreList { drop: DROP, own_best: None }, "no row");
        p.explore_scores.insert(DROP, ExploreScore { score: u32::MAX, attempts: 1, ..Default::default() });
        assert_eq!(ExploreList::build(&p, DROP).own_best, None, "a row whose SCORE is −1");
        p.explore_scores.insert(DROP, ExploreScore { score: 56_000_000, attempts: 2, ..Default::default() });
        let list = ExploreList::build(&p, DROP);
        assert_eq!(list.own_best(DROP), Some(56_000_000));
        assert_eq!(list.own_best(DROP + 1), None, "another drop's list holds no row of this one");
    }

    #[test]
    fn a_restart_after_a_new_best_rides_against_the_new_best() {
        let mut profile = Profile::new();
        profile.explore_scores.insert(DROP, ExploreScore { score: 1_030_436, attempts: 1, ..Default::default() });
        let mut rules = Rules::default();
        rules.events.insert(
            DROP,
            EventRule {
                key: DROP,
                name: "exp_himwest_everest_drop_1_trick".into(),
                drop_type: DropType::Explore,
                event_type: EventType::TrickGhosts as i64,
                reward: 0,
                places: None,
                goal: None,
                cost: 10_000,
                peak: 0,
                range: 0,
                ghost_drop: false,
            },
        );
        let screen = ExploreList::build(&profile, DROP);
        let value = 56_000_000;
        let r = EventResult {
            event_type: Some(EventType::TrickGhosts),
            points: value,
            time_ms: 400_000,
            descent_mm: 0,
            rewinds: 0,
            loops: 0,
            failed: false,
            value,
            rivals: vec![3_328_424, 1_030_436, 607_859],
            rank: Some(0),
            medal: None,
            place: 0,
            geotag_credits: 0,
            geotag_count: 0,
        };
        let ctx = crate::game::ride::ResultsContext {
            drop_key: DROP,
            event_type: EventType::TrickGhosts as i32,
            ..Default::default()
        };
        crate::game::ride::career_results(&ctx, &r, &mut profile, &rules);
        assert_eq!(profile.explore_scores[&DROP].score, value, "the profile keeps the new best");
        assert_eq!(screen.own_best(DROP), Some(1_030_436), "the drop screen's list until it is built again");
        let list = ExploreList::build(&profile, DROP);
        let mut v = UiValues::new();
        let mut loc = Localization::empty(Language::English);
        let mut board = ExploreLeaderboard::default();
        board.rivals = vec![("gold".into(), 3_328_424), ("silver".into(), 1_030_436), ("bronze".into(), 607_859)];
        let frame = ExploreFrame {
            event_type: EventType::TrickGhosts,
            player: "Player",
            picture: "",
            value: 0.0,
            own_best: list.own_best(DROP),
        };
        board.update(&mut v, &mut loc, frame);
        let own: Vec<_> =
            v.rows(id::LEADERBOARD).iter().filter(|r| r.name == "Player" && !r.is_player).map(|r| r.value).collect();
        assert_eq!(own, [56_000_000], "the own row is the new best");
    }
}
