use std::collections::HashMap;
use std::sync::Arc;

use ssx_assets::content::GameContent;
use ssx_assets::content::models::model_id;
use ssx_assets::content::resolve::{Found, MaterialUse};
use ssx_assets::content::track_packages::{MeshPass, TrackPackages};
use ssx_assets::object_mesh::ObjectMesh;
use ssx_assets::track_objects::GameObject;
use ssx_content::material::{self, MaterialDef};
use ssx_content::model::{AlphaMode, Extras, Model, ModelMaterial};
use ssx_content::object::{ObjectDef, ObjectLods};
use ssx_content::{ContentId, Layer};

use super::{ObjectStats, SHADOW_EFFECT};

pub(super) enum MeshJob {
    Disc { id: ContentId, label: String },
    Ready { mesh: Arc<ObjectMesh>, label: String },
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum MeshKey {
    Disc(ContentId),
    Part { model: ContentId, source: String, lod: usize, part: usize },
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum MaterialKey {
    Disc(String),
    Id(ContentId),
    Gltf(ContentId, String, Option<usize>),
}

#[derive(Default)]
pub(super) struct Gather {
    pub(super) meshes: Vec<MeshJob>,
    mesh_ids: HashMap<MeshKey, usize>,
    pub(super) materials: Vec<(MaterialDef, bool)>,
    material_ids: HashMap<(MaterialKey, bool), usize>,
}

pub(super) struct ObjectParts {
    pub(super) lods: Vec<Vec<(usize, usize)>>,
    pub(super) thresholds: Vec<f32>,
}

pub(super) fn gltf_default() -> ModelMaterial {
    ModelMaterial {
        name: None,
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
        alpha: AlphaMode::Opaque,
        alpha_cutoff: 0.5,
        double_sided: false,
        extras: Extras::default(),
    }
}

impl Gather {
    fn mesh(&mut self, key: MeshKey, job: impl FnOnce() -> MeshJob) -> usize {
        if let Some(&i) = self.mesh_ids.get(&key) {
            return i;
        }
        self.meshes.push(job());
        self.mesh_ids.insert(key, self.meshes.len() - 1);
        self.meshes.len() - 1
    }

    fn material(&mut self, key: MaterialKey, tangent_sign: bool, def: impl FnOnce() -> MaterialDef) -> usize {
        if let Some(&i) = self.material_ids.get(&(key.clone(), tangent_sign)) {
            return i;
        }
        self.materials.push((def(), tangent_sign));
        self.material_ids.insert((key, tangent_sign), self.materials.len() - 1);
        self.materials.len() - 1
    }

    pub(super) fn mesh_ready(
        &mut self,
        model: &ContentId,
        source: &str,
        part: usize,
        mesh: Arc<ObjectMesh>,
        label: String,
    ) -> usize {
        let key = MeshKey::Part { model: model.clone(), source: source.to_string(), lod: 0, part };
        self.mesh(key, || MeshJob::Ready { mesh, label })
    }

    pub(super) fn material_by_id(&mut self, id: ContentId, tangent_sign: bool, def: MaterialDef) -> usize {
        self.material(MaterialKey::Id(id), tangent_sign, || def)
    }

    pub(super) fn gltf_material(
        &mut self,
        model: &ContentId,
        source: &str,
        slot: Option<usize>,
        tangent_sign: bool,
        def: impl FnOnce() -> MaterialDef,
    ) -> usize {
        self.material(MaterialKey::Gltf(model.clone(), source.to_string(), slot), tangent_sign, def)
    }

    pub(super) fn disc_object(
        &mut self,
        content: &GameContent,
        meshes: &MeshPass,
        object: &GameObject,
        stats: &mut ObjectStats,
    ) -> ObjectParts {
        let mut lods = Vec::with_capacity(object.lods.len());
        for lod in &object.lods {
            let mut lod_parts = Vec::new();
            for m in lod {
                if m.material.effect.as_deref() == Some(SHADOW_EFFECT) {
                    stats.shadow_proxies += 1;
                }
                let Some(id) = model_id(&m.geometry).filter(|id| content.has(id)) else {
                    stats.problems.push(format!("missing {}", m.geometry));
                    continue;
                };
                let parts: Vec<(usize, bool)> = match meshes.model(&id) {
                    Some(found) => self.model_parts(found, 0, stats),
                    None => {
                        let label = id.path().rsplit('/').next().unwrap_or_default().to_string();
                        vec![(self.mesh(MeshKey::Disc(id.clone()), || MeshJob::Disc { id: id.clone(), label }), false)]
                    }
                };
                for (mesh, sign) in parts {
                    let material = self.material(MaterialKey::Disc(m.material.name.clone()), sign, || {
                        content.material_for(&m.material, MaterialUse::Objects)
                    });
                    lod_parts.push((mesh, material));
                }
            }
            lods.push(lod_parts);
        }
        ObjectParts { lods, thresholds: object.lod_thresholds.clone() }
    }

    fn model_parts(&mut self, found: &Found<Model>, lod: usize, stats: &mut ObjectStats) -> Vec<(usize, bool)> {
        let parts = found.value.lods.get(lod).map(|l| l.parts.as_slice()).unwrap_or_default();
        if lod == 0 && found.layer != Layer::Base && (found.value.lods.len() > 1 || found.value.collision.is_some()) {
            stats.problems.push(format!(
                "{} from {}: in place of one disc mesh only its LOD 0 is drawn (an object override gives LODs and \
                 collision)",
                found.id, found.source
            ));
        }
        parts
            .iter()
            .enumerate()
            .map(|(p, part)| {
                let key = MeshKey::Part { model: found.id.clone(), source: found.source.clone(), lod, part: p };
                let label = format!("{} lod{lod} part{p}", found.id);
                let mesh = self.mesh(key, || MeshJob::Ready { mesh: part.mesh.clone(), label });
                (mesh, part.mesh.tangent_signs.is_some())
            })
            .collect()
    }

    pub(super) fn package_object(
        &mut self,
        content: &GameContent,
        packages: &TrackPackages,
        found: &Found<ObjectDef>,
        stats: &mut ObjectStats,
    ) -> Option<ObjectParts> {
        let ObjectLods::Model { model, .. } = &found.value.lods else { return None };
        self.object_with_model(content, found, packages.object_model(model), stats)
    }

    pub(super) fn object_with_model(
        &mut self,
        content: &GameContent,
        found: &Found<ObjectDef>,
        m: Option<&Found<Model>>,
        stats: &mut ObjectStats,
    ) -> Option<ObjectParts> {
        let ObjectLods::Model { model, materials, material } = &found.value.lods else { return None };
        let Some(m) = m else {
            stats.problems.push(format!("{} from {}: its model {model} was not decoded", found.id, found.source));
            return None;
        };
        let mut lods = Vec::with_capacity(m.value.lods.len());
        for (k, lod) in m.value.lods.iter().enumerate() {
            let mut lod_parts = Vec::new();
            for (p, part) in lod.parts.iter().enumerate() {
                let key = MeshKey::Part { model: m.id.clone(), source: m.source.clone(), lod: k, part: p };
                let label = format!("{} lod{k} part{p}", m.id);
                let mesh = self.mesh(key, || MeshJob::Ready { mesh: part.mesh.clone(), label });
                let sign = part.mesh.tangent_signs.is_some();
                let slot = part.material;
                let gltf = slot.and_then(|s| m.value.materials.get(s));
                let bound = gltf.and_then(|g| g.name.as_ref()).and_then(|n| materials.get(n)).or(match slot {
                    None => material.as_ref(),
                    Some(_) => None,
                });
                let bound = match bound.map(|id| (id, content.material(id, MaterialUse::Objects))) {
                    Some((id, Ok(def))) => Some((id.clone(), def)),
                    Some((id, Err(e))) => {
                        stats.problems.push(format!("{}: material {id}: {e}; its glTF material is drawn", found.id));
                        None
                    }
                    None => None,
                };
                let material = match bound {
                    Some((id, def)) => self.material(MaterialKey::Id(id), sign, || (*def.value).clone()),
                    None => self.material(MaterialKey::Gltf(m.id.clone(), m.source.clone(), slot), sign, || {
                        let source = gltf.cloned().unwrap_or_else(gltf_default);
                        let (def, notes) = material::from_gltf(&m.id, &source, slot.unwrap_or(0));
                        stats.problems.extend(notes.into_iter().map(|n| format!("{}: {n}", m.id)));
                        def
                    }),
                };
                if self.materials[material].0.effect == SHADOW_EFFECT {
                    stats.shadow_proxies += 1;
                }
                lod_parts.push((mesh, material));
            }
            lods.push(lod_parts);
        }
        stats.package_objects.push(format!("{} from {}", found.id, found.source));
        Some(ObjectParts { lods, thresholds: found.value.lod_thresholds.clone() })
    }
}
