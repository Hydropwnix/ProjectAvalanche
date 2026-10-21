use ssx_assets::career::profile::BadgeText;

use crate::avm1::Ctx;
use crate::avm1::value::Value;
use crate::game::fe::{ActionParam, FrontEnd};
use crate::game::loc::Localization;

pub const DP_CATEGORIES: &str = "dp_badge_category_collection";
pub const DP_COLLECTION: &str = "dp_badge_collection";
pub const DP_INDEX: &str = "dp_badge_collection_index";
pub const DP_DETAILS: &str = "dp_badge_details";
pub const DP_RECENT: &str = "dp_badge_recent";
pub const CATEGORY_TITLES: [&str; 9] = [
    "TXT_BADGE_CATEGORY_CAMPAIGN",
    "TXT_BADGE_CATEGORY_EXPLORE",
    "TXT_BADGE_CATEGORY_GLOBALEVENTS",
    "TXT_BADGE_CATEGORY_ECONOMY",
    "TXT_BADGE_CATEGORY_RACE",
    "TXT_BADGE_CATEGORY_SURVIVE",
    "TXT_BADGE_CATEGORY_TRICKY",
    "TXT_BADGE_CATEGORY_TRICK",
    "TXT_BADGE_CATEGORY_TRACK_BONUS",
];
const EMPTY_ICON: &str = "assets/notificationIcons/empty_badge.dds";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct UiBadge {
    pub text: BadgeText,
    pub awarded: bool,
    pub percent: i32,
}

#[derive(Clone, Debug)]
pub struct Badges {
    pub active: bool,
    pub category: i32,
    pub index: i32,
    pub selecting_categories: bool,
    pub collection: Vec<UiBadge>,
    pub recent: Vec<UiBadge>,
    pub details: Option<UiBadge>,
}

impl Default for Badges {
    fn default() -> Self {
        Badges {
            active: false,
            category: 0,
            index: 0,
            selecting_categories: true,
            collection: Vec::new(),
            recent: Vec::new(),
            details: None,
        }
    }
}

fn collections(fe: &FrontEnd) -> (Vec<UiBadge>, Vec<UiBadge>) {
    let record = |key: u32, b: &BadgeText| UiBadge {
        text: b.clone(),
        awarded: fe.profile.stats.awarded(key),
        percent: (fe.badge_progress.get(&key).copied().unwrap_or(0.0) * 100.0) as i32,
    };
    let mut all: Vec<UiBadge> = fe.rules.badge_texts.iter().map(|(k, b)| record(*k, b)).collect();
    all.sort_by_key(|b| b.text.order);
    let recent =
        fe.profile.recent_badges.iter().filter_map(|k| fe.rules.badge_texts.get(k).map(|b| record(*k, b))).collect();
    (all, recent)
}

pub fn activate(fe: &mut FrontEnd) {
    if !fe.badges.active {
        let (collection, recent) = collections(fe);
        fe.badges = Badges { active: true, collection, recent, ..Default::default() };
        fe.note(format!(
            "badges: message 0xAE9953C7 {{1}} — {} badges, {} awarded, {} recent",
            fe.badges.collection.len(),
            fe.badges.collection.iter().filter(|b| b.awarded).count(),
            fe.badges.recent.len()
        ));
    }
    for dp in [DP_CATEGORIES, DP_COLLECTION, DP_INDEX, DP_DETAILS, DP_RECENT] {
        fe.publish.push(dp.into());
    }
}

pub fn deactivate(fe: &mut FrontEnd) {
    fe.badges = Badges::default();
}

pub fn action(fe: &mut FrontEnd, action: &str, param: &ActionParam) {
    match action {
        "A_ActivityFeedBadgesCategoryHighlight" => {
            fe.badges.category = param.field("index") as i32;
            fe.badges.selecting_categories = param.field("isSelectingCategories") > 0.0;
            fe.badges.index = 0;
            fe.publish.push(DP_COLLECTION.into());
            fe.publish.push(DP_INDEX.into());
            if !fe.badges.selecting_categories {
                fe.publish.push(DP_DETAILS.into());
            }
        }
        "A_ActivityFeedBadgesHighlight" => {
            fe.badges.index = param.number as i32;
            fe.publish.push(DP_DETAILS.into());
        }
        _ => {}
    }
}

pub fn build(fe: &mut FrontEnd, cx: &mut Ctx, loc: &mut Localization, dp: &str) -> Option<Value> {
    let in_category = |c: i32| fe.badges.collection.iter().filter(move |b| b.text.category == c);
    match dp {
        DP_INDEX => Some(Value::Number(0.0)),
        DP_CATEGORIES => {
            let mut out = Vec::new();
            for (c, title) in CATEGORY_TITLES.iter().enumerate() {
                let total = in_category(c as i32).count();
                let unlocked = in_category(c as i32).filter(|b| b.awarded).count();
                let o = Value::Object(cx.new_object());
                let progress = loc.format("TXT_BADGES_ACQUIRED_X_Y", &[&unlocked.to_string(), &total.to_string()]);
                for (k, v) in [
                    ("strItem", Value::str(&loc.text(title))),
                    ("numBadgesInCategory", Value::Number(total as f64)),
                    ("numUnlockedBadgesInCategory", Value::Number(unlocked as f64)),
                    ("progressTitle", Value::str(&progress)),
                ] {
                    let _ = cx.set_member(&o, k, v);
                }
                out.push(o);
            }
            Some(Value::Object(cx.new_array(out)))
        }
        DP_COLLECTION => {
            let mut out = Vec::new();
            for (i, b) in in_category(fe.badges.category).enumerate() {
                let o = Value::Object(cx.new_object());
                let icon = if b.awarded { b.text.icon.as_str() } else { EMPTY_ICON };
                let _ = cx.set_member(&o, "iconPath", Value::str(icon));
                let _ = cx.set_member(&o, "eventHighlight", Value::str("A_ActivityFeedBadgesHighlight"));
                let _ = cx.set_member(&o, "eventHighlightParam", Value::Number(i as f64));
                out.push(o);
            }
            Some(Value::Object(cx.new_array(out)))
        }
        DP_DETAILS => {
            let focus = in_category(fe.badges.category).nth(fe.badges.index.max(0) as usize).cloned();
            if in_category(fe.badges.category).next().is_some() {
                fe.badges.details = focus;
            }
            let o = Value::Object(cx.new_object());
            if let Some(b) = fe.badges.details.clone() {
                let (label, percent) = if b.awarded {
                    (loc.text("TXT_COMPLETE"), String::new())
                } else if b.percent == 0 {
                    (String::new(), String::new())
                } else {
                    (loc.text("TXT_BADGE_PROGRESS"), format!("{}%", b.percent))
                };
                for (k, v) in [
                    ("strTitle", Value::str(&loc.text(&b.text.title))),
                    ("strDescription", Value::str(&loc.text(&b.text.description))),
                    ("percentComplete", Value::Number(f64::from(b.percent))),
                    ("badgeProgressLabel", Value::str(&label)),
                    ("percentCompleteLabel", Value::str(&percent)),
                ] {
                    let _ = cx.set_member(&o, k, v);
                }
            }
            Some(o)
        }
        DP_RECENT => {
            let mut out = Vec::new();
            for b in &fe.badges.recent {
                let o = Value::Object(cx.new_object());
                let _ = cx.set_member(&o, "iconPath", Value::str(&b.text.icon));
                out.push(o);
            }
            Some(Value::Object(cx.new_array(out)))
        }
        _ => None,
    }
}
