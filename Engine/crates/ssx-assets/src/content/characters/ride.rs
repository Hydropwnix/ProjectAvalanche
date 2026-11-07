use std::collections::BTreeSet;
use std::path::Path;

use ssx_content::{BASE_NAMESPACE, ContentId, Kind};

use super::super::GameContent;
use super::super::level::LevelPackages;
use super::budget::RiderLooks;
use super::dress::{DressRequest, LookChoice};

#[derive(Clone, Debug)]
pub struct RideRider {
    pub request: DressRequest,
    pub decision: RiderLooks,
    pub notes: Vec<String>,
}

impl RideRider {
    pub fn stand_in(&self) -> &str {
        &self.request.stand_in
    }
}

pub fn package_items(content: &GameContent, keys: &[u32]) -> Vec<ContentId> {
    let by_key = content.package_item_keys();
    let mut out = Vec::new();
    for id in keys.iter().filter_map(|k| by_key.get(k)) {
        if !out.contains(id) {
            out.push(id.clone());
        }
    }
    out
}

pub fn ride_rider(
    big_root: &Path,
    name: &str,
    level: Option<&LevelPackages>,
    item_keys: &[u32],
    viewer_board: bool,
) -> Result<RideRider, String> {
    let content = GameContent::of(big_root);
    let mut notes = Vec::new();
    let base = |n: &str| ContentId::new(BASE_NAMESPACE, Kind::Character, n).ok();
    let id = match name.parse::<ContentId>() {
        Ok(id) if id.kind() == Kind::Character => Some(id),
        _ => base(name),
    };
    let fallback = crate::character::DEFAULT_CHARACTER;
    let fallback_id = base(fallback).ok_or_else(|| format!("the default rider {fallback:?} is not an id"))?;
    let (id, stand_in) = match id {
        Some(id) if id.namespace() == BASE_NAMESPACE => {
            let n = id.path().to_string();
            (id, n)
        }
        Some(id) => match content.character_def(&id).map(|d| d.value.stands_in_for) {
            Ok(Some(s)) => {
                let n = s.path().to_string();
                (id, n)
            }
            Ok(None) => {
                notes.push(format!("{id} names no stand-in; {fallback} is drawn"));
                (fallback_id, fallback.to_string())
            }
            Err(e) => {
                notes.push(format!("{id} cannot be read ({e}); {fallback} is drawn in its place"));
                (fallback_id, fallback.to_string())
            }
        },
        None => {
            notes.push(format!("{name:?} is not a rider's name; {fallback} is drawn"));
            (fallback_id, fallback.to_string())
        }
    };
    let mut items = package_items(&content, item_keys);
    if viewer_board
        && let Ok(d) = content.character_def(&id)
        && let Some(b) = d.value.default_board
        && !items.contains(&b)
    {
        items.push(b);
    }
    let decision = content.rider_looks(level, std::slice::from_ref(&id), &items);
    let look = decision.look(&id);
    if id.namespace() != BASE_NAMESPACE && look == LookChoice::Disc {
        notes.push(format!("{id}: its look is not drawn; its stand-in {stand_in}'s is"));
    }
    let allowed: BTreeSet<ContentId> = decision.items.clone();
    let request = DressRequest { id, stand_in, look, viewer_board, allowed_items: Some(allowed) };
    Ok(RideRider { request, decision, notes })
}
