//! A port addition: the menus' 3-D rider showcase (the Custom Rider Select mod). On the equipment screen's rider
//! step, the riders the list shows, which one is highlighted, and what each would wear and carry in a run: its
//! last suit and gear, as the run's own setup would commit them (`setup::commit` for that rider).

use super::{Equipment, state};
use crate::game::fe::inventory::{self, DropInfo, Inventory, flag};
use crate::game::fe::{FrontEnd, screens, setup};
use crate::game::loc::Localization;

/// The gear stats of the suit perks that make a rider's head or hands big.
const STAT_BIG_HEAD: i8 = 34;
const STAT_BIG_HANDS: i8 = 35;
const STAT_BIG_HEAD_AND_HANDS: i8 = 36;

/// One rider of the list.
#[derive(Clone, Debug, PartialEq)]
pub struct ShowcaseRider {
    /// The rider's career key (the list item's key).
    pub key: u32,
    /// The character's name (`mac`, `zoe`...): the rider to load.
    pub name: String,
    /// Its portrait pictures' name (`mac` for `mac_top5.dds`): how its card is found on the screen.
    pub portrait: String,
    /// Locked (not bought yet): drawn as a silhouette.
    pub locked: bool,
    /// What it wears and carries: (gear type, equipment description key), as a run's setup items.
    pub items: Vec<(usize, u32)>,
    /// The head lamp's two stats (its size), when it has one.
    pub lamp: [Option<u8>; 2],
    /// Diagnostic: the board the equipment screen has for this rider (name, keys, its descriptions).
    pub note: String,
    /// Its suit's name (as the equipment screen shows it), and whether that's a glowing suit (its name says
    /// "Glowing Suit", as "Mac's Legendary Glowing Suit (#95)"): only a glowing suit's glowing parts glow.
    pub suit: String,
    pub glowing: bool,
    /// Its suit's perk makes its head big, its hands big (the BigHead, BigHands and BigHeadAndHands perks).
    pub big_head: bool,
    pub big_hands: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Showcase {
    pub riders: Vec<ShowcaseRider>,
    pub highlighted: usize,
}

/// Whether the equipment screen is up on its rider step (the showcase shows).
pub fn active(fe: &FrontEnd) -> bool {
    fe.screen == screens::id::EQUIPMENT && fe.equipment.state == state::RIDER && fe.equipment.category == 0
}

/// Whether the equipment screen is past its rider step, on its gear steps (the chosen rider shown in 3-D in a view of
/// its own).
pub fn gear_view(fe: &FrontEnd) -> bool {
    (fe.screen == screens::id::EQUIPMENT
        && matches!(fe.equipment.state, state::GEAR | state::INVENTORY | state::READY | state::READY_CATEGORY))
        // (the full list (View More) too: the rider beside it)
        || (fe.screen == 0x31 && fe.equipment.state == state::INVENTORY)
}

/// The chosen rider (the one the gear steps dress), dressed as now (changes not saved yet included).
pub fn chosen(fe: &FrontEnd, loc: &mut Localization) -> Option<ShowcaseRider> {
    let inv = fe.equipment.inv.as_ref()?;
    // (the 3D Rider Select mod: a hovered suit or board, worn meanwhile in its place)
    if let Some((kind, slot, item)) = fe.equipment.preview {
        return dressed(fe, loc, inv, kind, slot, item);
    }
    build(fe, loc, inv, &[(inv.rider, false)]).into_iter().next()
}

/// The chosen rider with this suit or board (its kind, slot and item) in place of its own.
fn dressed(
    fe: &FrontEnd,
    loc: &mut Localization,
    inv: &Inventory,
    kind: usize,
    slot: usize,
    item: (u32, u8),
) -> Option<ShowcaseRider> {
    let mut one: Inventory = inv.clone();
    if let Some(l) = one.equipped.get_mut(kind) {
        if l.is_empty() {
            l.push(item);
        } else {
            let at = slot.min(l.len() - 1);
            l[at] = item;
        }
    }
    build(fe, loc, &one, &[(one.rider, false)]).into_iter().next()
}

/// What the previews loaded ahead depend on (cheap: to notice they changed): the list shown and the hovered item.
pub fn previews_key(fe: &FrontEnd) -> String {
    let eq = &fe.equipment;
    let lens = eq.inv.as_ref().map(|i| (i.owned_list.len(), i.store_list.len(), i.inventory_list.len()));
    format!("{} {} {} {:?} {:?} {}", eq.category, eq.state, eq.slot, eq.hovered, lens, eq.inv_first)
}

/// The 3D Rider Select mod: the chosen rider in each suit or board the gear list shows (the hovered one first, then
/// the others; in the full list, those near the highlight), to load ahead so hovering one shows it at once.
pub fn previews(fe: &FrontEnd, loc: &mut Localization) -> Vec<ShowcaseRider> {
    let eq = &fe.equipment;
    if !super::preview_kinds(eq.category) || !matches!(eq.state, state::GEAR | state::INVENTORY) {
        return Vec::new();
    }
    let Some(inv) = eq.inv.as_ref() else { return Vec::new() };
    let Ok(kind) = usize::try_from(super::kind_of(eq.category)) else { return Vec::new() };
    let slot = usize::try_from(eq.slot).unwrap_or(0);
    // (the items, nearest the hovered one first)
    let near = |list: &[inventory::UiItem], at: usize, reach: usize| -> Vec<(u32, u8)> {
        let mut order: Vec<usize> = (0..list.len()).filter(|&i| i.abs_diff(at) <= reach).collect();
        order.sort_by_key(|&i| i.abs_diff(at));
        order
            .into_iter()
            .filter_map(|i| list.get(i))
            .filter(|u| !u.has(flag::EMPTY) && !u.has(flag::MYSTERY))
            .map(|u| (u.key, u.src))
            .collect()
    };
    let hovered = |l: super::List| eq.hovered.filter(|h| h.0 == l).map(|h| h.1);
    let items: Vec<(u32, u8)> = if eq.state == state::INVENTORY {
        let at = hovered(super::List::Inventory).unwrap_or_else(|| usize::try_from(eq.inv_first).unwrap_or(0));
        near(&inv.inventory_list, at, 6)
    } else {
        let (mut owned, mut store) = (
            near(&inv.owned_list, hovered(super::List::Items).unwrap_or(0), usize::MAX),
            near(&inv.store_list, hovered(super::List::Store).unwrap_or(0), usize::MAX),
        );
        if hovered(super::List::Store).is_some() {
            std::mem::swap(&mut owned, &mut store);
        }
        owned.into_iter().chain(store).collect()
    };
    let mut out: Vec<ShowcaseRider> = Vec::new();
    for item in items {
        if let Some(r) = dressed(fe, loc, inv, kind, slot, item)
            && !out.contains(&r)
        {
            out.push(r);
        }
    }
    out
}

/// The highlighted rider's index in the list.
pub fn highlighted(eq: &Equipment) -> usize {
    eq.highlight.unwrap_or(usize::try_from(eq.rider_index).unwrap_or(0))
}

/// The keys of the riders the list shows, in order (cheap: to notice a changed list).
pub fn keys(fe: &FrontEnd) -> Vec<u32> {
    // (and the event: each kind of event dresses the riders in its own loadout; and what the chosen rider wears: the
    // screen dresses it for a new event a step after the event changes)
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    if let Some(inv) = fe.equipment.inv.as_ref() {
        (inv.rider, &inv.event_type, &inv.equipped).hash(&mut h);
    }
    let worn = h.finish();
    fe.equipment
        .list
        .iter()
        .map(|u| u.key)
        .chain([fe.equipment.drop, worn as u32, (worn >> 32) as u32])
        .collect()
}

/// The showcase's riders, each dressed as its run would be. None off the rider step.
pub fn showcase(fe: &FrontEnd, loc: &mut Localization) -> Option<Showcase> {
    if !active(fe) {
        return None;
    }
    let inv = fe.equipment.inv.as_ref()?;
    let list: Vec<(u32, bool)> = fe.equipment.list.iter().map(|u| (u.key, !u.has(flag::AVAILABLE))).collect();
    let riders = build(fe, loc, inv, &list);
    let highlighted = highlighted(&fe.equipment).min(riders.len().saturating_sub(1));
    Some(Showcase { riders, highlighted })
}

/// The riders the rider step's list will show, dressed as there, worked out anywhere in the menus (before the
/// equipment screen has opened): so their 3-D models can load ahead of it. The list as the screen builds it (a fresh
/// inventory on the profile's last rider, its rider list); each rider in its last suit and gear.
/// The rider last played (the rider step opens on them).
pub fn last_played(fe: &FrontEnd) -> u32 {
    inventory::event_loadouts::last_rider().filter(|k| fe.career.riders.iter().any(|r| r.key == *k)).unwrap_or_else(
        || fe.profile.attribs.get(&ssx_assets::career::profile::ATTRIB_LAST_RIDER).map_or(0, |&v| v as u32),
    )
}

/// The rider the equipment screen has chosen (on the rider step, the one it opened on).
pub fn chosen_rider(fe: &FrontEnd) -> u32 {
    fe.equipment.inv.as_ref().map_or_else(|| last_played(fe), |i| i.rider)
}

pub fn prefetch(fe: &FrontEnd, loc: &mut Localization) -> Vec<ShowcaseRider> {
    if fe.career.riders.is_empty() {
        return Vec::new();
    }
    let drop = fe.equipment.drop;
    let mut inv = Inventory::new(&fe.career);
    // (the rider last played first, as the rider step opens on it)
    let last = last_played(fe);
    inv.equip_rider(&fe.career, &fe.profile, drop, last);
    let info = DropInfo::of(&fe.career, &fe.profile, drop);
    let ctx = inv.store_context(&fe.career, &fe.profile, info.as_ref());
    inv.build_lists(&fe.career, &fe.profile, loc, info.as_ref(), &ctx, inventory::RIDER);
    let list: Vec<(u32, bool)> = inv.owned_list.iter().map(|u| (u.key, !u.has(flag::AVAILABLE))).collect();
    build(fe, loc, &inv, &list)
}

/// The 3D Rider Select mod: this rider dressed in each other kind of event's loadout kept for them (Trick It's,
/// Race It's...), to load ahead so going to another kind of event shows them in its gear at once. Those dressed the
/// same as one of `besides` (or each other) left out.
pub fn variants(fe: &FrontEnd, loc: &mut Localization, rider: u32, besides: &[ShowcaseRider]) -> Vec<ShowcaseRider> {
    let Some(locked) = besides.iter().find(|r| r.key == rider).map(|r| r.locked) else { return Vec::new() };
    let here = inventory::event_loadouts::event_kind(&fe.career, fe.equipment.drop);
    let mut out: Vec<ShowcaseRider> = Vec::new();
    for kind in inventory::event_loadouts::kinds_for(rider) {
        if here.as_deref() == Some(kind.as_str()) {
            continue;
        }
        // (an event of that kind to dress the rider for)
        let Some(drop) = fe.rules.events.iter().map(|(&k, _)| k).find(|&k| {
            inventory::event_loadouts::event_kind(&fe.career, k).as_deref() == Some(kind.as_str())
        }) else {
            continue;
        };
        let mut inv = Inventory::new(&fe.career);
        inv.equip_rider(&fe.career, &fe.profile, drop, rider);
        for r in build_for(fe, loc, &inv, &[(rider, locked)], drop) {
            if !besides.contains(&r) && !out.contains(&r) {
                out.push(r);
            }
        }
    }
    out
}

/// Each rider of `list`, dressed as its run would be (`inv`'s rider as `inv` has it, the others as last equipped).
fn build(fe: &FrontEnd, loc: &mut Localization, inv: &Inventory, list: &[(u32, bool)]) -> Vec<ShowcaseRider> {
    build_for(fe, loc, inv, list, fe.equipment.drop)
}

/// `build`, for this event (drop).
fn build_for(fe: &FrontEnd, loc: &mut Localization, inv: &Inventory, list: &[(u32, bool)], drop: u32) -> Vec<ShowcaseRider> {
    let mut riders = Vec::new();
    for &(key, locked) in list {
        let Some(r) = fe.career.riders.iter().find(|r| r.key == key) else { continue };
        // (the rider in use keeps what's equipped now, changes not saved yet included; the others what they had last)
        let mut one = inv.clone();
        if key != inv.rider {
            one.equip_rider(&fe.career, &fe.profile, drop, key);
        }
        let committed = setup::commit(&one, &fe.career, &fe.profile, loc);
        let items: Vec<(usize, u32)> = committed
            .gear
            .iter()
            .enumerate()
            .filter(|(_, g)| !g.is_cleared() && g.description != 0)
            .map(|(t, g)| (t, g.description))
            .collect();
        let suit = one
            .equipped
            .first()
            .and_then(|l| l.first())
            .and_then(|&(key, src)| one.record(&fe.career, &fe.profile, loc, key, src))
            .map(|rec| rec.name.to_string())
            .unwrap_or_default();
        let glowing = suit.to_lowercase().contains("glowing suit");
        let note = match one.equipped.get(1).and_then(|l| l.first()) {
            Some(&(key, src)) => match one.record(&fe.career, &fe.profile, loc, key, src) {
                Some(rec) => {
                    let desc = |k: u32| {
                        fe.career.descriptions.iter().find(|d| d.key == k).map_or_else(
                            || format!("{k:#010x} (no description)"),
                            |d| format!("{k:#010x} (id {}, rider {:#010x}, slot {}, icon {})", d.id, d.rider, d.slot, d.icon),
                        )
                    };
                    format!(
                        "equipped board {:?} key {key:#010x} src {src} rider {:#010x} descriptions [{}, {}]; committed board {}",
                        rec.name,
                        rec.rider,
                        desc(rec.descriptions[0]),
                        desc(rec.descriptions[1]),
                        desc(committed.gear.first().map_or(0, |g| g.description)),
                    )
                }
                None => format!("equipped board {key:#010x} src {src}: no record"),
            },
            None => "no board equipped".to_string(),
        };
        let suit_perk = |n: i8| {
            [setup::JACKET_TYPE, setup::PANTS_TYPE]
                .iter()
                .any(|&t| committed.gear.get(t).is_some_and(|g| g.stats.iter().any(|s| s.0 == n)))
        };
        let (big_head, big_hands) = (
            suit_perk(STAT_BIG_HEAD) || suit_perk(STAT_BIG_HEAD_AND_HANDS),
            suit_perk(STAT_BIG_HANDS) || suit_perk(STAT_BIG_HEAD_AND_HANDS),
        );
        let stat = |n: i8| committed.gear.get(6).and_then(|g| g.stats.iter().find(|s| s.0 == n)).map(|s| s.1);
        riders.push(ShowcaseRider {
            key,
            name: r.name.clone(),
            portrait: r.portrait.to_ascii_lowercase(),
            locked,
            items,
            lamp: [stat(20), stat(21)],
            note,
            suit,
            glowing,
            big_head,
            big_hands,
        });
    }
    riders
}
