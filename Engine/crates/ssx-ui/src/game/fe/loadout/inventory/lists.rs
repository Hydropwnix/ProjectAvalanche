use super::*;

pub(super) const PORTRAIT_LOCKED: &str = "assets/riderPortraitsLarge/locked/";
pub(super) const PORTRAIT_JACKET: &str = "assets/riderPortraitsLarge/jacket/";
pub(super) const PORTRAIT_PANTS: &str = "assets/riderPortraitsLarge/pants/";
pub(super) const ICON_NONE: &str = "assets/equipmentIcons/none.dds";
pub(super) const ICON_MISSING: &str = "assets/equipmentIcons/missingIcon.dds";
pub(super) const MYSTERY_ICON: &str = "assets/equipmentIcons/mystery.dds";

#[derive(Clone, Copy)]
pub(super) struct ListInputs<'a> {
    pub db: &'a CareerDb,
    pub profile: &'a Profile,
    pub drop: Option<&'a DropInfo>,
    pub ctx: &'a StoreContext,
}

pub fn level_purchase_description(loc: &mut Localization, rows: &[[u8; 5]], level: i64) -> String {
    use crate::game::loc::{FormatArg, format_positional};
    let Some((from, to)) = usize::try_from(level).ok().and_then(|l| Some((rows.get(l)?, rows.get(l.checked_add(1)?)?)))
    else {
        return String::new();
    };
    let mut texts = Vec::new();
    for (column, (&a, &b)) in to.iter().zip(from.iter()).enumerate() {
        let growth = i64::from(a) - i64::from(b);
        if growth <= 0 {
            continue;
        }
        let key = match column {
            2 => "TXT_EQUIP_BUY_LVL_GEAR",
            3 => "TXT_EQUIP_BUY_LVL_BOOST",
            4 => "TXT_EQUIP_BUY_LVL_GEOTAG",
            _ => "",
        };
        let text = if key.is_empty() { String::new() } else { loc.text(key) };
        texts.push(format_positional(&text, &[FormatArg::Int(growth)]));
    }
    // PORT(choice): four or five growing columns (no disc or tuner row pair has more than two) give an empty
    // description, as 0x8248C580's count check (`< 4`) leaves it; the executable formats every growing column's text
    // into a three-string array on its stack first, so a fourth or fifth one is written past the array, over the
    // frame's saved registers — a tuner or package row pair is a save's or a mod's data, and the port does not.
    if texts.len() > 3 {
        return String::new();
    }
    let shown = FormatArg::Int(level.saturating_add(2));
    let mut args = vec![shown];
    args.extend(texts.iter().map(|t| FormatArg::Str(t)));
    let text = loc.text(&format!("TXT_EQUIP_BUY_LVL_{}", texts.len()));
    format_positional(&text, &args)
}

impl Inventory {
    pub fn ui_item(
        &self,
        db: &CareerDb,
        profile: &Profile,
        loc: &mut Localization,
        drop: Option<&DropInfo>,
        rec: &Record,
    ) -> UiItem {
        let empty = Self::is_empty_item(rec.kind, rec.key, rec.src);
        let mut u = UiItem { key: rec.key, src: rec.src, slot: rec.slot, rarity: rec.rarity, ..Default::default() };
        if rec.mystery {
            u.name = loc.text("TXT_MYSTERY_ITEM_NAME");
            u.description = loc.text("TXT_MYSTERY_ITEM_DESC");
            u.asset = MYSTERY_ICON.into();
            u.flags |= flag::MYSTERY;
        } else {
            u.name = rec.name.clone();
            u.description = self.perk_description(loc, rec);
            u.perk = if rec.kind == 4 { -1 } else { (3..6).rev().find(|&i| rec.stats[i].0 != -1).map_or(-1, |i| rec.stats[i].0) };
            let icon = |k: u32| db.descriptions.iter().find(|d| d.key == k).map(|d| d.icon.clone()).unwrap_or_default();
            u.asset = icon(rec.descriptions[0]);
            u.secondary = icon(rec.descriptions[1]);
            // A port fix: a suit made in the store with its rider's first pants (its pants description none the career's
            // database knows: the suit's name counts it as pants 0) showed no pants in its picture: their picture is
            // the rider's first pants' (the jacket's, `<rider>_top_<n>`, beside `<rider>_bottom_0`).
            if rec.kind == 0
                && u.secondary.is_empty()
                && let Some((stem, _)) = u.asset.rsplit_once("_top_")
            {
                u.secondary = format!("{stem}_bottom_0.dds");
            }
            let package_portrait = db
                .riders
                .iter()
                .find(|r| r.key == rec.descriptions[0])
                .and_then(|r| r.content.as_ref())
                .and_then(|c| c.portrait.as_ref())
                .filter(|p| p.namespace() != ssx_content::BASE_NAMESPACE)
                .and_then(ssx_assets::content::textures::ui_path_of);
            // PORT(choice): a package's rider shows its one portrait (a package has no per-suit pictures, and no record
            // field names them yet); the disc composes the locked picture or the rider in its last suit (jacket + pants
            // pictures by the suit's look ids, 0x8248D2D0…0x8248D5D0).
            if let Some(path) = package_portrait {
                u.asset = path;
                u.secondary.clear();
            } else if let Some(r) = db.riders.iter().find(|r| r.key == rec.descriptions[0]) {
                if !rec.owned {
                    u.asset = format!("{PORTRAIT_LOCKED}{}.dds", r.portrait);
                } else {
                    let descs = match self.rider_suit(db, profile, r.key) {
                        Some((suit, 0)) => {
                            db.items.iter().find(|i| i.key == suit).map(|i| i.descriptions.clone()).unwrap_or_default()
                        }
                        Some((suit, _)) => {
                            profile.generated.get(&suit).map(|g| g.descriptions.to_vec()).unwrap_or_default()
                        }
                        None => Vec::new(),
                    };
                    let id = |k: Option<&u32>| {
                        k.and_then(|k| db.descriptions.iter().find(|d| d.key == *k)).map_or(0, |d| d.id)
                    };
                    u.asset = format!("{PORTRAIT_JACKET}{}_top{}.dds", r.portrait, id(descs.first()));
                    u.secondary = format!("{PORTRAIT_PANTS}{}_bottom{}.dds", r.portrait, id(descs.get(1)));
                }
            }
            if empty {
                u.asset = ICON_NONE.into();
                u.secondary.clear();
            } else if u.asset.is_empty() {
                u.asset = ICON_MISSING.into();
                u.secondary.clear();
            }
        }
        u.cost = if rec.owned {
            ssx_assets::career::store::purchase::sell_price(rec.cost, self.data.tables.sell_percent)
        } else {
            rec.cost
        };
        u.sort = self.sort_value(db, profile, drop, rec);
        u.quality = self.record_score(rec);
        for (k, &(t, v)) in rec.stats[..3].iter().enumerate() {
            u.stats[k] = (t, i64::from(v));
        }
        let kind_ok = (0..KINDS as i64).contains(&rec.kind);
        let equipped = kind_ok && self.equipped[rec.kind as usize].contains(&(rec.key, rec.src));
        if rec.recommended {
            u.flags |= flag::RECOMMENDED;
        }
        let boost_blocked =
            drop.is_some_and(|d| rec.kind == 3 && !d.survive() && rec.stats[..3].iter().any(|s| s.0 == 0x20));
        let masked = rec.slot >= 0 && self.gear_mask & (1u64 << (rec.slot & 0x3F)) as u32 != 0;
        let level_ok = rec.kind == RIDER || self.level_limit == -1 || self.level_limit >= rec.level;
        let available = empty || (rec.owned && !masked && level_ok);
        if available && !boost_blocked {
            u.flags |= flag::AVAILABLE;
        }
        if rec.cost > 0 {
            if !rec.owned {
                u.flags |= flag::PURCHASEABLE;
            } else if !equipped {
                u.flags |= flag::SELLABLE;
            }
        }
        if rec.kind != 4 && rec.kind != 0 && !rec.mystery && !empty {
            u.flags |= flag::SHOW_STATS;
        }
        if rec.special {
            u.flags |= flag::SPECIAL;
        }
        if kind_ok && self.new_items[rec.kind as usize].contains(&(rec.key, rec.src)) {
            u.flags |= flag::NEW;
        }
        if empty {
            u.flags |= flag::EMPTY;
        }
        if rec.kind != 0 {
            u.flags |= flag::SHOW_LEVEL;
        }
        if rec.kind == RIDER {
            let (level, percent) = Self::rider_level(db, profile, rec.key);
            u.level = level + 1;
            u.level_progress = percent;
            if rec.owned {
                let stats = profile.riders.get(&rec.key).cloned().unwrap_or_default();
                if let Some((_, price)) = ssx_assets::career::profile::level_purchase_tables(
                    &stats,
                    &db.progression.level_thresholds,
                    &db.progression.level_prices,
                ) {
                    u.cost = price;
                    u.flags |= flag::LEVEL_PURCHASEABLE;
                    u.description = level_purchase_description(loc, &db.progression.level_slots, level);
                    return u;
                }
            }
            u.description =
                db.riders.iter().find(|r| r.key == rec.key).map(|r| loc.text(&r.description)).unwrap_or_default();
        } else {
            u.level = rec.level + 1;
        }
        u
    }

    /// A suit's full portrait on a rider (the jacket and pants pictures its card shows): none for a package's rider
    /// (one picture of its own).
    pub fn suit_portrait(&self, db: &CareerDb, profile: &Profile, rider: u32, suit: (u32, u8)) -> Option<(String, String)> {
        let r = db.riders.iter().find(|r| r.key == rider)?;
        let package = r.content.as_ref().and_then(|c| c.portrait.as_ref()).is_some_and(|p| p.namespace() != ssx_content::BASE_NAMESPACE);
        if package {
            return None;
        }
        let descs = match suit.1 {
            0 => db.items.iter().find(|i| i.key == suit.0).map(|i| i.descriptions.clone()).unwrap_or_default(),
            _ => profile.generated.get(&suit.0).map(|g| g.descriptions.to_vec()).unwrap_or_default(),
        };
        let id = |k: Option<&u32>| k.and_then(|k| db.descriptions.iter().find(|d| d.key == *k)).map_or(0, |d| d.id);
        Some((
            format!("{PORTRAIT_JACKET}{}_top{}.dds", r.portrait, id(descs.first())),
            format!("{PORTRAIT_PANTS}{}_bottom{}.dds", r.portrait, id(descs.get(1))),
        ))
    }

    pub(super) fn perk_description(&self, loc: &mut Localization, rec: &Record) -> String {
        if rec.kind == 4 {
            return String::new();
        }
        let Some(i) = (3..6).rev().find(|&i| rec.stats[i].0 != -1) else { return String::new() };
        let (t, v) = rec.stats[i];
        loc.format(&format!("TXT_PERK_DESC_{t}"), &[&v.to_string()])
    }
}

impl Inventory {
    pub fn fixed_shown(&self, db: &CareerDb, profile: &Profile, drop: Option<&DropInfo>, kind: i64) -> Vec<u32> {
        let owned = |k: u32| self.catalog.entries.get(&k).is_some_and(|e| profile.offered(k, e));
        let forced = drop.and_then(|d| d.forced(kind));
        if kind == RIDER {
            let riders: Vec<&ssx_assets::career::data::Rider> = db
                .riders
                .iter()
                .filter(|r| r.default || owned(r.key))
                .filter(|r| forced.is_none_or(|f| f == r.key))
                .filter(|r| !drop.is_some_and(|d| d.rider_forced) || drop.and_then(|d| d.forced(RIDER)) == Some(r.key))
                .collect();
            if !ssx_core::rider_showcase::enabled() {
                return riders.iter().map(|r| r.key).collect();
            }
            // (the Custom Event Menus mod: the riders in a set order, the retro riders (`dlc…`) after the rest; any
            // others (a package's) after them, as the career lists them)
            const ORDER: [&str; 19] = [
                "zoe", "alex", "elise", "kaori", "tane", "moby", "mac", "psymon", "tyler", "griff", "eddie", "travis",
                "dlczoe", "dlcelise", "dlckaori", "dlcmoby", "dlcmac", "dlcpsymon", "dlceddie",
            ];
            let place = |r: &ssx_assets::career::data::Rider| {
                let (portrait, name) = (r.portrait.to_ascii_lowercase(), r.name.to_ascii_lowercase());
                let name = if name == "ty" { "tyler".to_string() } else { name };
                ORDER.iter().position(|&o| o == portrait || o == name).unwrap_or(ORDER.len())
            };
            let mut riders = riders;
            riders.sort_by_key(|r| place(r));
            return riders.iter().map(|r| r.key).collect();
        }
        let level = Self::rider_level(db, profile, self.rider).0;
        // (the Custom Event Menus mod: a retro rider (`dlc…`) has the default board of the rider it's a retro version
        // of)
        let retro_board = ssx_core::rider_showcase::enabled()
            .then(|| db.riders.iter().find(|r| r.key == self.rider))
            .flatten()
            .and_then(|r| {
                let (name, portrait) = (r.name.to_lowercase(), r.portrait.to_lowercase());
                let base = name.strip_prefix("dlc").or_else(|| portrait.strip_prefix("dlc"))?.to_string();
                Some(fnv1(format!("board_{base}_default").as_bytes()))
            });
        db.items
            .iter()
            .filter(|i| i.kind == kind)
            .filter(|i| {
                let own = owned(i.key);
                let mut shown = i.default || own;
                if i.rider != 0 && self.rider != 0 && retro_board != Some(i.key) {
                    shown &= i.rider == self.rider;
                }
                if !own {
                    shown &= level >= i.level;
                }
                match forced {
                    Some(f) => f == i.key,
                    None => shown,
                }
            })
            .map(|i| i.key)
            .collect()
    }

    pub fn generated_shown(&self, db: &CareerDb, profile: &Profile, kind: i64, ctx: &StoreContext) -> Vec<u32> {
        let _ = db;
        let mut out: Vec<u32> = profile
            .generated
            .values()
            .filter(|g| g.kind == kind && (g.charlock == 0 || g.charlock == self.rider))
            .map(|g| g.equip_id)
            .collect();
        if (0..KINDS as i64).contains(&kind) {
            out.extend(self.store.offers(&self.data, ctx, kind as usize).iter().map(|o| o.id));
        }
        out
    }

    pub fn build_lists(
        &mut self,
        db: &CareerDb,
        profile: &Profile,
        loc: &mut Localization,
        drop: Option<&DropInfo>,
        ctx: &StoreContext,
        kind: i64,
    ) {
        self.category_lists(ListInputs { db, profile, drop, ctx }, loc, kind, false);
    }

    pub fn refresh_lists(
        &mut self,
        db: &CareerDb,
        profile: &Profile,
        loc: &mut Localization,
        drop: Option<&DropInfo>,
        ctx: &StoreContext,
        kind: i64,
    ) {
        self.category_lists(ListInputs { db, profile, drop, ctx }, loc, kind, kind != RIDER);
    }

    pub fn put_bought_first(
        &mut self,
        db: &CareerDb,
        profile: &Profile,
        loc: &mut Localization,
        drop: Option<&DropInfo>,
        kind: i64,
        bought: (u32, u8),
    ) {
        if !(0..KINDS as i64).contains(&kind) {
            return;
        }
        let Some(rec) = self.record(db, profile, loc, bought.0, bought.1) else { return };
        let bit = |t: i64| if (0..32).contains(&t) { 1u32 << t } else { 0 };
        if (self.worn_gear | self.gear_mask) & bit(rec.slot) != 0 {
            return;
        }
        let item = self.ui_item(db, profile, loc, drop, &rec);
        self.owned_list.insert(0, item);
    }

    fn category_lists(&mut self, s: ListInputs, loc: &mut Localization, kind: i64, refresh: bool) {
        let ListInputs { db, profile, drop, ctx } = s;
        if !refresh {
            self.owned_list.clear();
            self.store_list.clear();
        }
        self.inventory_list.clear();
        self.store.clear_shown_special();
        if !refresh && let Some(k) = usize::try_from(kind).ok().filter(|&k| k < KINDS) {
            self.store.drop_expired_specials(k, self.now_ms);
        }
        let mut keys: Vec<(u32, u8)> = self.fixed_shown(db, profile, drop, kind).into_iter().map(|k| (k, 0)).collect();
        if kind != RIDER {
            keys.extend(self.generated_shown(db, profile, kind, ctx).into_iter().map(|k| (k, 1)));
        }
        let bit = |t: i64| if (0..32).contains(&t) { 1u32 << t } else { 0 };
        for (k, src) in keys {
            let Some(rec) = self.record(db, profile, loc, k, src) else { continue };
            if Self::is_empty_item(rec.kind, rec.key, rec.src) {
                continue;
            }
            let item = self.ui_item(db, profile, loc, drop, &rec);
            let worn = (self.worn_gear | self.gear_mask) & bit(rec.slot) != 0;
            let over_level = rec.kind != RIDER && self.level_limit != -1 && self.level_limit < rec.level;
            let equipped =
                (0..KINDS as i64).contains(&kind) && self.equipped[kind as usize].contains(&(rec.key, rec.src));
            if !refresh && !worn && !over_level {
                if !rec.owned {
                    let gear_ok = rec.kind != 2 || (ctx.gear_types | ctx.required_gear) & bit(rec.slot) != 0;
                    let boost_ok = rec.kind != 3 || item.sort >= 1;
                    if kind == RIDER {
                        self.owned_list.push(item.clone());
                    } else if gear_ok && boost_ok && item.rarity >= profile.shop_min_rarity {
                        self.store_list.push(item.clone());
                    }
                } else if !equipped || kind == RIDER {
                    self.owned_list.push(item.clone());
                }
            }
            if rec.owned && kind != RIDER {
                let mut it = item;
                if equipped || worn {
                    it.flags &= !flag::AVAILABLE;
                }
                self.inventory_list.push(it);
            }
        }
        if kind != RIDER {
            let mut owned = std::mem::take(&mut self.owned_list);
            if refresh {
                self.reread(s, loc, kind, &mut owned, true);
            } else {
                self.order_list(db, profile, loc, drop, kind, &mut owned);
            }
            self.owned_list = owned;
            self.inventory_list.sort_by(super::order::by_value);
            // (the Custom Event Menus mod: the suits' and boards' full lists grouped by their perks, each group best
            // first, those with no perk last)
            if matches!(kind, 0 | 1) && ssx_core::rider_showcase::enabled() {
                let group = |u: &UiItem| if u.perk < 0 { i64::MAX } else { u.perk };
                self.inventory_list
                    .sort_by(|a, b| group(a).cmp(&group(b)).then_with(|| super::order::by_value(a, b)));
            }
            // (the Custom Event Menus mod: favourite suits and boards first)
            if matches!(kind, 0 | 1) && ssx_core::rider_showcase::enabled() {
                let later = |u: &UiItem| !super::favorites::is_favourite(u);
                self.owned_list.sort_by_key(later);
                self.inventory_list.sort_by_key(later);
            }
            let with_empty = matches!(kind, 2..=4);
            self.owned_list.truncate(if with_empty { 6 } else { 7 });
            if with_empty && let Some(rec) = self.record(db, profile, loc, fnv1(EMPTY[kind as usize].as_bytes()), 0) {
                let e = self.ui_item(db, profile, loc, drop, &rec);
                self.owned_list.push(e.clone());
                self.inventory_list.push(e);
            }
            let mut store = std::mem::take(&mut self.store_list);
            if refresh {
                self.reread(s, loc, kind, &mut store, false);
            } else {
                self.order_list(db, profile, loc, drop, kind, &mut store);
            }
            self.store_list = store;
            self.build_store_front(s, loc, kind, refresh);
            if let Some(k) = usize::try_from(kind).ok().filter(|&k| k < KINDS)
                && let Some(front) = self.store_list.first().filter(|u| u.has(flag::SPECIAL))
            {
                self.store.show_special(k, front.key);
            }
            self.store_list.truncate(4);
            let equipped = self.equipped[kind as usize].clone();
            for it in &mut self.inventory_list {
                if equipped.contains(&(it.key, it.src)) {
                    it.flags |= flag::EQUIPPED;
                }
            }
        } else if !ssx_core::rider_showcase::enabled() {
            // (the Custom Event Menus mod: the riders kept in its order)
            self.owned_list.sort_by(super::order::by_value);
        }
        self.survival = self.survival_score(db, profile, drop);
        self.inventory_size = Self::inventory_size(db, profile);
    }

    fn reread(&self, s: ListInputs, loc: &mut Localization, kind: i64, list: &mut Vec<UiItem>, owned: bool) {
        let ListInputs { db, profile, drop, .. } = s;
        let old = std::mem::take(list);
        for u in old {
            let Some(rec) = self.record(db, profile, loc, u.key, u.src) else { continue };
            let keep = if owned { rec.owned && !Self::is_empty_item(kind, rec.key, rec.src) } else { !rec.owned };
            if keep {
                list.push(self.ui_item(db, profile, loc, drop, &rec));
            }
        }
    }

    pub fn seen(&mut self, kind: i64, entry: (u32, u8)) {
        if let Some(l) = usize::try_from(kind).ok().and_then(|k| self.new_items.get_mut(k)) {
            l.retain(|&e| e != entry);
        }
        if !(0..KINDS as i64).contains(&kind) {
            return;
        }
        for list in [&mut self.owned_list, &mut self.inventory_list] {
            if let Some(u) = list.iter_mut().find(|u| (u.key, u.src) == entry) {
                u.flags &= !flag::NEW;
            }
        }
    }

    pub fn inventory_size(db: &CareerDb, profile: &Profile) -> i32 {
        let rows = |n: usize| i64::from(u16::try_from(n % 0x1_0000).unwrap_or(0));
        let riders = i64::try_from(Self::rider_count(db, profile)).unwrap_or(i64::MAX);
        let n = rows(profile.owned_items.len()) - riders + rows(profile.generated.len());
        i32::try_from(n.max(0)).unwrap_or(i32::MAX)
    }

    pub fn rider_count(db: &CareerDb, profile: &Profile) -> usize {
        db.riders.iter().filter(|r| !r.locked || profile.owned_items.contains(&r.key)).count()
    }
}
