use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use ssx_content::model::write_skinned::{WriteJoint, WriteMaterial, WritePart, WriteSkinned, glb};
use ssx_content::rider::{CharacterDef, ItemDef};
use ssx_content::{ContentId, Kind};

use super::GameContent;
use super::export_records::material_toml;
use super::objects::material_def;
use crate::character::{Character, CharacterPart, Loadout, PartMaterial, Srt, load_character, load_character_with};

const TRACK: &str = "alps0101";

fn joints(rig: &ssx_formats::crsf::SceneGraph, bind: &[Srt]) -> Vec<WriteJoint> {
    rig.nodes
        .iter()
        .zip(bind)
        .map(|(n, s)| WriteJoint {
            name: n.name.clone(),
            parent: n.parent,
            translation: s.translation,
            rotation: ssx_formats::ant_anim::normalize(s.rotation),
            scale: s.scale,
        })
        .collect()
}

type LookFiles = (Vec<(String, Vec<u8>)>, BTreeMap<String, ContentId>);

fn look_files(
    big_root: &Path,
    c: &Character,
    parts: &[&CharacterPart],
    stem: &str,
    package: &str,
) -> Result<LookFiles> {
    let (rig, bind, _) = crate::anim_rigops::extend_character_rig(big_root, &c.rig, &c.bind_pose())?;
    let joints = joints(&rig, &bind);
    let mut names: Vec<String> = Vec::new();
    let mut files = Vec::new();
    let mut bound = BTreeMap::new();
    let mut write_parts = Vec::new();
    for p in parts {
        let PartMaterial::Disc(m) = &p.material else { bail!("{}: not a disc part", p.mesh_name) };
        let slot = match names.iter().position(|n| *n == m.name) {
            Some(i) => i,
            None => {
                names.push(m.name.clone());
                let def = GameContent::of(big_root).material_for(m, super::resolve::MaterialUse::Riders);
                let def = if def.family.is_some() { def } else { material_def(m) };
                match material_toml(&def) {
                    Ok(text) => {
                        files.push((format!("materials/{}.toml", m.name), text.into_bytes()));
                        let id = ContentId::new(package, Kind::Material, &m.name.to_ascii_lowercase())
                            .with_context(|| format!("{}: not an id", m.name))?;
                        bound.insert(m.name.clone(), id);
                    }
                    Err(e) => bail!("{}: {e}", m.name),
                }
                names.len() - 1
            }
        };
        write_parts.push(WritePart {
            slot: &p.slot,
            mesh: &p.mesh,
            material: Some(slot),
            palette: p.skin.palette.iter().map(|j| j.node).collect(),
            inverse_binds: p.skin.palette.iter().map(|j| j.inverse_bind).collect(),
            joints: std::borrow::Cow::Borrowed(&p.skin.joints),
            weights: &p.skin.weights,
        });
    }
    let materials = names.into_iter().map(|name| WriteMaterial { name, ..WriteMaterial::default() }).collect();
    let w = WriteSkinned { joints: &joints, parts: write_parts, materials };
    files.push((format!("{stem}.glb"), glb(&w).map_err(anyhow::Error::msg)?));
    Ok((files, bound))
}

fn record_text(head: &str, t: &toml::Table) -> Result<String> {
    Ok(format!("{head}\n{}", toml::to_string(t).context("writing the record")?))
}

pub fn export(big_root: &Path, id: &ContentId, out_dir: &Path, app_dir: &Path, package: &str) -> Result<Vec<PathBuf>> {
    let content = GameContent::of(big_root);
    let disc = content.disc_riders().context("the disc's riders and items could not be indexed")?;
    let record = disc.record(id).with_context(|| format!("{id}: not one of the disc's"))?;
    let leaf = id.path().rsplit('/').next().unwrap_or(id.path()).to_string();
    let files = match id.kind() {
        Kind::Character => {
            let c = load_character(big_root, TRACK, &leaf)?;
            let parts: Vec<&CharacterPart> = c.parts.iter().collect();
            let (mut files, bound) = look_files(big_root, &c, &parts, &leaf, package)?;
            let mut def =
                ssx_content::rider::parse_character_table(record).map_err(|p| anyhow::anyhow!("{id}: {p:?}"))?;
            def.face = Some(id.clone());
            def.materials = bound;
            let head = format!(
                "# {id} as the disc defines it (an export for your own use; content.md §17.4). As an override of \
                 {id}: [career] may go (the disc's record stays). As an added rider: set stands_in_for, and its own \
                 title, display_name, portraits."
            );
            files.push(("character.toml".into(), record_text(&head, &CharacterDef::to_table(&def))?.into_bytes()));
            files
        }
        Kind::Gear | Kind::Board => {
            let item = ssx_content::rider::parse_item_table(record, id.kind() == Kind::Board)
                .map_err(|p| anyhow::anyhow!("{id}: {p:?}"))?;
            let key = disc.key(id).with_context(|| format!("{id}: no record key"))?;
            let (c, parts) = item_parts(big_root, id, key, &item)?;
            let refs: Vec<&CharacterPart> = parts.iter().collect();
            let (mut files, bound) = look_files(big_root, &c, &refs, &leaf, package)?;
            let mut item = item;
            item.materials = bound;
            let file = if id.kind() == Kind::Board { "board.toml" } else { "gear.toml" };
            let head = format!("# {id} as the disc defines it (an export for your own use; content.md §17.5)");
            files.push((file.into(), record_text(&head, &ItemDef::to_table(&item))?.into_bytes()));
            files
        }
        k => bail!("{id}: {} is not a rider kind", k.as_str()),
    };
    super::export_records::write_files(id, out_dir, app_dir, files)
}

fn item_parts(big_root: &Path, id: &ContentId, key: u32, item: &ItemDef) -> Result<(Character, Vec<CharacterPart>)> {
    let db = crate::career::CareerDb::load_disc(big_root)?;
    let i = db.items.iter().find(|i| i.key == key).with_context(|| format!("{id}: no item"))?;
    let desc = i.descriptions.first().copied().context("the item names no look")?;
    let d = db.descriptions.iter().find(|d| d.key == desc).context("its look is not a description")?;
    let rider = [d.rider, i.rider]
        .into_iter()
        .find_map(|k| db.riders.iter().find(|r| r.key == k && k != 0).map(|r| r.name.clone()))
        .unwrap_or_else(|| crate::character::DEFAULT_CHARACTER.to_string());
    let gear = crate::character::GearItem::from_slot(usize::try_from(i.slot).unwrap_or(usize::MAX))
        .or(match i.slot {
            12 => Some(crate::character::GearItem::Jacket),
            13 => Some(crate::character::GearItem::Pants),
            _ => None,
        })
        .with_context(|| format!("{id}: GearItemType {}", i.slot))?;
    let mut loadout = Loadout::default();
    loadout.descriptions.push((gear, desc));
    if gear == crate::character::GearItem::Jacket
        && let Some(&pants) = i.descriptions.get(1)
    {
        loadout.descriptions.push((crate::character::GearItem::Pants, pants));
    }
    let c = load_character_with(big_root, TRACK, &rider, &loadout)?;
    let slots: Vec<&str> = match item.slot {
        "jacket" => vec!["jackets", "pants"],
        s => ssx_content::rider::gear_type(s).map(|(_, _, sl)| sl.to_vec()).unwrap_or_default(),
    };
    let parts = c.parts.iter().filter(|p| slots.contains(&p.slot.as_str())).cloned().collect::<Vec<_>>();
    if parts.is_empty() {
        bail!("{id}: its look draws nothing on {rider}");
    }
    Ok((c, parts))
}
