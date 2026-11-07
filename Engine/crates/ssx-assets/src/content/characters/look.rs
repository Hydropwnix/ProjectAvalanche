use std::sync::Arc;

use ssx_content::material::{Family, MaterialDef, from_gltf_character};
use ssx_content::model::{Mesh, MeshSkin, Model, ModelMaterial};
use ssx_content::rider::slot_of;
use ssx_content::skeleton::{RowMatrix, Skeleton};
use ssx_content::{ContentId, Layer};

use super::super::GameContent;
use super::super::resolve::MaterialUse;
use super::LookSource;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LookKind {
    Character,
    Gear(&'static [&'static str]),
    Board,
}

#[derive(Clone, Debug)]
pub struct LookPart {
    pub slot: &'static str,
    pub lod: usize,
    pub mesh: Mesh,
    pub joints: Vec<[u16; 4]>,
    pub weights: Vec<[f32; 4]>,
    pub palette: Vec<(String, usize, RowMatrix)>,
    pub face_offsets: bool,
    pub material: MaterialDef,
}

#[derive(Clone, Debug)]
pub struct Look {
    pub id: ContentId,
    pub source: String,
    pub layer: Layer,
    pub model: Arc<Model>,
    pub parts: Vec<LookPart>,
    pub notes: Vec<String>,
}

impl Look {
    pub fn lod(&self, k: usize) -> impl Iterator<Item = &LookPart> {
        self.parts.iter().filter(move |p| p.lod == k)
    }

    pub fn lods(&self) -> usize {
        self.parts.iter().map(|p| p.lod + 1).max().unwrap_or(0)
    }
}

pub struct Skeletons<'a> {
    pub face: Option<&'a Skeleton>,
    pub plain: &'a Skeleton,
}

pub fn face_set(id: &ContentId, def: &ssx_content::rider::CharacterDef) -> Option<String> {
    def.face
        .as_ref()
        .or(def.stands_in_for.as_ref())
        .map(|f| f.path().to_string())
        .or_else(|| (id.namespace() == ssx_content::BASE_NAMESPACE).then(|| id.path().to_string()))
}

fn bind(skin: &ssx_content::model::ModelSkin, s: &Skeletons) -> Result<(Vec<usize>, bool), Vec<String>> {
    if let Some(face) = s.face {
        match ssx_content::model::skin::bind_skin(skin, face) {
            Ok(p) => return Ok((p, true)),
            Err(why) => {
                return ssx_content::model::skin::bind_skin(skin, s.plain).map(|p| (p, false)).map_err(|_| why);
            }
        }
    }
    ssx_content::model::skin::bind_skin(skin, s.plain).map(|p| (p, false))
}

pub fn build(
    content: &GameContent,
    id: &ContentId,
    at: &LookSource,
    model: Arc<Model>,
    kind: LookKind,
    materials: &std::collections::BTreeMap<String, ContentId>,
    skeletons: &Skeletons,
) -> Result<Look, Vec<String>> {
    let mut problems = Vec::new();
    let mut notes = model.notes.clone();
    if model.collision.is_some() {
        notes.push("its collision node is not read (a rider's collision is the physics' own)".into());
    }
    let mut bound = Vec::with_capacity(model.skins.len());
    for (s, skin) in model.skins.iter().enumerate() {
        match bind(skin, skeletons) {
            Ok(b) => bound.push(Some(b)),
            Err(why) => {
                problems.extend(why.into_iter().map(|w| format!("skins[{s}]: {w}")));
                bound.push(None);
            }
        }
    }
    let board = skeletons.plain.find("Board");
    let mut parts = Vec::new();
    for (k, lod) in model.lods.iter().enumerate() {
        for (p, part) in lod.parts.iter().enumerate() {
            let at_part = format!("LOD {k} part {p} ({:?})", part.mesh.name);
            let slot = match kind {
                LookKind::Board => Some("snowboard"),
                _ => slot_of(part.slot.as_deref(), &part.mesh.name),
            };
            let Some(slot) = slot else {
                problems.push(format!(
                    "{at_part}: no slot (name its node after a boarder slot — head, hair, eyes, headgear, jackets, pants, \
                     gloves, boots, goggles, headlamp, protection, thermal, oxygentank, iceaxe, wingsuit — or set \
                     ava_slot)"
                ));
                continue;
            };
            match kind {
                LookKind::Character if slot == "snowboard" => {
                    problems.push(format!("{at_part}: a character has no board part (a board is a board entry)"));
                    continue;
                }
                LookKind::Gear(slots) if !slots.contains(&slot) => {
                    problems.push(format!("{at_part}: the slot {slot}; this gear fills {}", slots.join(", ")));
                    continue;
                }
                _ => {}
            }
            let (joints, weights, palette, face) = match (part.skin, &part.mesh.skin) {
                (Some(s), Some(ms)) => {
                    let Some((pal, face)) = bound.get(s).cloned().flatten() else { continue };
                    let skel = if face { skeletons.face.unwrap_or(skeletons.plain) } else { skeletons.plain };
                    let joint = |u: usize| pal.get(u).and_then(|&j| skel.joints.get(j).map(|sj| (j, sj)));
                    if kind == LookKind::Board
                        && let Some(b) = board
                        && let Some(off) =
                            weighted_joints(ms).find(|&u| joint(u).is_none_or(|(j, _)| !skel.is_below(j, b)))
                    {
                        let name = joint(off).map_or("?", |(_, sj)| sj.name.as_str());
                        problems.push(format!(
                            "{at_part}: weighted to {name:?}; a board is weighted to Board and the joints below it"
                        ));
                        continue;
                    }
                    let Some((joints, used)) = compact(ms, pal.len()) else {
                        problems.push(format!(
                            "{at_part}: weighted to more than {MAX_PART_JOINTS} joints (a part's skin is drawn with at \
                             most {MAX_PART_JOINTS})"
                        ));
                        continue;
                    };
                    let palette: Option<Vec<_>> =
                        used.iter().map(|&u| joint(u).map(|(j, sj)| (sj.name.clone(), j, sj.inverse_bind))).collect();
                    let Some(palette) = palette else {
                        problems.push(format!("{at_part}: a joint index outside its skin"));
                        continue;
                    };
                    (joints, ms.weights.clone(), palette, face)
                }
                (None, _) if kind == LookKind::Board => {
                    let Some(b) = board else {
                        problems.push("the skeleton has no Board joint".into());
                        continue;
                    };
                    let n = part.mesh.positions.len();
                    let j = &skeletons.plain.joints[b];
                    (vec![[0; 4]; n], vec![[1.0, 0.0, 0.0, 0.0]; n], vec![(j.name.clone(), b, j.inverse_bind)], false)
                }
                _ => {
                    problems.push(format!(
                        "{at_part}: not skinned (a rider's part is skinned to the skeleton; a board alone may be rigid)"
                    ));
                    continue;
                }
            };
            let material = match part_material(content, id, &model, part.material, slot, materials) {
                Ok((m, n)) => {
                    notes.extend(n);
                    m
                }
                Err(e) => {
                    problems.push(format!("{at_part}: {e}"));
                    continue;
                }
            };
            let mut mesh = (*part.mesh).clone();
            mesh.skin = None;
            parts.push(LookPart { slot, lod: k, mesh, joints, weights, palette, face_offsets: face, material });
        }
    }
    if parts.iter().all(|p| p.lod != 0) {
        problems.push("no part in LOD 0".into());
    }
    if !problems.is_empty() {
        return Err(problems);
    }
    Ok(Look { id: id.clone(), source: at.source.clone(), layer: at.layer, model, parts, notes })
}

pub const MAX_PART_JOINTS: usize = 256;

fn compact(ms: &MeshSkin, skin_joints: usize) -> Option<(Vec<[u16; 4]>, Vec<usize>)> {
    let mut used: Vec<usize> = weighted_joints(ms).filter(|&j| j < skin_joints).collect();
    used.sort_unstable();
    if used.len() > MAX_PART_JOINTS {
        return None;
    }
    let mut remap = vec![0u16; skin_joints];
    for (new, &old) in used.iter().enumerate() {
        remap[old] = u16::try_from(new).ok()?;
    }
    let joints = ms
        .joints
        .iter()
        .zip(&ms.weights)
        .map(|(j, w)| {
            std::array::from_fn(|i| if w[i] > 0.0 { remap.get(usize::from(j[i])).copied().unwrap_or(0) } else { 0 })
        })
        .collect();
    Some((joints, used))
}

fn weighted_joints(s: &MeshSkin) -> impl Iterator<Item = usize> + '_ {
    let mut seen = std::collections::BTreeSet::new();
    s.joints
        .iter()
        .zip(&s.weights)
        .flat_map(|(j, w)| j.iter().zip(w).filter(|(_, w)| **w > 0.0).map(|(j, _)| usize::from(*j)))
        .filter(move |j| seen.insert(*j))
}

fn part_material(
    content: &GameContent,
    id: &ContentId,
    model: &Model,
    slot_index: Option<usize>,
    slot: &str,
    materials: &std::collections::BTreeMap<String, ContentId>,
) -> Result<(MaterialDef, Vec<String>), String> {
    let gltf: Option<&ModelMaterial> = slot_index.and_then(|i| model.materials.get(i));
    if let Some(bound) = gltf.and_then(|m| m.name.as_ref()).and_then(|n| materials.get(n)) {
        let m = content.material(bound, MaterialUse::Riders).map_err(|e| e.to_string())?;
        if m.value.family != Some(Family::Character) {
            return Err(format!("{bound} is not of the character family"));
        }
        return Ok(((*m.value).clone(), Vec::new()));
    }
    let default = ModelMaterial {
        name: Some("glTF default".into()),
        base_colour: [1.0; 4],
        base_colour_texture: None,
        metallic: 1.0,
        roughness: 1.0,
        metallic_roughness_texture: None,
        normal_texture: None,
        normal_scale: 1.0,
        occlusion_texture: None,
        emissive: [0.0; 3],
        emissive_texture: None,
        alpha: ssx_content::model::AlphaMode::Opaque,
        alpha_cutoff: 0.5,
        double_sided: false,
        extras: Default::default(),
    };
    Ok(from_gltf_character(id, gltf.unwrap_or(&default), slot_index.unwrap_or(0), slot))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_part_keeps_only_the_joints_it_weights() {
        let ms = MeshSkin {
            joints: vec![[5, 2, 0, 0], [2, 7, 9, 0]],
            weights: vec![[0.5, 0.5, 0.0, 0.0], [0.25, 0.75, 0.0, 0.0]],
        };
        let (joints, used) = compact(&ms, 10).unwrap();
        assert_eq!(used, [2, 5, 7]);
        assert_eq!(joints, [[1, 0, 0, 0], [0, 2, 0, 0]]);
        let many = |n: u16| MeshSkin {
            joints: (0..n).map(|i| [i, 0, 0, 0]).collect(),
            weights: (0..n).map(|_| [1.0, 0.0, 0.0, 0.0]).collect(),
        };
        assert_eq!(compact(&many(256), 286).map(|(_, u)| u.len()), Some(256));
        assert!(compact(&many(257), 286).is_none());
    }
}
