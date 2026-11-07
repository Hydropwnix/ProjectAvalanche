use serde_json::{Value, json};

use super::assemble_glb;
use super::write::Bin;
use crate::skeleton::RowMatrix;

#[derive(Clone, Debug)]
pub struct WriteJoint {
    pub name: String,
    pub parent: Option<usize>,
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

pub struct WritePart<'a> {
    pub slot: &'a str,
    pub mesh: &'a super::Mesh,
    pub material: Option<usize>,
    pub palette: Vec<usize>,
    pub inverse_binds: Vec<RowMatrix>,
    pub joints: std::borrow::Cow<'a, [[u16; 4]]>,
    pub weights: &'a [[f32; 4]],
}

#[derive(Clone, Debug, Default)]
pub struct WriteMaterial {
    pub name: String,
    pub base_colour: Option<[f32; 4]>,
    pub metallic_roughness: Option<(f32, f32)>,
}

pub struct WriteSkinned<'a> {
    pub joints: &'a [WriteJoint],
    pub parts: Vec<WritePart<'a>>,
    pub materials: Vec<WriteMaterial>,
}

pub fn glb(model: &WriteSkinned<'_>) -> Result<Vec<u8>, String> {
    let nj = model.joints.len();
    for (i, j) in model.joints.iter().enumerate() {
        if j.parent.is_some_and(|p| p >= i) {
            return Err(format!("joint {}: its parent does not come before it", ssx_core::text::quoted(&j.name)));
        }
        if j.translation.iter().chain(&j.rotation).chain(&j.scale).any(|v| !v.is_finite()) {
            return Err(format!("joint {}: a value that is not finite", ssx_core::text::quoted(&j.name)));
        }
    }
    let mut bin = Bin::default();
    let mut nodes: Vec<Value> = model
        .joints
        .iter()
        .map(|j| json!({"name": j.name, "translation": j.translation, "rotation": j.rotation, "scale": j.scale}))
        .collect();
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); nj];
    let mut roots = Vec::new();
    for (i, j) in model.joints.iter().enumerate() {
        match j.parent {
            Some(p) => children[p].push(i),
            None => roots.push(i),
        }
    }
    for (i, c) in children.into_iter().enumerate() {
        if !c.is_empty() {
            nodes[i]["children"] = json!(c);
        }
    }
    let (mut meshes, mut skins) = (Vec::new(), Vec::new());
    for p in &model.parts {
        let m = p.mesh;
        let n = m.positions.len();
        if m.positions.iter().flatten().any(|v| !v.is_finite()) {
            return Err(format!("{}: a position that is not a finite number", p.slot));
        }
        if p.joints.len() != n || p.weights.len() != n || p.palette.len() != p.inverse_binds.len() {
            return Err(format!("{}: influences or palette of another length", p.slot));
        }
        if p.palette.iter().any(|&j| j >= nj) || p.joints.iter().flatten().any(|&j| usize::from(j) >= p.palette.len()) {
            return Err(format!("{}: a joint index out of range", p.slot));
        }
        let mut prim = bin.primitive(m, p.material);
        let joints: Vec<u8> = p.joints.iter().flatten().flat_map(|j| j.to_le_bytes()).collect();
        let ja = bin.push(&joints, n, "VEC4", 5123, false, json!({}));
        let wa = bin.floats(p.weights.iter().flatten().copied(), n, "VEC4", json!({}));
        prim["attributes"]["JOINTS_0"] = json!(ja);
        prim["attributes"]["WEIGHTS_0"] = json!(wa);
        meshes.push(json!({"name": p.slot, "primitives": [prim]}));
        let ibm = bin.floats(p.inverse_binds.iter().flatten().flatten().copied(), p.palette.len(), "MAT4", json!({}));
        skins.push(json!({"joints": p.palette, "inverseBindMatrices": ibm}));
        roots.push(nodes.len());
        nodes.push(json!({
            "name": p.slot,
            "mesh": meshes.len() - 1,
            "skin": skins.len() - 1,
            "extras": {"ava_slot": p.slot},
        }));
    }
    let mut materials: Vec<Value> = Vec::new();
    for m in &model.materials {
        let mut v = json!({"name": m.name});
        if let Some(c) = m.base_colour {
            if c.iter().any(|x| !x.is_finite()) {
                return Err(format!("material {}: a colour that is not finite", ssx_core::text::quoted(&m.name)));
            }
            v["pbrMetallicRoughness"]["baseColorFactor"] = json!(c);
        }
        if let Some((metal, rough)) = m.metallic_roughness {
            v["pbrMetallicRoughness"]["metallicFactor"] = json!(metal);
            v["pbrMetallicRoughness"]["roughnessFactor"] = json!(rough);
        }
        materials.push(v);
    }
    let mut root = json!({
        "asset": {"version": "2.0", "generator": "Project Avalanche content export"},
        "scene": 0,
        "scenes": [{"nodes": roots}],
        "nodes": nodes,
        "meshes": meshes,
        "skins": skins,
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
