use std::collections::BTreeMap;

use anyhow::{Result, bail};
use ssx_assets::content::GameContent;
use ssx_assets::content::resolve::{Found, MaterialUse};
use ssx_assets::content::track_packages::{
    MAX_HELD_BYTES, MAX_IMAGE_BYTES, MAX_TRIANGLES, MAX_VERTICES, MeshPass, Usage,
};
use ssx_assets::track_objects::Placement;
use ssx_content::map::{Map, PropLook};
use ssx_content::model::Model;
use ssx_content::object::ObjectLods;
use ssx_content::{ContentId, material};

use super::ObjectStats;
use super::gather::{Gather, ObjectParts};

pub(super) struct MapObjects {
    pub(super) objects: Vec<ObjectParts>,
    pub(super) placements: Vec<(usize, Placement)>,
    pub(super) used: Usage,
    pub(super) summary: String,
}

fn placement(p: &ssx_content::map::Placement, ao: f32) -> Placement {
    Placement { scale: p.scale, rotation: p.rotation, position: p.position, unk: [1.0, 1.0, ao, 0.0, 0.0, 0.0] }
}

fn over(used: &Usage) -> Option<String> {
    let rows = [
        ("vertices drawn", used.vertices, MAX_VERTICES),
        ("triangles drawn", used.triangles, MAX_TRIANGLES),
        ("bytes of models held", used.held_bytes, MAX_HELD_BYTES),
        ("bytes of images decoded", used.image_bytes, MAX_IMAGE_BYTES),
    ];
    rows.iter().find(|(_, n, cap)| n > cap).map(|(what, n, cap)| format!("{n} {what}; at most {cap} on a level"))
}

pub(super) fn gather(
    gather: &mut Gather,
    content: &GameContent,
    found: &Found<Map>,
    (meshes, base): (&MeshPass, Usage),
    terrain: Usage,
    stats: &mut ObjectStats,
) -> Result<MapObjects> {
    let (id, map) = (&found.id, &*found.value);
    let first_material = gather.materials.len();
    let mut used = terrain;
    let mut objects: Vec<ObjectParts> = Vec::new();
    let mut by_id: BTreeMap<ContentId, Option<usize>> = BTreeMap::new();
    let mut placements = Vec::new();
    if let Some(p) = &map.props {
        used.vertices += p.cost.vertices;
        used.triangles += p.cost.triangles;
        used.held_bytes += p.cost.held_bytes;
        used.image_bytes += p.cost.drawn_image_bytes;
    }
    let mut own = 0;
    for prop in &map.definition.props {
        let object = match &prop.look {
            PropLook::Object(oid) => match by_id.get(oid) {
                Some(&k) => k,
                None => {
                    let k = object_of(gather, content, oid, (meshes, base, &mut used), stats)?.map(|parts| {
                        objects.push(parts);
                        objects.len() - 1
                    });
                    by_id.insert(oid.clone(), k);
                    k
                }
            },
            PropLook::Mesh { .. } => {
                let k = own;
                own += 1;
                let parts = own_mesh(gather, content, found, k, prop.material.as_ref(), stats);
                objects.push(parts);
                Some(objects.len() - 1)
            }
        };
        if let Some(o) = object {
            placements.push((o, placement(&prop.placement, prop.ao)));
        }
    }
    let textures = package_textures(content, &gather.materials[first_material..])?;
    let total = Usage { image_bytes: base.image_bytes + used.image_bytes + textures, ..sum(&base, &used) };
    if let Some(why) = over(&total) {
        bail!("{id}: the level would draw {why}");
    }
    let summary = format!(
        "map {id}: {} props of {} objects; the map draws {} vertices, {} triangles, {} bytes of models held, {} bytes \
         of images and {textures} of package textures (the level: {} vertices, {} triangles, {} bytes held, {} bytes \
         of images)",
        placements.len(),
        objects.len(),
        used.vertices,
        used.triangles,
        used.held_bytes,
        used.image_bytes,
        total.vertices,
        total.triangles,
        total.held_bytes,
        total.image_bytes
    );
    Ok(MapObjects { objects, placements, used, summary })
}

fn sum(a: &Usage, b: &Usage) -> Usage {
    Usage {
        collision_triangles: a.collision_triangles + b.collision_triangles,
        vertices: a.vertices + b.vertices,
        triangles: a.triangles + b.triangles,
        held_bytes: a.held_bytes + b.held_bytes,
        image_bytes: a.image_bytes + b.image_bytes,
        joints: a.joints + b.joints,
    }
}

fn package_textures(content: &GameContent, materials: &[(material::MaterialDef, bool)]) -> Result<u64> {
    let ids: std::collections::BTreeSet<&ContentId> = materials
        .iter()
        .flat_map(|(m, _)| m.textures.values())
        .filter_map(|t| match t {
            material::TextureRef::Id(id) => Some(id),
            _ => None,
        })
        .collect();
    let (mut files, mut bytes) = (std::collections::BTreeSet::new(), 0u64);
    for id in ids {
        let Some((source, layer, _)) = content.package_providers(id).first().cloned() else { continue };
        let (probe, file) = content.texture_probe_from(id, &source, layer).map_err(|e| anyhow::anyhow!("{e}"))?;
        if files.insert((source, file)) {
            bytes += probe.decoded_bytes();
        }
    }
    Ok(bytes)
}

fn object_of(
    gather: &mut Gather,
    content: &GameContent,
    oid: &ContentId,
    (meshes, base, used): (&MeshPass, Usage, &mut Usage),
    stats: &mut ObjectStats,
) -> Result<Option<ObjectParts>> {
    let found = content.map_object_def(oid).map_err(|e| anyhow::anyhow!("{e}"))?;
    let ObjectLods::Model { model, .. } = &found.value.lods else {
        let object = content.disc_game_object(oid)?;
        return Ok(Some(gather.disc_object(content, meshes, &object, stats)));
    };
    let Some((source, layer, _)) = content.package_providers(model).first().cloned() else {
        bail!("{oid}: its model {model} is in no mounted package")
    };
    let (cost, _) = content.model_cost_from(model, &source, layer).map_err(|e| anyhow::anyhow!("{e}"))?;
    let after = Usage {
        vertices: used.vertices + cost.vertices,
        triangles: used.triangles + cost.triangles,
        held_bytes: used.held_bytes + cost.held_bytes,
        image_bytes: used.image_bytes + cost.drawn_image_bytes,
        ..*used
    };
    if let Some(why) = over(&sum(&base, &after)) {
        bail!("{oid}: its model {model} from {source} would bring the level to {why}");
    }
    let m: Found<Model> = content.model_from(model, &source, layer).map_err(|e| anyhow::anyhow!("{e}"))?;
    *used = after;
    Ok(gather.object_with_model(content, &found, Some(&m), stats))
}

fn own_mesh(
    gather: &mut Gather,
    content: &GameContent,
    found: &Found<Map>,
    k: usize,
    material: Option<&ContentId>,
    stats: &mut ObjectStats,
) -> ObjectParts {
    let (id, source) = (&found.id, found.source.as_str());
    let Some(p) = &found.value.props else { return ObjectParts { lods: vec![Vec::new()], thresholds: Vec::new() } };
    let range = p.ranges.get(k).cloned().unwrap_or_default();
    let bound = material.map(|mid| (mid, content.material(mid, MaterialUse::Objects)));
    let mut lod = Vec::new();
    for (i, part) in p.model.lods.first().map(|l| &l.parts[..]).unwrap_or_default().iter().enumerate() {
        if !range.contains(&i) {
            continue;
        }
        let label = format!("{id} prop part {i}");
        let mesh = gather.mesh_ready(id, source, i, part.mesh.clone(), label);
        let sign = part.mesh.tangent_signs.is_some();
        let mat = match &bound {
            Some((mid, Ok(def))) => gather.material_by_id((*mid).clone(), sign, (*def.value).clone()),
            Some((mid, Err(e))) => {
                stats.problems.push(format!("{id}: material {mid}: {e}; its glTF material is drawn"));
                gltf_material(gather, (id, source), &p.model, part.material, sign, stats)
            }
            None => gltf_material(gather, (id, source), &p.model, part.material, sign, stats),
        };
        lod.push((mesh, mat));
    }
    ObjectParts { lods: vec![lod], thresholds: Vec::new() }
}

fn gltf_material(
    gather: &mut Gather,
    (id, source): (&ContentId, &str),
    model: &Model,
    slot: Option<usize>,
    sign: bool,
    stats: &mut ObjectStats,
) -> usize {
    gather.gltf_material(id, source, slot, sign, || {
        let source = slot.and_then(|s| model.materials.get(s)).cloned().unwrap_or_else(super::gather::gltf_default);
        let (def, notes) = material::from_gltf(id, &source, slot.unwrap_or(0));
        stats.problems.extend(notes.into_iter().map(|n| format!("{id}: {n}")));
        def
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_level_over_its_drawn_budget_names_the_amount() {
        let base = Usage { vertices: MAX_VERTICES - 10, image_bytes: MAX_IMAGE_BYTES, ..Usage::default() };
        let map = Usage { vertices: 10, triangles: MAX_TRIANGLES, ..Usage::default() };
        assert_eq!(over(&sum(&base, &map)), None);
        let more = Usage { vertices: 11, ..map };
        assert_eq!(
            over(&sum(&base, &more)).as_deref(),
            Some(format!("{} vertices drawn; at most {MAX_VERTICES} on a level", MAX_VERTICES + 1).as_str())
        );
        let held = Usage { held_bytes: MAX_HELD_BYTES + 1, ..Usage::default() };
        assert!(over(&held).is_some_and(|w| w.contains("bytes of models held")));
    }
}
