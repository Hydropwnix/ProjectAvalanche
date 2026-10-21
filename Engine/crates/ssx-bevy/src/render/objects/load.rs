use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use anyhow::{Context, Result};
use bevy::asset::RenderAssetUsages;
use bevy::mesh::Indices;
use bevy::prelude::*;
use bevy::render::render_resource::PrimitiveTopology;
use ssx_assets::object_mesh::{self, ObjectMesh};
use ssx_assets::track_objects::{Placement, asset_file, load_track_objects};
use ssx_assets::vault_db::Material as VaultMaterial;
use ssx_formats::geom::GeomFile;

use super::specs::prepare_materials;
use super::{LoadedObjects, ObjectStats, SHADOW_EFFECT};

#[derive(Clone, Debug)]
pub(super) struct ObjectLod {
    pub(super) thresholds: Vec<f32>,
    pub(super) centre: Vec3,
    pub(super) half: Vec3,
    pub(super) class: ModelClass,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ModelClass {
    Prelight = 0,
    Forward = 1,
    Translucent = 2,
    Empty = 3,
}

impl ModelClass {
    fn of<'a>(techniques: impl IntoIterator<Item = Option<&'a str>>) -> ModelClass {
        let (mut forward, mut translucent, mut other) = (false, false, false);
        for t in techniques {
            forward |= t == Some("forward");
            translucent |= t == Some("translucent");
            other |= !forward && !translucent;
            if forward && translucent && other {
                break;
            }
        }
        match (other, translucent, forward) {
            (true, _, _) => ModelClass::Prelight,
            (false, true, _) => ModelClass::Translucent,
            (false, false, true) => ModelClass::Forward,
            _ => ModelClass::Empty,
        }
    }
}

impl ObjectLod {
    const EMPTY_HALF: f32 = 1.192_092_9e-7;

    pub(super) fn from_box(thresholds: Vec<f32>, lo_hi: Option<(Vec3, Vec3)>, class: ModelClass) -> Self {
        match lo_hi {
            Some((lo, hi)) => {
                let centre = (lo + hi) * 0.5;
                ObjectLod { thresholds, centre, half: hi - centre, class }
            }
            None => ObjectLod { thresholds, centre: Vec3::ZERO, half: Vec3::splat(Self::EMPTY_HALF), class },
        }
    }
}

pub fn load(big_root: &Path, track: &str) -> Result<LoadedObjects> {
    let start = Instant::now();
    let vault = load_track_objects(big_root, track)
        .with_context(|| format!("loading the objects of {track} from {}", big_root.display()))?;
    let mut stats = ObjectStats { objects: vault.objects.len(), ..default() };

    let mut geoms: Vec<(String, PathBuf)> = Vec::new();
    let mut geom_ids: HashMap<String, usize> = HashMap::new();
    let mut vault_materials: Vec<&VaultMaterial> = Vec::new();
    let mut material_ids: HashMap<&str, usize> = HashMap::new();
    let mut parts = Vec::with_capacity(vault.objects.len());
    for object in &vault.objects {
        let mut object_lods = Vec::with_capacity(object.lods.len());
        for lod in &object.lods {
            let mut lod_parts = Vec::new();
            for mesh in lod {
                let effect = mesh.material.effect.as_deref().unwrap_or("");
                if effect == SHADOW_EFFECT {
                    stats.shadow_proxies += 1;
                }
                let geom = match geom_ids.get(&mesh.geometry) {
                    Some(&g) => g,
                    None => {
                        let Some(path) = asset_file(big_root, &mesh.geometry) else {
                            stats.problems.push(format!("missing {}", mesh.geometry));
                            continue;
                        };
                        let stem = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
                        geoms.push((stem, path));
                        geom_ids.insert(mesh.geometry.clone(), geoms.len() - 1);
                        geoms.len() - 1
                    }
                };
                let material = *material_ids.entry(mesh.material.name.as_str()).or_insert_with(|| {
                    vault_materials.push(&mesh.material);
                    vault_materials.len() - 1
                });
                lod_parts.push((geom, material));
            }
            object_lods.push(lod_parts);
        }
        stats.objects_drawn += usize::from(object_lods.iter().any(|l| !l.is_empty()));
        parts.push(object_lods);
    }

    let loaded: Vec<(String, Mesh, (Vec3, Vec3))> =
        parallel_map(&geoms, |(stem, path)| -> Result<(String, Mesh, (Vec3, Vec3))> {
            let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
            let geom = GeomFile::parse(&data).with_context(|| format!("parsing {}", path.display()))?;
            let bounds = geom
                .meshes
                .first()
                .map_or((Vec3::ZERO, Vec3::ZERO), |m| (Vec3::from_array(m.bbox_min), Vec3::from_array(m.bbox_max)));
            let mesh = object_mesh::object_mesh(&geom).with_context(|| format!("converting {}", path.display()))?;
            Ok((stem.clone(), to_bevy_mesh(mesh, stem), bounds))
        })
        .into_iter()
        .collect::<Result<_>>()?;
    let geom_bounds: Vec<(Vec3, Vec3)> = loaded.iter().map(|(_, _, b)| *b).collect();
    let meshes: Vec<(String, Mesh)> = loaded.into_iter().map(|(s, m, _)| (s, m)).collect();
    let lods: Vec<ObjectLod> = vault
        .objects
        .iter()
        .zip(&parts)
        .map(|(object, object_parts)| {
            let thresholds =
                (0..object.lods.len()).map(|k| object.lod_thresholds.get(k).copied().unwrap_or(0.0)).collect();
            let lod0 = object_parts.first().map(Vec::as_slice).unwrap_or_default();
            let union = lod0.iter().map(|&(g, _)| geom_bounds[g]).reduce(|(lo, hi), (l, h)| (lo.min(l), hi.max(h)));
            let techniques = object_parts.iter().flatten().map(|&(_, m)| vault_materials[m].technique.as_deref());
            ObjectLod::from_box(thresholds, union, ModelClass::of(techniques))
        })
        .collect();

    let (images, materials) = prepare_materials(big_root, &vault_materials, &mut stats.problems)?;

    let placements: Vec<(usize, Placement)> =
        vault.instances.iter().flat_map(|(object, list)| list.iter().map(move |p| (*object, *p))).collect();
    for (object, _) in &placements {
        for &(_, material) in parts[*object].iter().flatten() {
            let effect = vault_materials[material].effect.clone().unwrap_or_else(|| "(no effect)".into());
            *stats.effects.entry(effect).or_default() += 1;
            stats.entities += 1;
        }
    }
    stats.placements = placements.len();
    stats.meshes = meshes.len();
    stats.materials = materials.len();
    stats.textures = images.len();
    stats.load_secs = start.elapsed().as_secs_f32();
    Ok(LoadedObjects { stats, meshes, images, materials, parts, lods, placements })
}

pub(super) fn parallel_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get()).min(items.len()).max(1);
    let next = AtomicUsize::new(0);
    let mut out: Vec<Option<R>> = (0..items.len()).map(|_| None).collect();
    std::thread::scope(|scope| {
        let jobs: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(item) = items.get(i) else { break };
                        done.push((i, f(item)));
                    }
                    done
                })
            })
            .collect();
        for job in jobs {
            for (i, r) in job.join().unwrap_or_else(|e| std::panic::resume_unwind(e)) {
                out[i] = Some(r);
            }
        }
    });
    out.into_iter().map(|r| r.unwrap_or_else(|| unreachable!("every index is taken once"))).collect()
}

pub(super) fn to_bevy_mesh(mesh: ObjectMesh, label: &str) -> Mesh {
    let mut out = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, mesh.positions)
        .with_inserted_indices(Indices::U32(mesh.indices));
    if let Some(colours) = &mesh.colours {
        let lanes: Vec<[f32; 2]> = colours.iter().map(|c| [f32::from(c[2]) / 255.0, f32::from(c[1]) / 255.0]).collect();
        out.insert_attribute(Mesh::ATTRIBUTE_UV_1, lanes);
    }
    match mesh.normals {
        Some(normals) => {
            let unit = |n: [f32; 3]| Vec3::from_array(n).try_normalize().unwrap_or(Vec3::Y).to_array();
            out.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals.into_iter().map(unit).collect::<Vec<_>>());
        }
        None => out.compute_smooth_normals(),
    }
    if let Some(uv0) = mesh.uv0 {
        out.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv0);
    }
    match mesh.tangents {
        Some(tangents) => {
            let tangents: Vec<[f32; 4]> = tangents.into_iter().map(|[x, y, z]| [x, y, z, 1.0]).collect();
            out.insert_attribute(Mesh::ATTRIBUTE_TANGENT, tangents);
        }
        None if out.contains_attribute(Mesh::ATTRIBUTE_UV_0) => warn!("{label}: no g0; drawn without its normal map"),
        None => {}
    }
    out
}
