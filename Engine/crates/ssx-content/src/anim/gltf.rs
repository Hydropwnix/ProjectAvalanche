use std::collections::BTreeSet;

use gltf_json::Root;
use gltf_json::accessor::{ComponentType, Type};
use gltf_json::animation::{Interpolation as GltfInterpolation, Property};
use gltf_json::validation::Checked;
use ssx_core::text::{quoted, shown};

use super::{AnimError, Interpolation, JointTrack, Keys, MAX_JOINT_TRACKS, MAX_KEYS, MAX_NAME_BYTES, Path, check_keys};
use crate::model::{FileReader, Layout, read_f32, with_buffers};

const F32_SCALAR: Layout = (Type::Scalar, ComponentType::F32, false);
const F32_VEC3: Layout = (Type::Vec3, ComponentType::F32, false);
const ROTATION: [Layout; 5] = [
    (Type::Vec4, ComponentType::F32, false),
    (Type::Vec4, ComponentType::I8, true),
    (Type::Vec4, ComponentType::U8, true),
    (Type::Vec4, ComponentType::I16, true),
    (Type::Vec4, ComponentType::U16, true),
];

pub fn limits() -> crate::model::Limits {
    crate::model::Limits {
        max_file_bytes: 64 << 20,
        max_held_bytes: 64 << 20,
        max_data_uri_bytes: 16 << 20,
        ..crate::model::Limits::default()
    }
}

pub fn read_gltf(
    rel: &str,
    bytes: &[u8],
    files: &dyn FileReader,
    name: Option<&str>,
) -> Result<Vec<JointTrack>, AnimError> {
    let fail = |message: String| AnimError { file: rel.into(), line: 0, column: 0, message };
    with_buffers(rel, bytes, files, &limits(), |root, buffers| tracks(root, buffers, name))
        .map_err(|e| fail(e.message.clone()))
}

fn tracks(root: &Root, buffers: &[&[u8]], name: Option<&str>) -> Result<Vec<JointTrack>, String> {
    let index = match name {
        Some(n) => {
            let found: Vec<usize> =
                (0..root.animations.len()).filter(|&i| root.animations[i].name.as_deref() == Some(n)).collect();
            match found[..] {
                [i] => i,
                [] => return Err(format!("no animation is named {}", quoted(n))),
                _ => return Err(format!("{} animations are named {}", found.len(), quoted(n))),
            }
        }
        None => match root.animations.len() {
            1 => 0,
            0 => return Err("the file has no animation".into()),
            n => return Err(format!("the file has {n} animations: name one (`animation` in anim.toml)")),
        },
    };
    let anim = &root.animations[index];
    let at = format!("animations[{index}]");
    if anim.channels.is_empty() || anim.channels.len() > MAX_JOINT_TRACKS {
        return Err(format!("{at}: {} channels (1 to {MAX_JOINT_TRACKS})", anim.channels.len()));
    }
    let mut parent: Vec<Option<usize>> = vec![None; root.nodes.len()];
    for (p, n) in root.nodes.iter().enumerate() {
        for c in n.children.iter().flatten() {
            if let Some(slot) = parent.get_mut(c.value()) {
                *slot = Some(p);
            }
        }
    }
    let mut seen = BTreeSet::new();
    let mut out = Vec::with_capacity(anim.channels.len());
    for (ci, ch) in anim.channels.iter().enumerate() {
        let what = format!("{at}.channels[{ci}]");
        let node = ch.target.node.value();
        let n = root.nodes.get(node).ok_or_else(|| format!("{what}: nodes[{node}] does not exist"))?;
        let joint = match n.name.as_deref() {
            Some(j) if !j.is_empty() && j.len() <= MAX_NAME_BYTES && !j.chars().any(char::is_control) => j,
            Some(j) => {
                return Err(format!("{what}: node {}: a joint's name is 1 to {MAX_NAME_BYTES} bytes", quoted(j)));
            }
            None => return Err(format!("{what}: nodes[{node}] has no name (a channel drives a joint by its name)")),
        };
        if root.nodes.iter().enumerate().any(|(i, o)| i != node && o.name.as_deref() == Some(joint)) {
            return Err(format!("{what}: two nodes are named {}", quoted(joint)));
        }
        let path = match ch.target.path {
            Checked::Valid(Property::Rotation) => Path::Rotation,
            Checked::Valid(Property::Translation) => Path::Translation,
            Checked::Valid(Property::Scale) => Path::Scale,
            Checked::Valid(Property::MorphTargetWeights) => {
                return Err(format!("{what}: morph target weights (the game's clips drive joints and float DOFs)"));
            }
            Checked::Invalid => return Err(format!("{what}: an unknown target path")),
        };
        if !seen.insert((joint.to_string(), path)) {
            return Err(format!("{what}: {} {} has a channel already", quoted(joint), path.name()));
        }
        let s = anim.samplers.get(ch.sampler.value()).ok_or_else(|| format!("{what}: its sampler does not exist"))?;
        let interpolation = match s.interpolation {
            Checked::Valid(GltfInterpolation::Step) => Interpolation::Step,
            Checked::Valid(GltfInterpolation::Linear) => Interpolation::Linear,
            Checked::Valid(GltfInterpolation::CubicSpline) => Interpolation::CubicSpline,
            Checked::Invalid => return Err(format!("{what}: an unknown interpolation")),
        };
        let (times, _) = read_f32(root, buffers, s.input.value(), &[F32_SCALAR], MAX_KEYS as u64)
            .map_err(|e| format!("{what}: its times: {e}"))?;
        let per_key = interpolation.per_key();
        let accept: &[Layout] = if path == Path::Rotation { &ROTATION } else { &[F32_VEC3] };
        let max = (times.len() * per_key) as u64;
        let (values, width) =
            read_f32(root, buffers, s.output.value(), accept, max).map_err(|e| format!("{what}: its values: {e}"))?;
        if width != path.width() || values.len() != times.len() * per_key * width {
            return Err(format!(
                "{what}: {} values of {width} for {} keys ({} interpolation: {} values a key)",
                values.len() / width.max(1),
                times.len(),
                interpolation.name(),
                per_key
            ));
        }
        let keys = Keys { interpolation, times, values };
        check_keys(&keys, Some(path), &format!("{what} ({} {})", shown(joint), path.name()))?;
        let parent = parent[node].and_then(|p| root.nodes[p].name.clone());
        out.push(JointTrack { joint: joint.to_string(), path, keys, parent });
    }
    Ok(out)
}
