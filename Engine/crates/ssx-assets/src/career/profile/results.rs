use super::*;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EventOutcome {
    pub event: u32,
    pub rider: u32,
    pub failed: bool,
    pub value: u32,
    pub place: u32,
    pub bot_scores: Vec<u32>,
    pub leaderboard: Vec<u32>,
    pub rewinds: u32,
    pub splits: [u32; 4],
    pub track_bonuses: Vec<String>,
    pub places_passed: u32,
    pub geotag_credits: i64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResultReport {
    pub success: bool,
    pub medal: Option<Medal>,
    pub reward: i64,
    pub bonus: i64,
    pub leaderboard: i64,
    pub total: i64,
    pub level: (i64, i64),
    pub notices: Vec<Notice>,
}

pub fn campaign_success(rule: &EventRule, o: &EventOutcome) -> bool {
    if o.failed && rule.event_type != 4 {
        return false;
    }
    let allowed = rule.places.filter(|&n| n > 0).map_or(0, |n| n - 1);
    match rule.event_type {
        0 => rule.goal.unwrap_or(0) < i64::from(o.value),
        1 => o.bot_scores.iter().filter(|&&s| s > o.value).count() as i64 <= allowed,
        2..=6 => true,
        7 => i64::from(o.place) <= allowed,
        _ => false,
    }
}

pub fn leaderboard_rank(ascending: bool, value: u32, rows: &[u32]) -> usize {
    rows.iter()
        .position(|&row| row == u32::MAX || (value != u32::MAX && if ascending { value <= row } else { row <= value }))
        .unwrap_or(rows.len())
}

pub fn is_better(ascending: bool, new: u32, old: u32) -> bool {
    if new == u32::MAX {
        return false;
    }
    if old == u32::MAX {
        return true;
    }
    if ascending { new < old } else { new > old }
}

impl Profile {
    pub fn apply_result(&mut self, rules: &Rules, o: &EventOutcome) -> ResultReport {
        let mut r = ResultReport::default();
        let Some(rule) = rules.event(o.event).cloned() else { return r };
        let had_record = self.explore_scores.contains_key(&o.event);
        match rule.drop_type {
            DropType::Campaign => {
                r.success = campaign_success(&rule, o);
                if r.success && !o.failed {
                    r.reward = rule.reward;
                    if self.drop_state(o.event) == DropState::Completed {
                        r.reward = r.reward.min(CAMPAIGN_REPLAY_CAP);
                    }
                }
                let c = self.drop_counts.entry(o.event).or_default();
                if r.success {
                    c.completions += 1
                } else {
                    c.failures += 1
                }
                self.globe_last_drop = o.event;
                if r.success && self.drop_state(o.event) == DropState::Unlocked {
                    r.notices = self.complete_campaign_drop(rules, o.event);
                }
                let mut movies = Vec::new();
                self.fire_movies(o.event, Some(r.success), &mut movies);
                r.notices.extend(movies);
            }
            DropType::Explore if !o.failed => {
                if rule.event_type == 3 {
                    r.success = true;
                } else {
                    let rank = leaderboard_rank(rule.ascending(), o.value, &o.leaderboard);
                    r.medal = Medal::from_rank(rank);
                    r.success = r.medal.is_some();
                    if let Some(m) = r.medal {
                        let counts = self.medals.entry(o.event).or_default();
                        for (k, c) in counts.iter_mut().enumerate() {
                            *c += u32::from(k <= m as usize);
                        }
                        let tier = rules.explore_rewards.get(rule.reward.clamp(0, 4) as usize);
                        r.reward = tier.map_or(0, |t| t[m as usize]);
                    }
                }
            }
            _ => {}
        }
        if !o.failed {
            let mut paid: Vec<i64> =
                o.track_bonuses.iter().filter_map(|b| rules.track_bonuses.get(b).copied()).collect();
            paid.sort_by(|a, b| b.cmp(a));
            r.bonus = paid.iter().take(PAID_TRACK_BONUSES).sum::<i64>();
            if !had_record {
                r.bonus += FIRST_RUN_BONUS;
            } else if rules.track_bonuses.is_empty() {
                r.bonus += REPEAT_BONUS;
            }
            r.bonus += o.geotag_credits;
            if rule.drop_type != DropType::Campaign {
                let per = rules.explore_rewards.get(rule.reward.clamp(0, 4) as usize).map_or(0, |t| t[3]);
                r.leaderboard = per * i64::from(o.places_passed);
            }
        }
        r.total = r.reward + r.bonus + r.leaderboard;
        if !o.failed {
            self.add_credits(-o.geotag_credits, true);
        }
        self.add_credits(r.total, true);
        let stats = self.riders.entry(o.rider).or_default();
        let before = update_rider_level(&mut stats.clone(), &rules.level_thresholds, &rules.level_prices).0;
        stats.money_earned += r.total;
        r.level = update_rider_level(stats, &rules.level_thresholds, &rules.level_prices);
        if r.level.0 > before {
            r.notices.push(Notice::LevelUp { rider: o.rider, level: r.level.0 });
        }
        self.record_score(&rule, o);
        r
    }

    fn record_score(&mut self, rule: &EventRule, o: &EventOutcome) {
        let row = self.explore_scores.entry(o.event).or_insert(ExploreScore { score: u32::MAX, ..Default::default() });
        let survive = matches!(rule.event_type, 3 | 4);
        if is_better(rule.ascending(), o.value, row.score) && (survive || !o.failed) {
            row.score = o.value;
            row.splits = o.splits;
        }
        if row.attempts == 0 || o.rewinds < row.rewinds {
            row.rewinds = o.rewinds;
        }
        row.attempts += 1;
        row.deaths += u32::from(o.failed);
    }
}
