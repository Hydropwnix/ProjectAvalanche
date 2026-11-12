//! A port addition, not the game's: single-player mods read the profile's credits and riders' XP and set them
//! (`getProfile`, `setCredits`, `setRiderXp`). Nothing here runs in a network session: the frame carries no profile
//! there and orders that still arrive are refused.

use bevy::prelude::*;
use ssx_assets::career::profile::{
    Profile, RiderStats, level_count, set_level_cap, set_rider_level, update_rider_level,
};
use ssx_script::host::game::{PROFILE_MAX, ProfileOrder, ProfileRiderView, ProfileView};
use ssx_ui::game::fe::FrontEnd;
use ssx_ui::game::fe::pause::GameRequest;
use ssx_ui::game::values::UiValues;

/// The value the front end adds to the bank on screen at its next update (the store's and the results' path).
const BANK_ADJUST: u16 = 0x93;
/// The bank the front end shows (`USER_bank`).
const BANK: u16 = 0x228;
const NAME_MAX: usize = 64;
const DISPLAY_MAX: usize = 128;

fn clip(s: &str, max: usize) -> String {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// The level as the game shows it (its count from 0, plus 1).
fn level_of(fe: &FrontEnd, stats: &RiderStats) -> i64 {
    let mut s = stats.clone();
    (update_rider_level(&mut s, &fe.rules.level_thresholds, &fe.rules.level_prices).0 + 1).clamp(1, 64)
}

/// The profile as the scripts see it this frame; `None` in a network session or with no front end.
pub(super) fn view(fe: Option<&FrontEnd>, in_session: bool) -> Option<ProfileView> {
    if in_session {
        return None;
    }
    let fe = fe?;
    let p: &Profile = &fe.profile;
    let riders = fe
        .career
        .riders
        .iter()
        .filter(|r| !r.name.is_empty() && r.name.len() <= NAME_MAX)
        .map(|r| {
            let stats = p.riders.get(&r.key).cloned().unwrap_or_default();
            ProfileRiderView {
                key: r.key,
                name: r.name.clone(),
                display_name: clip(&r.display_name, DISPLAY_MAX),
                xp: stats.money_earned.clamp(0, PROFILE_MAX),
                level: level_of(fe, &stats),
            }
        })
        .collect();
    Some(ProfileView {
        credits: p.credits().clamp(0, PROFILE_MAX),
        infinite_credits: p.infinite_credits,
        unlimited_gear: i64::from(p.unlimited_gear),
        shop_min_rarity: p.shop_min_rarity.clamp(0, 4),
        max_level: (level_count(&fe.rules.level_thresholds) as i64).clamp(0, 64),
        riders,
    })
}

/// Applies the scripts' profile orders to the front end's profile and asks for a save.
pub(super) fn apply(orders: Vec<ProfileOrder>, fe: Option<&mut FrontEnd>, values: &mut UiValues, in_session: bool) {
    if orders.is_empty() {
        return;
    }
    if in_session {
        warn!("scripts: {} profile changes refused: a network session's profile is the server's", orders.len());
        return;
    }
    let Some(fe) = fe else {
        warn!("scripts: {} profile changes refused: the front end is not up", orders.len());
        return;
    };
    for o in orders {
        match o {
            ProfileOrder::Credits { package, credits } => {
                if fe.profile.infinite_credits {
                    info!("scripts: {package}'s credits {credits} not set: infinite credits are on");
                    continue;
                }
                let before = fe.profile.credits();
                let delta = credits.clamp(0, PROFILE_MAX) - before;
                fe.profile.add_credits(delta, false);
                let step = delta.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
                let shown = values.int(BANK_ADJUST);
                values.set_int(BANK_ADJUST, shown.saturating_add(step));
                info!("scripts: {package} set the credits {before} -> {}", fe.profile.credits());
            }
            ProfileOrder::RiderXp { package, rider, xp } => {
                let (thresholds, prices) = (fe.rules.level_thresholds.clone(), fe.rules.level_prices.clone());
                let stats = fe.profile.riders.entry(rider).or_default();
                let before = stats.money_earned;
                let xp = xp.clamp(0, PROFILE_MAX);
                if xp < before {
                    // Lowering: the level reached is worked out again from nothing (it only ever rises otherwise).
                    stats.max_achieved_level = 0;
                    stats.next_level_offset = 0;
                }
                stats.money_earned = xp;
                let (level, _) = update_rider_level(stats, &thresholds, &prices);
                info!("scripts: {package} set rider {rider:#010x}'s XP {before} -> {xp} (level {})", level + 1);
            }
            ProfileOrder::RiderLevel { package, rider, level } => {
                let (thresholds, prices) = (fe.rules.level_thresholds.clone(), fe.rules.level_prices.clone());
                let stats = fe.profile.riders.entry(rider).or_default();
                let reached = set_rider_level(stats, level - 1, &thresholds, &prices) + 1;
                info!("scripts: {package} set rider {rider:#010x} to level {reached} (asked {level})");
            }
            ProfileOrder::ShopMinRarity { package, rarity } => {
                if fe.profile.shop_min_rarity != rarity {
                    fe.profile.shop_min_rarity = rarity.clamp(0, 4);
                    info!("scripts: {package} set the shops' lowest rarity to {rarity}");
                }
            }
            ProfileOrder::MaxLevel { package, level } => {
                let game = fe.rules.level_thresholds.len() as i64;
                let cap = if level > game { level } else { 0 };
                set_level_cap(cap);
                info!("scripts: {package} set the highest rider level to {}", cap.max(game));
            }
            ProfileOrder::UnlimitedGear { package, mask } => {
                let mask = u32::try_from(mask).unwrap_or(0) & ssx_assets::career::profile::unlimited_gear::ALL;
                if fe.profile.unlimited_gear != mask {
                    fe.profile.unlimited_gear = mask;
                    info!("scripts: {package} set the unlimited gear to {mask:#04x}");
                }
            }
            ProfileOrder::InfiniteCredits { package, on } => {
                if fe.profile.infinite_credits != on {
                    fe.profile.set_infinite_credits(on);
                    values.set_int(BANK_ADJUST, 0);
                    values.set_int(BANK, fe.profile.credits().clamp(0, i64::from(i32::MAX)) as i32);
                    info!("scripts: {package} turned infinite credits {}", if on { "on" } else { "off" });
                }
            }
        }
    }
    fe.publish.extend(["dp_user_data", "dp_playable_characters"].map(String::from));
    fe.requests.push(GameRequest::Save);
}
