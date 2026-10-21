use super::*;

pub struct GenLook<'a> {
    pub kind: i64,
    pub slot: i64,
    pub level: i64,
    pub rarity: i64,
    pub charlock: u32,
    pub descriptions: [u32; 2],
    pub stats: &'a [(i64, u8); 6],
    pub name_format: [i64; 4],
}

impl<'a> GenLook<'a> {
    pub fn of_offer(o: &'a StoreItem) -> Self {
        GenLook {
            kind: o.kind,
            slot: o.slot,
            level: o.level,
            rarity: o.rarity,
            charlock: o.charlock,
            descriptions: o.descriptions,
            stats: &o.stats,
            name_format: o.name_format,
        }
    }

    pub fn of_row(r: &'a ssx_assets::career::profile::GeneratedItem) -> Self {
        GenLook {
            kind: r.kind,
            slot: r.slot,
            level: r.level,
            rarity: r.rarity,
            charlock: r.charlock,
            descriptions: r.descriptions,
            stats: &r.stats,
            name_format: r.name_format,
        }
    }
}

pub fn dominant_stat(stats: &[(i64, u8); 6], level: i64) -> i64 {
    let n = stats.iter().take_while(|s| s.0 != -1).count();
    if n == 0 {
        return -1;
    }
    let mut first = 0;
    for i in 1..n {
        if stats[first].1 < stats[i].1 {
            first = i;
        }
    }
    if n == 1 {
        return stats[first].0;
    }
    let mut second = usize::from(first == 0);
    for i in 1..n {
        if i != first && stats[second].1 < stats[i].1 {
            second = i;
        }
    }
    let margin = if level > 2 { 6 } else { 3 };
    if u32::from(stats[first].1) <= u32::from(stats[second].1) + margin { -1 } else { stats[first].0 }
}

pub(super) fn stat_gear(stat: i64) -> Option<i64> {
    ssx_assets::career::store::stat_gear_type(stat)
}

pub fn generated_name(db: &CareerDb, loc: &mut Localization, g: &GenLook) -> String {
    let [format, descriptor, stat_id, _equip_id] = g.name_format;
    let rareness = loc.text(&format!("TXT_GEN_RARENESS{}_{descriptor}", g.rarity));
    let dom = dominant_stat(g.stats, g.level);
    let stat = if dom == -1 {
        loc.text(&format!("TXT_GEN_MULTI_STAT{stat_id}"))
    } else {
        loc.text(&format!("TXT_GEN_STAT{dom}_{stat_id}"))
    };
    let equip_key = match g.kind {
        1 => format!("TXT_GEN_EQUIP_BOARD{stat_id}"),
        0 | 2 => format!("TXT_GEN_EQUIP_GEAR{}_{stat_id}", g.slot),
        3 => format!("TXT_GEN_EQUIP_BOOST{stat_id}"),
        4 => format!("TXT_GEN_EQUIP_GEOTAG{stat_id}"),
        _ => format!("TXT_GEN_EQUIP_BOARD{stat_id}"),
    };
    let equip = loc.text(&equip_key);
    let no_desc = g.rarity < 2;
    match g.kind {
        0 => {
            let rider = db.riders.iter().find(|r| r.key == g.charlock).map(|r| loc.text(&r.title)).unwrap_or_default();
            let character = loc.format("TXT_SUIT_CHARACTER", &[&rider]);
            let id = |k: u32| db.descriptions.iter().find(|d| d.key == k).map_or(0, |d| d.id);
            let version = (id(g.descriptions[0]) & 0xFFF_FFFF) * 16 + id(g.descriptions[1]) + 1;
            let version = loc.format("TXT_SUIT_VERSION", &[&version.to_string()]);
            let mut key = format!("TXT_GEN_NAME_FORMAT_SUIT{format}");
            if g.rarity <= 2 {
                key.push_str("_NO_DESC");
            }
            if g.stats[0].0 == -1 {
                key.push_str("_NO_STAT");
            }
            loc.format(&key, &[&character, &rareness, &stat, &equip, &version])
        }
        1 | 2 => {
            let key = format!("TXT_GEN_NAME_FORMAT{format}{}", if no_desc { "_NO_DESC" } else { "" });
            loc.format(&key, &[&rareness, &stat, &equip])
        }
        3 => {
            let gear = g.stats.iter().find_map(|s| stat_gear(s.0));
            let kind_word = gear.map(|t| loc.text(&format!("TXT_GEN_EQUIP_GEAR{t}_0"))).unwrap_or_default();
            let mut key = format!("TXT_GEN_NAME_FORMAT_BOOST{format}");
            if no_desc {
                key.push_str("_NO_DESC");
            }
            if gear.is_none() {
                key.push_str("_NO_TYPE");
            }
            loc.format(&key, &[&rareness, &stat, &kind_word, &equip])
        }
        _ => {
            let key = format!("TXT_GEN_NAME_FORMAT_NO_STAT{format}{}", if no_desc { "_NO_DESC" } else { "" });
            loc.format(&key, &[&rareness, &equip])
        }
    }
}

impl Inventory {
    pub fn generated_record(
        &self,
        db: &CareerDb,
        profile: &Profile,
        loc: &mut Localization,
        key: u32,
    ) -> Option<Record> {
        let key = match self.store.last_bought {
            Some((stock, equip_id)) if stock == key => equip_id,
            _ => key,
        };
        if key >= 1_000_000 {
            let o = self.store.stock.iter().flatten().find(|o| o.id == key)?;
            return Some(Record {
                owned: false,
                special: o.special,
                mystery: o.mystery,
                cost: o.cost,
                level: o.level,
                rarity: o.rarity,
                rider: o.charlock,
                descriptions: o.descriptions,
                key,
                src: 1,
                kind: o.kind,
                name: generated_name(db, loc, &GenLook::of_offer(o)),
                slot: o.slot,
                stats: o.stats,
                ..Default::default()
            });
        }
        let r = profile.generated.get(&key)?;
        Some(Record {
            owned: true,
            cost: r.cost,
            level: r.level,
            rarity: r.rarity,
            rider: r.charlock,
            descriptions: r.descriptions,
            key,
            src: 1,
            kind: r.kind,
            name: generated_name(db, loc, &GenLook::of_row(r)),
            slot: r.slot,
            stats: r.stats,
            ..Default::default()
        })
    }

    pub fn record(
        &self,
        db: &CareerDb,
        profile: &Profile,
        loc: &mut Localization,
        key: u32,
        src: u8,
    ) -> Option<Record> {
        if src == 1 { self.generated_record(db, profile, loc, key) } else { self.fixed_record(db, profile, loc, key) }
    }
}
