use serde_json::{Value, json};

use super::collision::SURFACES;
use super::{Collision, Mesh, assemble_glb};

pub struct WriteModel<'a> {
    pub name: &'a str,
    pub lods: Vec<Vec<(&'a Mesh, Option<usize>)>>,
    pub materials: Vec<String>,
    pub collision: Option<&'a Collision>,
}

#[derive(Default)]
pub(crate) struct Bin {
    pub(crate) data: Vec<u8>,
    pub(crate) views: Vec<Value>,
    pub(crate) accessors: Vec<Value>,
}

impl Bin {
    pub(crate) fn push(
        &mut self,
        bytes: &[u8],
        count: usize,
        kind: &str,
        component: u32,
        normalized: bool,
        extra: Value,
    ) -> usize {
        while !self.data.len().is_multiple_of(4) {
            self.data.push(0);
        }
        self.views.push(json!({"buffer": 0, "byteOffset": self.data.len(), "byteLength": bytes.len()}));
        self.data.extend_from_slice(bytes);
        let mut a =
            json!({"bufferView": self.views.len() - 1, "count": count, "type": kind, "componentType": component});
        if normalized {
            a["normalized"] = json!(true);
        }
        if let (Value::Object(a), Value::Object(e)) = (&mut a, extra) {
            a.extend(e);
        }
        self.accessors.push(a);
        self.accessors.len() - 1
    }

    pub(crate) fn floats(
        &mut self,
        values: impl Iterator<Item = f32>,
        count: usize,
        kind: &str,
        extra: Value,
    ) -> usize {
        let bytes: Vec<u8> = values.flat_map(f32::to_le_bytes).collect();
        self.push(&bytes, count, kind, 5126, false, extra)
    }

    pub(super) fn primitive(&mut self, mesh: &Mesh, material: Option<usize>) -> Value {
        let n = mesh.positions.len();
        let (lo, hi) = mesh.position_bounds().unwrap_or(([0.0; 3], [0.0; 3]));
        let mut attributes = serde_json::Map::new();
        let pos = self.floats(mesh.positions.iter().flatten().copied(), n, "VEC3", json!({"min": lo, "max": hi}));
        attributes.insert("POSITION".into(), json!(pos));
        if let Some(v) = &mesh.normals {
            attributes.insert("NORMAL".into(), json!(self.floats(v.iter().flatten().copied(), n, "VEC3", json!({}))));
        }
        if let Some(t) = &mesh.tangents {
            let signs = mesh.tangent_signs.as_deref();
            let values = t
                .iter()
                .enumerate()
                .flat_map(|(i, t)| [t[0], t[1], t[2], signs.and_then(|s| s.get(i)).copied().unwrap_or(1.0)]);
            attributes.insert("TANGENT".into(), json!(self.floats(values, n, "VEC4", json!({}))));
        }
        for (name, uv) in [("TEXCOORD_0", &mesh.uv0), ("TEXCOORD_1", &mesh.uv1)] {
            if let Some(uv) = uv {
                attributes.insert(name.into(), json!(self.floats(uv.iter().flatten().copied(), n, "VEC2", json!({}))));
            }
        }
        if let Some(c) = &mesh.colours {
            let bytes: Vec<u8> = c.iter().flat_map(|b| [b[3], b[2], b[1], b[0]]).collect();
            attributes.insert("COLOR_0".into(), json!(self.push(&bytes, n, "VEC4", 5121, true, json!({}))));
        }
        let indices: Vec<u8> = mesh.indices.iter().flat_map(|i| i.to_le_bytes()).collect();
        let idx = self.push(&indices, mesh.indices.len(), "SCALAR", 5125, false, json!({}));
        let mut p = json!({"attributes": attributes, "indices": idx, "mode": 4});
        if let Some(m) = material {
            p["material"] = json!(m);
        }
        p
    }
}

pub fn glb(model: &WriteModel<'_>) -> Result<Vec<u8>, String> {
    let meshes_of = model.lods.iter().flatten().map(|(m, _)| *m);
    for m in meshes_of {
        let finite = m.positions.iter().flatten().all(|v| v.is_finite());
        if !finite {
            return Err(format!("{}: a position that is not a finite number", m.name));
        }
    }
    let mut bin = Bin::default();
    let (mut nodes, mut meshes, mut roots) = (Vec::new(), Vec::new(), Vec::new());
    for (k, parts) in model.lods.iter().enumerate() {
        let primitives: Vec<Value> = parts.iter().map(|(m, mat)| bin.primitive(m, *mat)).collect();
        meshes.push(json!({"name": format!("lod{k}"), "primitives": primitives}));
        let name = if k == 0 { model.name.to_string() } else { format!("{}_lod{k}", model.name) };
        roots.push(nodes.len());
        nodes.push(json!({"name": name, "mesh": meshes.len() - 1}));
    }
    if let Some(c) = model.collision {
        let mut sets: Vec<((u32, u32), Mesh)> = Vec::new();
        for cluster in &c.clusters {
            let key = (cluster.surface_id, cluster.group);
            let at = match sets.iter().position(|(k, _)| *k == key) {
                Some(i) => i,
                None => {
                    sets.push((key, Mesh::default()));
                    sets.len() - 1
                }
            };
            let mesh = &mut sets[at].1;
            let base = mesh.positions.len() as u32;
            mesh.positions.extend(&cluster.vertices);
            mesh.indices.extend(cluster.triangles.iter().flat_map(|t| t.map(|i| base + u32::from(i))));
        }
        let mut children = Vec::new();
        for ((surface, group), mesh) in &sets {
            let name = SURFACES.iter().find(|(_, id)| id == surface).map_or("powder", |(n, _)| n);
            let primitive = bin.primitive(mesh, None);
            meshes.push(json!({"name": format!("collision_{name}_{group}"), "primitives": [primitive]}));
            let mut extras = json!({"ava_surface": name});
            if *group != 0 {
                extras["ava_group"] = json!(group);
            }
            children.push(nodes.len());
            nodes
                .push(json!({"name": format!("collision_{name}_{group}"), "mesh": meshes.len() - 1, "extras": extras}));
        }
        roots.push(nodes.len());
        nodes.push(json!({"name": "collision", "children": children}));
    }
    let materials: Vec<Value> = model.materials.iter().map(|n| json!({"name": n})).collect();
    let mut root = json!({
        "asset": {"version": "2.0", "generator": "Project Avalanche content export"},
        "scene": 0,
        "scenes": [{"nodes": roots}],
        "nodes": nodes,
        "meshes": meshes,
        "accessors": bin.accessors,
        "bufferViews": bin.views,
        "buffers": [{"byteLength": bin.data.len()}],
    });
    if !materials.is_empty() {
        root["materials"] = json!(materials);
    }
    let text = serde_json::to_vec(&root).map_err(|e| format!("writing the glTF JSON: {e}"))?;
    Ok(assemble_glb(&text, Some(&bin.data)))
}
