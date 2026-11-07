use gltf_json::Root;
use gltf_json::accessor::{ComponentType, Type};
use gltf_json::mesh::Semantic;
use gltf_json::validation::Checked;

use super::accessor::{F32_VEC4, read_f32, read_matrices};
use super::{Limits, MeshSkin, Model, ModelSkin};
use crate::skeleton::{RowMatrix, Skeleton, det3};

pub const WEIGHT_SUM_TOLERANCE: f32 = 1e-3;

pub const MAX_INFLUENCES: usize = 4;

const JOINT_LAYOUTS: [super::accessor::Layout; 2] =
    [(Type::Vec4, ComponentType::U8, false), (Type::Vec4, ComponentType::U16, false)];
const WEIGHT_LAYOUTS: [super::accessor::Layout; 3] =
    [F32_VEC4, (Type::Vec4, ComponentType::U8, true), (Type::Vec4, ComponentType::U16, true)];

pub fn read_skin(root: &Root, buffers: &[&[u8]], i: usize, limits: &Limits) -> Result<ModelSkin, String> {
    let what = format!("skins[{i}]");
    let s = &root.skins[i];
    let n = s.joints.len();
    if n == 0 || n > limits.max_skin_joints {
        return Err(format!("{what}: {n} joints (1 to {} are read)", limits.max_skin_joints));
    }
    let mut joints = Vec::with_capacity(n);
    let mut seen = std::collections::BTreeSet::new();
    for (k, j) in s.joints.iter().enumerate() {
        let node = &root.nodes[j.value()];
        let name = node.name.as_deref().filter(|n| !n.is_empty()).ok_or_else(|| {
            format!("{what}.joints[{k}] (nodes[{}]): a joint without a name (joints bind by name)", j.value())
        })?;
        if name.len() > 128 {
            return Err(format!("{what}.joints[{k}]: a joint name of {} bytes (at most 128)", name.len()));
        }
        if !seen.insert(j.value()) {
            return Err(format!("{what}: nodes[{}] is a joint twice", j.value()));
        }
        if node.mesh.is_some() {
            return Err(format!(
                "{what}.joints[{k}] {}: a joint node with a mesh (a part is skinned, not parented)",
                ssx_core::text::quoted(name)
            ));
        }
        joints.push(name.to_string());
    }
    let mut names: Vec<&String> = joints.iter().collect();
    names.sort();
    if let Some(w) = names.windows(2).find(|w| w[0] == w[1]) {
        return Err(format!("{what}: the joint name {} twice", ssx_core::text::quoted(w[0])));
    }
    let inverse_binds = match s.inverse_bind_matrices {
        Some(a) => read_matrices(root, buffers, a.value(), n as u64)?,
        None => vec![IDENTITY; n],
    };
    for (k, m) in inverse_binds.iter().enumerate() {
        if [m[0][3], m[1][3], m[2][3], m[3][3]] != [0.0, 0.0, 0.0, 1.0] {
            return Err(format!(
                "{what}: inverse bind matrix {k} ({}) is not affine (its fourth row)",
                ssx_core::text::quoted(&joints[k])
            ));
        }
        let det = det3(m);
        if !det.is_finite() || det.abs() < 1e-9 {
            return Err(format!(
                "{what}: inverse bind matrix {k} ({}) cannot be inverted (determinant {det:e})",
                ssx_core::text::quoted(&joints[k])
            ));
        }
    }
    Ok(ModelSkin { joints, inverse_binds })
}

const IDENTITY: RowMatrix = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];

pub fn read_influences(
    root: &Root,
    buffers: &[&[u8]],
    p: &gltf_json::mesh::Primitive,
    what: &str,
    vertices: usize,
    joint_count: usize,
    max: u64,
) -> Result<MeshSkin, String> {
    let attr = |s: Semantic| p.attributes.get(&Checked::Valid(s)).map(|a| a.value());
    let mut sets = Vec::new();
    for set in 0.. {
        match (attr(Semantic::Joints(set)), attr(Semantic::Weights(set))) {
            (None, None) if set > 0 => break,
            (Some(j), Some(w)) => sets.push((set, j, w)),
            (None, None) => return Err(format!("{what}: a skinned mesh without JOINTS_0 and WEIGHTS_0")),
            _ => return Err(format!("{what}: JOINTS_{set} and WEIGHTS_{set} come in pairs")),
        }
        if set > 7 {
            return Err(format!("{what}: more than eight sets of joints and weights"));
        }
    }
    let mut joints = vec![[0u16; 4]; vertices];
    let mut weights = vec![[0f32; 4]; vertices];
    let mut sums = vec![0f64; vertices];
    let mut int_sums = vec![0u32; vertices];
    let mut kinds = std::collections::BTreeSet::new();
    for (set, ja, wa) in sets {
        let (j, _) = read_f32(root, buffers, ja, &JOINT_LAYOUTS, max)?;
        let (w, _) = read_f32(root, buffers, wa, &WEIGHT_LAYOUTS, max)?;
        let wc = match &root.accessors[wa].component_type {
            Checked::Valid(c) => c.0,
            Checked::Invalid => return Err(format!("{what}: WEIGHTS_{set} of an unknown component type")),
        };
        kinds.insert(match wc {
            ComponentType::U8 => 255u32,
            ComponentType::U16 => 65535,
            _ => 0,
        });
        if j.len() != vertices * 4 || w.len() != vertices * 4 {
            return Err(format!("{what}: JOINTS_{set} / WEIGHTS_{set} do not have one element per vertex"));
        }
        for v in 0..vertices {
            for k in 0..4 {
                let (index, weight) = (j[v * 4 + k], w[v * 4 + k]);
                if index as usize >= joint_count {
                    return Err(format!(
                        "{what}: vertex {v} names joint {index} of the skin, which has {joint_count} (JOINTS_{set})"
                    ));
                }
                if weight < 0.0 {
                    return Err(format!("{what}: vertex {v} has the negative weight {weight} (WEIGHTS_{set})"));
                }
                if weight == 0.0 {
                    continue;
                }
                if set > 0 {
                    return Err(format!(
                        "{what}: vertex {v} has more than {MAX_INFLUENCES} influences (a non-zero WEIGHTS_{set}); \
                         the riders' skins have at most {MAX_INFLUENCES}"
                    ));
                }
                let index = index as u16;
                if joints[v][..k].iter().zip(&weights[v][..k]).any(|(&i, &x)| i == index && x != 0.0) {
                    return Err(format!("{what}: vertex {v} names joint {index} twice with a weight"));
                }
                (joints[v][k], weights[v][k]) = (index, weight);
                sums[v] += f64::from(weight);
                if wc != ComponentType::F32 {
                    int_sums[v] += (weight * if wc == ComponentType::U8 { 255.0 } else { 65535.0 }).round() as u32;
                }
            }
        }
    }
    if kinds.len() > 1 {
        return Err(format!("{what}: weight sets of different component types"));
    }
    let integer = kinds.iter().next().copied().unwrap_or(0);
    for v in 0..vertices {
        if integer > 0 {
            if int_sums[v] != integer {
                return Err(format!(
                    "{what}: vertex {v}'s normalised weights sum to {}/{integer}; glTF requires exactly {integer}",
                    int_sums[v]
                ));
            }
            continue;
        }
        let s = sums[v];
        if s == 0.0 {
            return Err(format!("{what}: vertex {v} has no influence (all its weights are 0)"));
        }
        if (s - 1.0).abs() > f64::from(WEIGHT_SUM_TOLERANCE) {
            return Err(format!(
                "{what}: vertex {v}'s weights sum to {s:.6}; at most {WEIGHT_SUM_TOLERANCE:e} from 1 is read"
            ));
        }
        for x in &mut weights[v] {
            *x = (f64::from(*x) / s) as f32;
        }
    }
    Ok(MeshSkin { joints, weights })
}

pub fn bind_skin(skin: &ModelSkin, skeleton: &Skeleton) -> Result<Vec<usize>, Vec<String>> {
    let mut problems = Vec::new();
    let mut palette = Vec::with_capacity(skin.joints.len());
    for (name, ibm) in skin.joints.iter().zip(&skin.inverse_binds) {
        match skeleton.find(name) {
            Some(j) => {
                if let Some(why) = skeleton.bind_mismatch(j, ibm) {
                    problems.push(why);
                }
                palette.push(j);
            }
            None => problems.push(format!(
                "the joint {} is not a joint of the riders' skeleton ({} joints, by name)",
                super::uri::quoted(name),
                skeleton.len()
            )),
        }
    }
    if problems.is_empty() { Ok(palette) } else { Err(problems) }
}

pub fn bind(model: &Model, skeleton: &Skeleton) -> Result<Vec<Vec<usize>>, Vec<String>> {
    let mut problems = Vec::new();
    let mut out = Vec::with_capacity(model.skins.len());
    for (s, skin) in model.skins.iter().enumerate() {
        match bind_skin(skin, skeleton) {
            Ok(p) => out.push(p),
            Err(ps) => problems.extend(ps.into_iter().map(|p| format!("skins[{s}]: {p}"))),
        }
    }
    if problems.is_empty() { Ok(out) } else { Err(problems) }
}
