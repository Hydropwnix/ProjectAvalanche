use std::path::Path;

use anyhow::{Context, Result};
use ssx_content::rider::{OutfitSlot, gear_type};
use ssx_content::{ContentId, Kind};

use super::LookSource;
use super::look::{self, Look, LookKind, LookPart, Skeletons};
use super::skeleton::RIG_FILE;
use super::skeleton::RiderSkeleton;
use crate::character::{
    Character, CharacterPart, GearItem, Loadout, PaletteJoint, PartMaterial, Skin, SlotParts, bind_error,
    equipped_parts, load_character_with, mesh_part, model_matrices,
};
use crate::content::GameContent;
use ssx_content::rider::CharacterDef;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LookChoice {
    Disc,
    Package(LookSource),
}

#[derive(Clone, Debug)]
pub struct DressRequest {
    pub id: ContentId,
    pub stand_in: String,
    pub look: LookChoice,
    pub viewer_board: bool,
    pub allowed_items: Option<std::collections::BTreeSet<ContentId>>,
}

impl DressRequest {
    pub fn disc(name: &str, viewer_board: bool) -> Option<DressRequest> {
        let id = ContentId::new(ssx_content::BASE_NAMESPACE, Kind::Character, name).ok()?;
        Some(DressRequest { id, stand_in: name.to_string(), look: LookChoice::Disc, viewer_board, allowed_items: None })
    }
}

pub const VIEWER_BOARD: &str = "common_board_mesh";

fn split_loadout(content: &GameContent, loadout: &Loadout) -> (Loadout, Vec<(GearItem, ContentId)>) {
    if !content.provides_riders() {
        return (loadout.clone(), Vec::new());
    }
    let keys = content.package_item_keys();
    let mut vault = loadout.clone();
    let mut package = Vec::new();
    vault.descriptions.retain(|&(item, key)| match keys.get(&key) {
        Some(id) => {
            package.push((item, id.clone()));
            false
        }
        None => true,
    });
    (vault, package)
}

fn item_slot(item: GearItem) -> &'static str {
    match item {
        GearItem::Board => "snowboard",
        GearItem::Wingsuit => "wingsuit",
        GearItem::IceAxe => "iceaxe",
        GearItem::Armour => "protection",
        GearItem::Thermal => "thermal",
        GearItem::Oxygen => "oxygentank",
        GearItem::HeadLamp => "headlamp",
        GearItem::PulseGoggles => "goggles",
        GearItem::Jacket => "jackets",
        GearItem::Pants => "pants",
    }
}

fn parts_of(look: &Look, rig: &ssx_formats::crsf::SceneGraph, bind: &[crate::character::Srt]) -> Vec<CharacterPart> {
    parts_of_lod(look, 0, rig, bind)
}

pub(super) fn parts_of_lod(
    look: &Look,
    k: usize,
    rig: &ssx_formats::crsf::SceneGraph,
    bind: &[crate::character::Srt],
) -> Vec<CharacterPart> {
    let model = model_matrices(rig, bind);
    look.lod(k)
        .map(|lp: &LookPart| {
            let skin = Skin {
                joints: lp.joints.clone(),
                weights: lp.weights.clone(),
                palette: lp
                    .palette
                    .iter()
                    .map(|(name, node, ibm)| PaletteJoint { name: name.clone(), node: *node, inverse_bind: *ibm })
                    .collect(),
            };
            let error = bind_error(&lp.mesh, &skin, &model);
            CharacterPart {
                slot: lp.slot.to_string(),
                mesh_name: lp.mesh.name.clone(),
                source: format!("{} ({}): {}", look.source, look.layer, look.id),
                mesh: lp.mesh.clone(),
                skin,
                material: PartMaterial::Def(Box::new(lp.material.clone())),
                face_offsets: lp.face_offsets,
                bind_error: error,
            }
        })
        .collect()
}

pub(super) fn item_look(content: &GameContent, id: &ContentId, skeletons: &Skeletons) -> Result<Look, String> {
    let def = content.item_def(id).map_err(|e| e.to_string())?.value;
    let kind = match gear_type(def.slot) {
        Some(("board", _, _)) => LookKind::Board,
        Some((_, _, slots)) => LookKind::Gear(slots),
        None => return Err(format!("{}: not a GearItemType", def.slot)),
    };
    let (model, at) = content.rider_look_model(id).map_err(|e| e.to_string())?;
    look::build(content, id, &at, model, kind, &def.materials, skeletons).map_err(|p| p.join("; "))
}

impl Character {
    fn replace_slot(&mut self, slot: &str, parts: Vec<CharacterPart>) {
        self.parts.retain(|p| p.slot != slot);
        self.light_nodes.retain(|g| g.slot != slot);
        self.parts.extend(parts);
    }
}

pub fn dress(big_root: &Path, track: Option<&str>, req: &DressRequest, loadout: &Loadout) -> Result<Character> {
    let content = GameContent::of(big_root);
    let (vault, package) = split_loadout(&content, loadout);
    if req.look == LookChoice::Disc && package.is_empty() {
        let mut c = character_on(big_root, track, &req.stand_in, &vault)?;
        viewer_board(big_root, track, req, &mut c);
        return Ok(c);
    }
    let plain = content.riders.skeletons.get(big_root, None)?;
    let built = match &req.look {
        LookChoice::Disc => None,
        LookChoice::Package(at) => {
            let made = (|| -> Result<(CharacterDef, std::sync::Arc<RiderSkeleton>, Look)> {
                let def = content.character_def(&req.id).map_err(anyhow::Error::from)?.value;
                let face = look::face_set(&req.id, &def).unwrap_or_else(|| req.stand_in.clone());
                let sk = content.riders.skeletons.get(big_root, Some(&face))?;
                let skeletons = Skeletons { face: Some(&sk.skeleton), plain: &plain.skeleton };
                let model = content.look_model(&req.id, at)?;
                let look = look::build(&content, &req.id, at, model, LookKind::Character, &def.materials, &skeletons)
                    .map_err(|p| anyhow::anyhow!("{}: {}", req.id, p.join("; ")))?;
                Ok((def, sk, look))
            })();
            match made {
                Ok(b) => Some((at, b)),
                Err(e) => {
                    content.notice(format!(
                        "content: {} from {} is not used (the disc's look is drawn): {e:#}",
                        req.id, at.source
                    ));
                    None
                }
            }
        }
    };
    let sk = match &built {
        Some((_, (_, sk, _))) => sk.clone(),
        None => content.riders.skeletons.get(big_root, Some(&req.stand_in))?,
    };
    let skeletons = Skeletons { face: Some(&sk.skeleton), plain: &plain.skeleton };
    let mut c = match &built {
        None => character_on(big_root, track, &req.stand_in, &vault)?,
        Some((at, (def, _, look))) => {
            let rig_file = crate::track_objects::asset_file(big_root, RIG_FILE).context("the riders' rig")?;
            let mut c = Character {
                name: req.stand_in.clone(),
                chain: format!("package {} ({}): {}; standing in for {}", at.source, at.layer, req.id, req.stand_in),
                parts: parts_of(look, &sk.rig, &sk.bind),
                left_out: Vec::new(),
                rig_file,
                rig: sk.base_rig.clone(),
                face_offsets: sk.face_offsets.clone(),
                snowboard_mesh: None,
                light_nodes: Vec::new(),
                problems: look.notes.clone(),
            };
            for (slot, o) in &def.outfit {
                match o {
                    OutfitSlot::Empty => c.replace_slot(slot, Vec::new()),
                    OutfitSlot::Gear(g) => match item_look(&content, g, &skeletons) {
                        Ok(l) => {
                            let parts: Vec<CharacterPart> =
                                parts_of(&l, &sk.rig, &sk.bind).into_iter().filter(|p| p.slot == *slot).collect();
                            c.replace_slot(slot, parts);
                        }
                        Err(e) => c.problems.push(format!("outfit {slot}: {g}: {e}")),
                    },
                }
            }
            let (slots, problems) = equipped_parts(big_root, track, &req.stand_in, &vault, &c.rig)?;
            c.problems.extend(problems);
            for (slot, s) in slots {
                if package.iter().any(|(item, _)| item_slot(*item) == slot) {
                    continue;
                }
                match s {
                    SlotParts::Equipped(parts, lights) => {
                        c.replace_slot(&slot, parts);
                        c.light_nodes.extend(lights);
                    }
                    SlotParts::Cleared => c.replace_slot(&slot, Vec::new()),
                    SlotParts::Default => {}
                }
            }
            c
        }
    };
    if !package.is_empty() {
        for (item, id) in &package {
            if req.allowed_items.as_ref().is_some_and(|a| !a.contains(id)) {
                c.problems.push(format!("{item:?} {id}: not drawn (the level's budget; the slot as the look has it)"));
                continue;
            }
            match item_look(&content, id, &skeletons) {
                Ok(l) => {
                    let slot = item_slot(*item);
                    let parts = parts_of(&l, &sk.rig, &sk.bind).into_iter().filter(|p| p.slot == slot).collect();
                    c.replace_slot(slot, parts);
                    c.chain.push_str(&format!("; {item:?} {id} from {}", l.source));
                }
                Err(e) => c.problems.push(format!("{item:?} {id}: {e} (the slot as the look has it)")),
            }
        }
    }
    viewer_board(big_root, track, req, &mut c);
    Ok(c)
}

fn character_on(big_root: &Path, track: Option<&str>, name: &str, loadout: &Loadout) -> Result<Character> {
    match track {
        Some(t) => load_character_with(big_root, t, name, loadout),
        None => crate::character::load_character_untracked(big_root, name, loadout),
    }
}

fn viewer_board(big_root: &Path, track: Option<&str>, req: &DressRequest, c: &mut Character) {
    if !req.viewer_board || c.parts.iter().any(|p| p.slot == "snowboard") {
        return;
    }
    let content = GameContent::of(big_root);
    let default = match &req.look {
        LookChoice::Package(_) => content.character_def(&req.id).ok().and_then(|d| d.value.default_board),
        LookChoice::Disc => None,
    };
    let allowed = |b: &ContentId| req.allowed_items.as_ref().is_none_or(|a| a.contains(b));
    if let Some(board) = default.filter(|b| !content.look_sources(b).is_empty() && allowed(b)) {
        let drawn = (|| -> Result<Vec<CharacterPart>> {
            let plain = content.riders.skeletons.get(big_root, None)?;
            let sk = content.riders.skeletons.get(big_root, Some(&req.stand_in))?;
            let skeletons = Skeletons { face: Some(&sk.skeleton), plain: &plain.skeleton };
            let l = item_look(&content, &board, &skeletons).map_err(anyhow::Error::msg)?;
            Ok(parts_of(&l, &sk.rig, &sk.bind).into_iter().filter(|p| p.slot == "snowboard").collect())
        })();
        match drawn {
            Ok(parts) if !parts.is_empty() => {
                c.chain.push_str(&format!("; default board {board}"));
                c.parts.extend(parts);
                return;
            }
            Ok(_) => c.problems.push(format!("default board {board}: no snowboard part (the disc's board is drawn)")),
            Err(e) => c.problems.push(format!("default board {board}: {e:#} (the disc's board is drawn)")),
        }
    }
    match mesh_part(big_root, track, VIEWER_BOARD, "snowboard", &c.rig) {
        Ok(p) => c.parts.push(p),
        Err(e) => c.problems.push(format!("the viewer's board {VIEWER_BOARD}: {e:#}")),
    }
}
