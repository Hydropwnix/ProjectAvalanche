use std::collections::BTreeMap;
use std::path::Path;

use ssx_lua::game::objectives::{Effect, Name, Runner, StatKey, StatValue, World};
use ssx_physics::objectives::{DropScripts, GAME_SCRIPTS, LevelObjectives, LoadWorld};
use ssx_physics::stats::StatsStore;

use super::FrontEnd;
use crate::game::loc::Localization;

#[derive(Clone, Debug)]
pub struct FeScripts {
    pub runner: Runner,
    pub problems: usize,
}

impl FeScripts {
    pub fn load(big_root: &Path, permanent_flags: &[u32]) -> Self {
        let scripts = DropScripts::load(big_root, Vec::new());
        let game = scripts.named(&GAME_SCRIPTS);
        let mut o = LevelObjectives::load(&game, &[], LoadWorld::default(), false, None, permanent_flags);
        o.runner.level_active = false;
        FeScripts { runner: o.runner, problems: o.report.problems.len() }
    }
}

struct FeWorld<'a> {
    store: &'a mut StatsStore,
    effects: Vec<(String, Effect)>,
}

impl World for FeWorld<'_> {
    fn stat(&mut self, key: &StatKey) -> Option<StatValue> {
        self.store.get(key)
    }
    fn badge_set(&mut self, badge: &Name, category: i32) -> Option<bool> {
        Some(self.store.awarded(badge.hash, usize::try_from(category).unwrap_or(2)))
    }
    fn track_bonus_set(&mut self, bonus: &Name) -> Option<bool> {
        Some(self.store.awarded(bonus.hash, 0))
    }
    fn effect(&mut self, objective: &Name, e: &Effect) {
        match e {
            Effect::AwardBadge { badge, .. } => self.store.award(&badge.text, 2),
            Effect::AwardTrackBonus { bonus, .. } => self.store.award(&bonus.text, 0),
            _ => {}
        }
        self.effects.push((objective.text.to_string(), e.clone()));
    }
}

pub fn update(fe: &mut FrontEnd, loc: &mut Localization, dt: f32) {
    if fe.in_drop {
        return;
    }
    let Some(mut scripts) = fe.scripts.take() else { return };
    let mut store = StatsStore::default();
    crate::game::stats::into_session(&mut store, &fe.profile, &BTreeMap::new());
    let mut world = FeWorld { store: &mut store, effects: Vec::new() };
    scripts.runner.update(dt, &mut world);
    let effects = std::mem::take(&mut world.effects);
    fe.scripts = Some(scripts);
    let reported = effects.iter().filter(|(_, e)| matches!(e, Effect::BadgeProgress { .. })).count();
    if reported > 0 {
        fe.note(format!("front end: set A: {reported} badge progress report(s)"));
    }
    for (objective, e) in effects {
        match e {
            Effect::BadgeProgress { badge, progress } => {
                fe.badge_progress.insert(badge.hash, progress);
            }
            Effect::AwardBadge { badge, .. } => {
                let Some(b) = fe.rules.badge_texts.get(&badge.hash).cloned() else { continue };
                if fe.profile.award_badge(badge.hash) {
                    fe.note(format!("front end: {objective}: badge {} awarded ({})", b.name, b.title));
                    super::notify::post_badge(fe, loc, &b);
                }
            }
            _ => {}
        }
    }
}
