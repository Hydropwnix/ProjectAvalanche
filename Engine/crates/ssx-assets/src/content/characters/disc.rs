use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use ssx_content::rider::{CharacterDef, ItemDef, ItemKind, RiderEntry, RiderRecord, gear_type_name};
use ssx_content::{Asset, BASE_NAMESPACE, ContentId, Kind, LoadError, Source};

use super::super::textures::fe_texture_id;
use crate::career::CareerDb;

pub struct DiscRiders {
    records: BTreeMap<ContentId, toml::Table>,
    pub by_key: BTreeMap<(Kind, u32), ContentId>,
    pub looks: BTreeMap<u32, ContentId>,
    pub seconds: f64,
}

pub fn string_id(key: &str) -> Option<ContentId> {
    ContentId::new(BASE_NAMESPACE, Kind::String, &super::super::segment(key)?).ok()
}

impl DiscRiders {
    pub fn index(big: &Path) -> Result<DiscRiders> {
        let start = Instant::now();
        let ids = super::super::records::rider_ids(big)?;
        let db = CareerDb::load_disc(big)?;
        let rider_id = |key: u32| -> Option<ContentId> {
            ids.iter().find(|(_, (k, c))| *k == Kind::Character && *c == key).map(|(id, _)| id.clone())
        };
        let mut records = BTreeMap::new();
        let mut by_key = BTreeMap::new();
        let mut looks = BTreeMap::new();
        for (id, &(kind, key)) in &ids {
            by_key.insert((kind, key), id.clone());
            if kind != Kind::Character
                && let Some(i) = db.items.iter().find(|i| i.key == key)
            {
                looks.extend(i.descriptions.iter().map(|&d| (d, id.clone())));
            }
            let table = match kind {
                Kind::Character => {
                    let Some(r) = db.riders.iter().find(|r| r.key == key) else { continue };
                    let Some(title) = string_id(&r.title) else { continue };
                    let career = RiderRecord {
                        title,
                        display_name: r.display_name.clone(),
                        description: string_id(&r.description),
                        cost: r.cost,
                        locked: r.locked,
                        stat_bonuses: r.stat_bonuses.clone(),
                        portrait: fe_texture_id(&format!("assets/riderPortraitsLarge/locked/{}.dds", r.portrait)),
                        round_icon: fe_texture_id(&r.round_icon),
                        icon: fe_texture_id(&r.icon),
                        default: r.default,
                        unk_940b9b22: r.unk_940b9b22,
                    };
                    CharacterDef { career: Some(career), ..CharacterDef::default() }.to_table()
                }
                _ => {
                    let Some(i) = db.items.iter().find(|i| i.key == key) else { continue };
                    let (Some(title), Some(slot)) = (string_id(&i.title), gear_type_name(i.slot)) else { continue };
                    let kind = match i.kind {
                        0 => ItemKind::Suit,
                        1 => ItemKind::Board,
                        _ => ItemKind::Gear,
                    };
                    ItemDef {
                        kind,
                        slot,
                        title,
                        rider: (i.rider != 0).then(|| rider_id(i.rider)).flatten(),
                        locked: i.locked,
                        default: i.default,
                        level: i.level,
                        quality: i.quality,
                        cost: i.cost,
                        attributes: i.attributes.clone(),
                        icon: None,
                        unk_b6ef00c6: i.unk_b6ef00c6,
                        materials: BTreeMap::new(),
                    }
                    .to_table()
                }
            };
            records.insert(id.clone(), table);
        }
        Ok(DiscRiders { records, by_key, looks, seconds: start.elapsed().as_secs_f64() })
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn id_of(&self, kind: Kind, key: u32) -> Option<&ContentId> {
        self.by_key.get(&(kind, key))
    }

    pub fn key(&self, id: &ContentId) -> Option<u32> {
        self.by_key.iter().find(|(_, i)| *i == id).map(|((_, k), _)| *k)
    }

    pub fn record(&self, id: &ContentId) -> Option<&toml::Table> {
        self.records.get(id)
    }
}

impl Source for DiscRiders {
    fn name(&self) -> &str {
        BASE_NAMESPACE
    }

    fn has(&self, id: &ContentId) -> bool {
        self.records.contains_key(id)
    }

    fn load(&self, id: &ContentId) -> Result<Asset, LoadError> {
        Ok(Asset::Value(Arc::new(self.entry(id)?)))
    }

    fn list(&self, kind: Kind) -> Vec<ContentId> {
        self.records.keys().filter(|id| id.kind() == kind).cloned().collect()
    }

    fn rider_entry(&self, id: &ContentId) -> Option<Result<RiderEntry, LoadError>> {
        super::RIDER_KINDS.contains(&id.kind()).then(|| self.entry(id))
    }
}

impl DiscRiders {
    fn entry(&self, id: &ContentId) -> Result<RiderEntry, LoadError> {
        let t = self.records.get(id).ok_or_else(|| LoadError::Missing(id.clone()))?;
        Ok(RiderEntry { data: Some(("the disc's record".into(), t.clone())), model: None })
    }
}
