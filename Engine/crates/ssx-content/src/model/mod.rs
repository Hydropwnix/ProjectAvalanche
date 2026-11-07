mod accessor;
pub(crate) mod base64;
pub mod collision;
mod glb;
mod gltf;
mod json;
mod mesh;
mod probe;
pub mod scene;
mod shape;
pub mod skin;
mod tangents;
#[cfg(test)]
mod tests;
mod uri;
pub mod write;
pub mod write_skinned;

use std::ops::Range;
use std::sync::Arc;

pub(crate) use accessor::{Layout, read_f32};
pub use collision::{Collision, CollisionCluster, CollisionTriangle, TriangleEdges};
pub use glb::assemble as assemble_glb;
pub(crate) use gltf::with_buffers;
pub use gltf::{FileReader, decode_file};

pub fn decode_node_meshes(
    rel: &str,
    bytes: &[u8],
    files: &dyn FileReader,
    limits: &Limits,
    nodes: &[usize],
) -> Result<(Model, Vec<Range<usize>>, ModelCost), ModelError> {
    let read = gltf::read_with(rel, bytes, files, limits, |r, dir, held| r.node_meshes(nodes, dir, files, held))?;
    let gltf::NodeMeshes { mut model, ranges, held_bytes, drawn_image_bytes } = read;
    model.source = Some(ContentHash::of(bytes));
    let cost = ModelCost {
        vertices: model.vertex_count() as u64,
        triangles: model.triangle_count() as u64,
        held_bytes,
        drawn_image_bytes,
        ..ModelCost::default()
    };
    Ok((model, ranges, cost))
}
pub use mesh::Transform;
pub use probe::{ModelCost, files_named, files_of, probe_file, read_json};

use crate::hash::ContentHash;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    pub name: String,
    pub positions: Vec<[f32; 3]>,
    pub normals: Option<Vec<[f32; 3]>>,
    pub tangents: Option<Vec<[f32; 3]>>,
    pub tangent_signs: Option<Vec<f32>>,
    pub uv0: Option<Vec<[f32; 2]>>,
    pub uv1: Option<Vec<[f32; 2]>>,
    pub colours: Option<Vec<[u8; 4]>>,
    pub indices: Vec<u32>,
    pub groups: Vec<Range<u32>>,
    pub translucent: bool,
    pub bounds: Option<([f32; 3], [f32; 3])>,
    pub skin: Option<MeshSkin>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeshSkin {
    pub joints: Vec<[u16; 4]>,
    pub weights: Vec<[f32; 4]>,
}

impl Mesh {
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn position_bounds(&self) -> Option<([f32; 3], [f32; 3])> {
        let first = *self.positions.first()?;
        Some(self.positions.iter().fold((first, first), |(lo, hi), p| {
            (std::array::from_fn(|i| lo[i].min(p[i])), std::array::from_fn(|i| hi[i].max(p[i])))
        }))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub mesh: Arc<Mesh>,
    pub material: Option<usize>,
    pub skin: Option<usize>,
    pub slot: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lod {
    pub parts: Vec<Part>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModelMaterial {
    pub name: Option<String>,
    pub base_colour: [f32; 4],
    pub base_colour_texture: Option<ImageRef>,
    pub metallic: f32,
    pub roughness: f32,
    pub metallic_roughness_texture: Option<ImageRef>,
    pub normal_texture: Option<ImageRef>,
    pub normal_scale: f32,
    pub occlusion_texture: Option<ImageRef>,
    pub emissive: [f32; 3],
    pub emissive_texture: Option<ImageRef>,
    pub alpha: AlphaMode,
    pub alpha_cutoff: f32,
    pub double_sided: bool,
    pub extras: Extras,
}

impl ModelMaterial {
    pub fn drawn_images(&self) -> impl Iterator<Item = ImageRef> {
        [self.base_colour_texture, self.normal_texture].into_iter().flatten().filter(|r| r.tex_coord == 0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageRef {
    pub image: usize,
    pub tex_coord: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlphaMode {
    Opaque,
    Mask,
    Blend,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelImage {
    pub file_name: String,
    pub bytes: Arc<Vec<u8>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Extras {
    pub surface: Option<String>,
    pub group: Option<u32>,
    pub slot: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelSkin {
    pub joints: Vec<String>,
    pub inverse_binds: Vec<crate::skeleton::RowMatrix>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Model {
    pub lods: Vec<Lod>,
    pub collision: Option<Arc<Collision>>,
    pub materials: Vec<ModelMaterial>,
    pub images: Vec<ModelImage>,
    pub skins: Vec<ModelSkin>,
    pub source: Option<ContentHash>,
    pub notes: Vec<String>,
}

impl Model {
    pub fn single(mesh: Mesh) -> Model {
        let part = Part { mesh: Arc::new(mesh), material: None, skin: None, slot: None };
        Model { lods: vec![Lod { parts: vec![part] }], ..Model::default() }
    }

    pub fn vertex_count(&self) -> usize {
        self.lods.iter().flat_map(|l| &l.parts).map(|p| p.mesh.positions.len()).sum()
    }

    pub fn triangle_count(&self) -> usize {
        self.lods.iter().flat_map(|l| &l.parts).map(|p| p.mesh.triangle_count()).sum()
    }

    pub fn lod0_mesh(&self) -> Result<(Mesh, Vec<String>), String> {
        let parts = self.lods.first().map(|l| l.parts.as_slice()).unwrap_or_default();
        let [first, rest @ ..] = parts else { return Err("no mesh in LOD 0".into()) };
        if rest.is_empty() {
            return Ok(((*first.mesh).clone(), Vec::new()));
        }
        let all = |f: &dyn Fn(&Mesh) -> bool| parts.iter().all(|p| f(&p.mesh));
        let mut notes = Vec::new();
        let mut keep = |name: &str, every: bool, any: bool| {
            if any && !every {
                notes.push(format!("{name}: not on every part of LOD 0, left out"));
            }
            every
        };
        let any = |f: &dyn Fn(&Mesh) -> bool| parts.iter().any(|p| f(&p.mesh));
        let normals = keep("normals", all(&|m| m.normals.is_some()), any(&|m| m.normals.is_some()));
        let tangents = keep("tangents", all(&|m| m.tangents.is_some()), any(&|m| m.tangents.is_some()));
        let uv0 = keep("TEXCOORD_0", all(&|m| m.uv0.is_some()), any(&|m| m.uv0.is_some()));
        let uv1 = keep("TEXCOORD_1", all(&|m| m.uv1.is_some()), any(&|m| m.uv1.is_some()));
        let colours = keep("COLOR_0", all(&|m| m.colours.is_some()), any(&|m| m.colours.is_some()));
        let signs = tangents && any(&|m| m.tangent_signs.is_some());
        let mut out = Mesh { name: first.mesh.name.clone(), ..Mesh::default() };
        let (mut n, mut t, mut s, mut u0, mut u1, mut c) = (vec![], vec![], vec![], vec![], vec![], vec![]);
        for p in parts {
            let m = &p.mesh;
            let base = u32::try_from(out.positions.len()).map_err(|_| "LOD 0 has too many vertices".to_string())?;
            out.indices.extend(m.indices.iter().map(|i| base + i));
            out.positions.extend(&m.positions);
            n.extend(m.normals.iter().flatten());
            t.extend(m.tangents.iter().flatten());
            match &m.tangent_signs {
                Some(v) => s.extend(v),
                None => s.extend(std::iter::repeat_n(1.0, m.positions.len())),
            }
            u0.extend(m.uv0.iter().flatten());
            u1.extend(m.uv1.iter().flatten());
            c.extend(m.colours.iter().flatten());
        }
        out.normals = normals.then_some(n);
        out.tangents = tangents.then_some(t);
        out.tangent_signs = signs.then_some(s);
        out.uv0 = uv0.then_some(u0);
        out.uv1 = uv1.then_some(u1);
        out.colours = colours.then_some(c);
        out.bounds = out.position_bounds();
        Ok((out, notes))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limits {
    pub max_file_bytes: u64,
    pub max_json_bytes: u64,
    pub max_json_depth: usize,
    pub max_held_bytes: u64,
    pub max_data_uri_bytes: u64,
    pub max_image_side: u32,
    pub max_image_decoded_bytes: u64,
    pub max_items: usize,
    pub max_primitives: usize,
    pub max_node_depth: usize,
    pub max_primitive_vertices: u64,
    pub max_vertices: u64,
    pub max_triangles: u64,
    pub max_lods: usize,
    pub max_coordinate: f32,
    pub read_skins: bool,
    pub max_skins: usize,
    pub max_skin_joints: usize,
    pub max_morph_targets: usize,
    pub max_attributes: usize,
    pub max_extension_values: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_file_bytes: 256 << 20,
            max_json_bytes: 16 << 20,
            max_json_depth: 64,
            max_held_bytes: 256 << 20,
            max_data_uri_bytes: 16 << 20,
            max_image_side: 4096,
            max_image_decoded_bytes: 256 << 20,
            max_items: 4096,
            max_primitives: 4096,
            max_node_depth: 64,
            max_primitive_vertices: 1 << 20,
            max_vertices: 4 << 20,
            max_triangles: 4 << 20,
            max_lods: 8,
            max_coordinate: 1.0e6,
            read_skins: false,
            max_skins: 64,
            max_skin_joints: 286,
            max_morph_targets: 8,
            max_attributes: 64,
            max_extension_values: 1 << 18,
        }
    }
}

impl Limits {
    pub fn skinned() -> Limits {
        Limits {
            read_skins: true,
            max_items: 1024,
            max_primitives: 256,
            max_primitive_vertices: 1 << 18,
            max_vertices: 1 << 20,
            max_triangles: 1 << 20,
            ..Limits::default()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{}", ssx_core::text::located(.file, .message))]
pub struct ModelError {
    pub file: String,
    pub message: String,
}

impl ModelError {
    pub fn new(file: &str, message: impl Into<String>) -> ModelError {
        ModelError { file: file.to_string(), message: message.into() }
    }
}

pub const FORMATS: [&str; 2] = ["glb", "gltf"];
