use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use ssx_content::rider::{CharacterDef, ItemDef, ItemKind, gear_type};
use ssx_content::{ContentId, Kind};
use ssx_formats::attrib::fnv1;

use super::super::GameContent;
use super::super::textures::{ui_image_path, ui_path_of};
use crate::career::{CareerDb, Description, Item, ItemContent, Rider, RiderContent};

pub fn content_key(id: &ContentId) -> u32 {
    fnv1(id.as_str().as_bytes())
}

pub fn pants_key(id: &ContentId) -> u32 {
    fnv1(format!("{id}#pants").as_bytes())
}

pub fn string_key(id: &ContentId) -> String {
    if id.namespace() == ssx_content::BASE_NAMESPACE { id.path().to_string() } else { id.to_string() }
}

fn disc_keys(db: &CareerDb) -> BTreeSet<u32> {
    let mut out: BTreeSet<u32> = db.riders.iter().map(|r| r.key).collect();
    out.extend(db.items.iter().map(|i| i.key));
    out.extend(db.descriptions.iter().map(|d| d.key));
    out.extend(db.events.iter().map(|e| e.key));
    out
}

pub fn apply(big_root: &Path, db: &mut CareerDb) -> Vec<(ContentId, String)> {
    let content = GameContent::of(big_root);
    let mut refused = Vec::new();
    if !content.provides_riders() {
        return refused;
    }
    let Some(disc) = content.disc_riders() else { return refused };
    let mut refuse = |id: &ContentId, why: String, line: String| {
        content.notice(line);
        refused.push((id.clone(), why));
    };
    let mut used = disc_keys(db);
    let board_key = |d: &CharacterDef| {
        d.default_board
            .as_ref()
            .and_then(|b| if b.namespace() == ssx_content::BASE_NAMESPACE { disc.key(b) } else { Some(content_key(b)) })
    };
    let touched: BTreeSet<ContentId> = content.with_registry(|r| {
        let mut ids: BTreeSet<ContentId> =
            super::RIDER_KINDS.iter().flat_map(|&k| r.list_above(ssx_content::Layer::Base, k)).collect();
        ids.extend(
            r.patched_above(ssx_content::Layer::Base).into_iter().filter(|i| super::RIDER_KINDS.contains(&i.kind())),
        );
        ids.into_iter().filter(|i| i.namespace() == ssx_content::BASE_NAMESPACE).collect()
    });
    for id in &touched {
        let Some(key) = disc.by_key.iter().find(|(_, i)| *i == id).map(|((_, key), _)| *key) else { continue };
        match id.kind() {
            Kind::Character => match content.character_def(id) {
                Ok(d) => {
                    if let (Some(r), Some(c)) = (db.riders.iter_mut().find(|r| r.key == key), d.value.career.as_ref()) {
                        let disc = (r.stat_bonuses != c.stat_bonuses).then(|| r.stat_bonuses.clone());
                        write_rider(r, c);
                        r.content = Some(RiderContent {
                            id: id.clone(),
                            stand_in: key,
                            portrait: c.portrait.clone(),
                            round_icon: c.round_icon.clone(),
                            icon: c.icon.clone(),
                            added: false,
                            default_board: board_key(&d.value),
                            disc_stat_bonuses: disc,
                        });
                    }
                }
                Err(e) => refuse(id, e.to_string(), format!("content: {id}'s record is not used (the disc's is): {e}")),
            },
            _ => match content.item_def(id) {
                Ok(d) => {
                    if let Some(i) = db.items.iter_mut().find(|i| i.key == key) {
                        let disc = (i.attributes != d.value.attributes).then(|| i.attributes.clone());
                        write_item(i, &d.value, db_rider_key(&d.value));
                        i.content = Some(ItemContent { id: id.clone(), added: false, disc_attributes: disc });
                    }
                }
                Err(e) => refuse(id, e.to_string(), format!("content: {id}'s record is not used (the disc's is): {e}")),
            },
        }
    }
    for id in content.added(Kind::Character) {
        let key = content_key(&id);
        if !used.insert(key) {
            let why = format!("its key {key:#010x} is another record's");
            refuse(&id, why.clone(), format!("content: {id} is not added: {why}"));
            continue;
        }
        let made = content.character_def(&id).map_err(|e| e.to_string()).and_then(|d| {
            let board = board_key(&d.value);
            added_rider(&id, &d.value, board, db)
        });
        match made {
            Ok(r) => db.riders.push(r),
            Err(why) => refuse(&id, why.clone(), format!("content: {id} is not added: {why}")),
        }
    }
    for kind in [Kind::Gear, Kind::Board] {
        for id in content.added(kind) {
            let keys = [content_key(&id), pants_key(&id)];
            if keys.iter().any(|k| used.contains(k)) {
                let why = format!("its key {:#010x} is another record's", keys[0]);
                refuse(&id, why.clone(), format!("content: {id} is not added: {why}"));
                continue;
            }
            match content.item_def(&id).map_err(|e| e.to_string()).and_then(|d| added_item(&id, &d.value, db)) {
                Ok((item, descs)) => {
                    used.extend(keys);
                    db.items.push(item);
                    db.descriptions.extend(descs);
                }
                Err(why) => refuse(&id, why.clone(), format!("content: {id} is not added: {why}")),
            }
        }
    }
    refused
}

fn db_rider_key(d: &ItemDef) -> u32 {
    d.rider.as_ref().map_or(0, |r| {
        if r.namespace() == ssx_content::BASE_NAMESPACE { fnv1(r.path().as_bytes()) } else { content_key(r) }
    })
}

fn write_rider(r: &mut Rider, c: &ssx_content::rider::RiderRecord) {
    r.title = string_key(&c.title);
    r.display_name = c.display_name.clone();
    if let Some(d) = &c.description {
        r.description = string_key(d);
    }
    r.cost = c.cost;
    r.locked = c.locked;
    r.stat_bonuses = c.stat_bonuses.clone();
    r.default = c.default;
    r.unk_940b9b22 = c.unk_940b9b22;
    if let Some(p) = c.round_icon.as_ref().and_then(ui_path_of) {
        r.round_icon = p;
    }
    if let Some(p) = c.icon.as_ref().and_then(ui_path_of) {
        r.icon = p;
    }
}

fn added_rider(id: &ContentId, d: &CharacterDef, default_board: Option<u32>, db: &CareerDb) -> Result<Rider, String> {
    let c = d.career.as_ref().ok_or("an added rider needs its rider record ([career])")?;
    let stand = d.stands_in_for.as_ref().ok_or("an added rider needs stands_in_for (a disc rider)")?;
    let stand_in = fnv1(stand.path().as_bytes());
    if !db.riders.iter().any(|r| r.key == stand_in && r.content.as_ref().is_none_or(|c| !c.added)) {
        return Err(format!("its stand-in {stand} is not a rider of the disc"));
    }
    if db.riders.iter().any(|r| r.display_name.eq_ignore_ascii_case(&c.display_name)) {
        return Err(format!("its display name {:?} is another rider's (it names its stats)", c.display_name));
    }
    let mut r = Rider {
        key: content_key(id),
        name: id.to_string(),
        title: String::new(),
        display_name: String::new(),
        cost: 0,
        locked: false,
        default: false,
        unk_940b9b22: 0,
        stat_bonuses: Vec::new(),
        portrait: String::new(),
        description: String::new(),
        round_icon: String::new(),
        icon: String::new(),
        content: Some(RiderContent {
            id: id.clone(),
            stand_in,
            portrait: c.portrait.clone(),
            round_icon: c.round_icon.clone(),
            icon: c.icon.clone(),
            added: true,
            default_board,
            disc_stat_bonuses: None,
        }),
    };
    write_rider(&mut r, c);
    Ok(r)
}

fn write_item(i: &mut Item, d: &ItemDef, rider: u32) {
    i.title = string_key(&d.title);
    i.rider = rider;
    i.locked = d.locked;
    i.default = d.default;
    i.level = d.level;
    i.quality = d.quality;
    i.cost = d.cost;
    i.attributes = d.attributes.clone();
    i.unk_b6ef00c6 = d.unk_b6ef00c6;
}

fn added_item(id: &ContentId, d: &ItemDef, db: &CareerDb) -> Result<(Item, Vec<Description>), String> {
    let slot = gear_type(d.slot).map(|(_, v, _)| v).ok_or_else(|| format!("{}: not a GearItemType", d.slot))?;
    let rider = db_rider_key(d);
    if rider != 0 && !db.riders.iter().any(|r| r.key == rider) {
        return Err(format!("its rider {} is not a rider", d.rider.as_ref().map_or("", |r| r.as_str())));
    }
    let icon = d.icon.as_ref().and_then(ui_image_path).unwrap_or_default();
    let mut keys = vec![content_key(id)];
    if d.kind == ItemKind::Suit {
        keys.push(pants_key(id));
    }
    let descs = keys
        .iter()
        .zip([slot, 13])
        .map(|(&key, s)| Description { key, rider, id: 0, slot: s, excluded: false, icon: icon.clone() })
        .collect();
    let mut item = Item {
        key: content_key(id),
        group: 0,
        title: String::new(),
        kind: d.kind.value(),
        slot,
        rider,
        locked: false,
        default: false,
        level: 0,
        quality: 0,
        cost: 0,
        attributes: Vec::new(),
        unk_b6ef00c6: 0,
        descriptions: keys,
        content: Some(ItemContent { id: id.clone(), added: true, disc_attributes: None }),
    };
    write_item(&mut item, d, rider);
    Ok((item, descs))
}

pub fn port_strings(db: &CareerDb) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for r in db.riders.iter().filter(|r| r.content.as_ref().is_some_and(|c| c.added)) {
        out.insert(r.title.clone(), r.display_name.clone());
        if r.description.contains(':') {
            out.insert(r.description.clone(), String::new());
        }
    }
    for i in db.items.iter().filter(|i| i.content.as_ref().is_some_and(|c| c.added) && i.title.contains(':')) {
        let shown = i.title.rsplit_once('/').map_or(i.title.as_str(), |(_, k)| k).to_string();
        out.insert(i.title.clone(), shown);
    }
    out
}
