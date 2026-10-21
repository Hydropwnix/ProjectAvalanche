use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use glam::{Quat, Vec3};
use ssx_assets::track_objects::{Placement, asset_file, load_track_objects};
use ssx_formats::pob::{Matrix, ObjectData, PobFile, Volume, VolumeKind};

use crate::collision::{CollisionWorld, PropEntity, TriEdges, class_byte};

pub fn prop_material_id(child_surface_id: u32, ao: f32) -> u32 {
    let low = if -ao >= 0.0 { 0.0 } else { ao };
    let clamped = if 1.0 - low >= 0.0 { low } else { 1.0 };
    (child_surface_id & 0xFF) | (((clamped * 255.0) as u32) << 8)
}

#[derive(Clone, Debug)]
pub struct LocalCluster {
    pub vertices: Vec<Vec3>,
    pub triangles: Vec<[u8; 3]>,
    pub surface_id: u32,
    pub class: u8,
    pub edges: Vec<TriEdges>,
    pub material: [f32; 3],
}

pub fn unit_triangle_edges(
    unit: &ssx_formats::pob::Unit,
    mesh_flags: u16,
    vertex: impl Fn(u8) -> Vec3,
) -> Result<Vec<TriEdges>> {
    use ssx_formats::pob::edge_cosine;
    let codes = &unit.edge_codes;
    if codes.is_empty() {
        bail!("cluster unit without edge codes (none on the disc: every unit is 0x21 or 0x22)");
    }
    let base = 0x103 | u32::from(mesh_flags & 0x10);
    let make = |vc: [u8; 3], ec: [Option<u8>; 3], diag: (f32, bool)| {
        let mut flags = base;
        let mut cos = [1.0f32; 3];
        for k in 0..3 {
            if vc[k] & 0x40 != 0 {
                flags |= 0x200 << k;
            }
            let (c, convex) = match ec[k] {
                Some(code) => (edge_cosine(code), code & 0x20 != 0),
                None => diag,
            };
            if convex {
                flags |= 0x20 << k;
                cos[k] = c;
            }
        }
        TriEdges { flags, edge_cos: cos }
    };
    let i = &unit.indices;
    Ok(match unit.kind() {
        1 => vec![make([codes[0], codes[1], codes[2]], [Some(codes[0]), Some(codes[1]), Some(codes[2])], (1.0, false))],
        2 => {
            let (v0, v1, v2, v3) = (vertex(i[0]), vertex(i[1]), vertex(i[2]), vertex(i[3]));
            let nb = (v1 - v0).cross(v2 - v0);
            let na = (v2 - v3).cross(v1 - v3);
            let convex = (v2 - v1).dot(nb.cross(na)) > 0.0;
            let (lb, la) = (nb.length_squared(), na.length_squared());
            let cos = if lb > f32::MIN_POSITIVE && la > f32::MIN_POSITIVE {
                nb.dot(na) / (lb.sqrt() * la.sqrt())
            } else {
                1.0
            };
            vec![
                make([codes[0], codes[1], codes[2]], [Some(codes[0]), None, Some(codes[2])], (cos, convex)),
                make([codes[3], codes[2], codes[1]], [Some(codes[3]), None, Some(codes[1])], (cos, convex)),
            ]
        }
        k => bail!("cluster unit kind {k} not handled (none on the disc)"),
    })
}

pub fn pob_part_material(pob: &PobFile) -> [f32; 3] {
    pob.objects
        .iter()
        .find_map(|o| match &o.data {
            ObjectData::AssemblyDefinition(d) => d.parts.first().map(|p| p.material),
            _ => None,
        })
        .unwrap_or(crate::collision::DEFAULT_PART_MATERIAL)
}

pub fn transform_point(m: &Matrix, p: Vec3) -> Vec3 {
    let row = |i: usize| Vec3::from_array(m[i]);
    row(0) * p.x + row(1) * p.y + row(2) * p.z + row(3)
}

pub fn place(pl: &Placement, p: Vec3) -> Vec3 {
    let q = Quat::from_xyzw(pl.rotation[0], pl.rotation[1], pl.rotation[2], pl.rotation[3]);
    Vec3::from_array(pl.position) + q * (Vec3::from_array(pl.scale) * p)
}

pub fn pob_clusters(pob: &PobFile) -> Result<(Vec<LocalCluster>, usize)> {
    let mut out = Vec::new();
    let material = pob_part_material(pob);
    let mut cylinders = 0;
    let outer: Vec<&Volume> =
        pob.objects.iter().filter_map(|o| if let ObjectData::Volume(v) = &o.data { Some(v) } else { None }).collect();
    for ov in outer {
        let Some(sma_at) = ov.aggregate else { bail!("outer volume is a {:?}, expected an aggregate", ov.kind) };
        let Some(ObjectData::SimpleMappedArray(sma)) = pob.object_at(sma_at).map(|o| &o.data) else {
            bail!("outer volume does not point at a SimpleMappedArray");
        };
        for child in &sma.volumes {
            let xf = |p: Vec3| transform_point(&ov.transform, transform_point(&child.transform, p));
            match child.kind {
                VolumeKind::Aggregate => {
                    let at = child.aggregate.context("aggregate child without target")?;
                    let Some(ObjectData::ClusteredMesh(cm)) = pob.object_at(at).map(|o| &o.data) else {
                        bail!("SMA child aggregate is not a ClusteredMesh");
                    };
                    for c in &cm.clusters {
                        let vertices: Vec<Vec3> = c.vertices.iter().map(|&p| xf(Vec3::from_array(p))).collect();
                        let mut triangles = Vec::new();
                        let mut edges = Vec::new();
                        for u in &c.units {
                            triangles.extend(u.triangles().unwrap_or_default());
                            let raw = |k: u8| Vec3::from_array(c.vertices[k as usize]);
                            edges.extend(unit_triangle_edges(u, cm.flags, raw)?);
                        }
                        out.push(LocalCluster {
                            vertices,
                            triangles,
                            surface_id: child.surface_id,
                            class: class_byte(child.group_id),
                            edges,
                            material,
                        });
                    }
                }
                VolumeKind::Cylinder => {
                    child.cylinder().context("cylinder parameters")?;
                    cylinders += 1;
                }
                other => bail!("SMA child volume kind {other:?} not handled (none on the disc)"),
            }
        }
    }
    Ok((out, cylinders))
}

pub fn add_track_props(world: &mut CollisionWorld, big_root: &Path, track: &str) -> Result<()> {
    let objects = load_track_objects(big_root, track)?;
    let mut cache: HashMap<String, Option<Vec<LocalCluster>>> = HashMap::new();
    for (idx, placements) in &objects.instances {
        let Some(pob_path) = objects.objects.get(*idx).and_then(|o| o.pob.as_ref()) else { continue };
        if !cache.contains_key(pob_path) {
            let loaded = match asset_file(big_root, pob_path) {
                Some(path) => {
                    let data = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
                    let pob = PobFile::parse(&data).with_context(|| format!("parsing {}", path.display()))?;
                    let (clusters, cylinders) = pob_clusters(&pob).with_context(|| path.display().to_string())?;
                    world.stats.prop_files += 1;
                    world.stats.prop_cylinders += cylinders;
                    Some(clusters)
                }
                None => {
                    world.stats.prop_missing += 1;
                    world.stats.prop_missing_paths.push(pob_path.clone());
                    None
                }
            };
            cache.insert(pob_path.clone(), loaded);
        }
        let Some(Some(clusters)) = cache.get(pob_path) else { continue };
        let object = &objects.objects[*idx];
        for pl in placements {
            world.stats.prop_instances += 1;
            let entity = world.add_entity(PropEntity { object: object.name.clone(), audio: object.audio.clone() });
            for c in clusters {
                let v: Vec<Vec3> = c.vertices.iter().map(|&p| place(pl, p)).collect();
                let material = prop_material_id(c.surface_id, pl.ao_multiplier());
                world.push_cluster(&v, None, None, &c.triangles, material, c.class);
                world.set_cluster_entity(entity);
                world.set_triangle_edges(&c.edges, c.material);
                world.stats.prop_class_clusters[c.class as usize] += 1;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_material_id_carries_the_placement_ao_byte() {
        assert_eq!(prop_material_id(10, 1.0), 0xFF0A);
        assert_eq!(prop_material_id(41, 0.5), (127 << 8) | 41);
        assert_eq!(prop_material_id(0x10A, 0.0), 0x0A);
        assert_eq!(prop_material_id(10, 3.2) >> 8, 255);
        assert_eq!(prop_material_id(10, -0.0) >> 8, 0);
        assert_eq!(prop_material_id(10, f32::NAN) >> 8, 255);
        let w = crate::surface::object_contact_weights(prop_material_id(10, 0.4), 1.0);
        assert!((w[crate::surface::AMBIENT_OCCLUSION] - 102.0 / 255.0).abs() < 1e-6);
    }
}
