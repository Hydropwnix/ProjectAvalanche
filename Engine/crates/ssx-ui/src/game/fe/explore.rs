use ssx_assets::career::DropType;

use super::FrontEnd;

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
