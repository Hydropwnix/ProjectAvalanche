use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::{Clip, Interpolation, Path};
use crate::model::assemble_glb;
use crate::model::write::Bin;
use crate::model::write_skinned::WriteJoint;

fn interpolation(i: Interpolation) -> &'static str {
    match i {
        Interpolation::Step => "STEP",
        Interpolation::Linear => "LINEAR",
        Interpolation::CubicSpline => "CUBICSPLINE",
    }
}

pub fn glb(joints: &[WriteJoint], name: &str, clip: &Clip) -> Result<Vec<u8>, String> {
    let mut by_name = BTreeMap::new();
    for (i, j) in joints.iter().enumerate() {
        if j.parent.is_some_and(|p| p >= i) {
            return Err(format!("joint {}: its parent does not come before it", ssx_core::text::quoted(&j.name)));
        }
        if j.translation.iter().chain(&j.rotation).chain(&j.scale).any(|v| !v.is_finite()) {
            return Err(format!("joint {}: a value that is not finite", ssx_core::text::quoted(&j.name)));
        }
        if by_name.insert(j.name.as_str(), i).is_some() {
            return Err(format!("two joints are named {}", ssx_core::text::quoted(&j.name)));
        }
    }
    let mut nodes: Vec<Value> = joints
        .iter()
        .map(|j| json!({"name": j.name, "translation": j.translation, "rotation": j.rotation, "scale": j.scale}))
        .collect();
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); joints.len()];
    let mut roots = Vec::new();
    for (i, j) in joints.iter().enumerate() {
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
    let mut bin = Bin::default();
    let (mut channels, mut samplers) = (Vec::new(), Vec::new());
    for t in &clip.joints {
        let node = *by_name
            .get(t.joint.as_str())
            .ok_or_else(|| format!("no joint node {}", ssx_core::text::quoted(&t.joint)))?;
        let (lo, hi) = (t.keys.times.first().copied(), t.keys.times.last().copied());
        let (Some(lo), Some(hi)) = (lo, hi) else { return Err(format!("{} {}: no keys", t.joint, t.path.name())) };
        let n = t.keys.times.len();
        if t.keys.values.len() != n * t.path.width() * t.keys.interpolation.per_key() {
            return Err(format!("{} {}: values for another count of keys", t.joint, t.path.name()));
        }
        let input = bin.floats(t.keys.times.iter().copied(), n, "SCALAR", json!({"min": [lo], "max": [hi]}));
        let kind = if t.path == Path::Rotation { "VEC4" } else { "VEC3" };
        let count = n * t.keys.interpolation.per_key();
        let output = bin.floats(t.keys.values.iter().copied(), count, kind, json!({}));
        samplers.push(json!({"input": input, "output": output, "interpolation": interpolation(t.keys.interpolation)}));
        channels.push(json!({"sampler": samplers.len() - 1, "target": {"node": node, "path": t.path.name()}}));
    }
    if channels.is_empty() {
        return Err("the clip has no joint track".into());
    }
    let root = json!({
        "asset": {"version": "2.0", "generator": "Project Avalanche content export"},
        "scene": 0,
        "scenes": [{"nodes": roots}],
        "nodes": nodes,
        "animations": [{"name": name, "channels": channels, "samplers": samplers}],
        "accessors": bin.accessors,
        "bufferViews": bin.views,
        "buffers": [{"byteLength": bin.data.len()}],
    });
    let json = serde_json::to_vec(&root).map_err(|e| e.to_string())?;
    Ok(assemble_glb(&json, Some(&bin.data)))
}

pub fn anim_toml(clip: &Clip, gltf: Option<&str>, animation: &str, header: &str) -> String {
    let mut s = String::from(header);
    s.push_str("[anim]\n");
    if let Some(g) = gltf {
        s.push_str(&format!("gltf = {}\n", toml::Value::String(g.into())));
        s.push_str(&format!("animation = {}\n", toml::Value::String(animation.into())));
    }
    s.push_str(&format!("additive = {}\n", clip.additive));
    for f in &clip.floats {
        let list = |xs: &[f32]| xs.iter().map(|x| format!("{x:?}")).collect::<Vec<_>>();
        s.push_str("\n[[anim.float]]\n");
        s.push_str(&format!("dof = {}\n", toml::Value::String(f.dof.clone())));
        s.push_str(&format!("interpolation = \"{}\"\n", f.keys.interpolation.name()));
        s.push_str(&format!("times = [{}]\n", list(&f.keys.times).join(", ")));
        s.push_str(&format!("values = [{}]\n", list(&f.keys.values).join(", ")));
    }
    s
}
