use gltf_json::Root;
use gltf_json::validation::Validate;

use super::Limits;

pub fn depth(json: &[u8]) -> usize {
    let (mut depth, mut max, mut in_string, mut escaped) = (0usize, 0usize, false, false);
    for &b in json {
        if in_string {
            match b {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' | b'[' => {
                depth += 1;
                max = max.max(depth);
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    max
}

pub fn parse(json: &[u8], limits: &Limits) -> Result<Root, String> {
    if json.len() as u64 > limits.max_json_bytes {
        return Err(format!("the JSON has {} bytes; at most {} are read", json.len(), limits.max_json_bytes));
    }
    let d = depth(json);
    if d > limits.max_json_depth {
        return Err(format!("the JSON is nested {d} deep; at most {} is read", limits.max_json_depth));
    }
    super::shape::check(json, limits)?;
    let root: Root =
        serde_json::from_slice(json).map_err(|e| format!("JSON: {}", ssx_core::text::message(&e.to_string())))?;
    let counts = [
        ("accessors", root.accessors.len()),
        ("animations", root.animations.len()),
        ("buffers", root.buffers.len()),
        ("bufferViews", root.buffer_views.len()),
        ("cameras", root.cameras.len()),
        ("images", root.images.len()),
        ("materials", root.materials.len()),
        ("meshes", root.meshes.len()),
        ("nodes", root.nodes.len()),
        ("samplers", root.samplers.len()),
        ("scenes", root.scenes.len()),
        ("skins", root.skins.len()),
        ("textures", root.textures.len()),
    ];
    for (name, n) in counts {
        if n > limits.max_items {
            return Err(format!("{n} {name}; at most {} are read", limits.max_items));
        }
    }
    let primitives: usize = root.meshes.iter().map(|m| m.primitives.len()).sum();
    if primitives > limits.max_primitives {
        return Err(format!("{primitives} mesh primitives; at most {} are read", limits.max_primitives));
    }
    let major = root.asset.version.split('.').next().unwrap_or("");
    if major != "2" {
        return Err(format!("glTF version {}; only 2.x is read", ssx_core::text::quoted(&root.asset.version)));
    }
    if let Some(min) = &root.asset.min_version
        && min != "2.0"
    {
        return Err(format!("needs glTF {}; this reader is 2.0", ssx_core::text::quoted(min)));
    }
    trees(&root)?;
    if !root.extensions_required.is_empty() {
        return Err(format!(
            "requires the extensions {}; none is supported",
            ssx_core::text::quoted_list(&root.extensions_required)
        ));
    }
    for (m, mesh) in root.meshes.iter().enumerate() {
        for (p, prim) in mesh.primitives.iter().enumerate() {
            if let Some((name, i)) = prim.attributes.iter().find(|(_, i)| i.value() >= root.accessors.len()) {
                let name = match name {
                    gltf_json::validation::Checked::Valid(s) => s.to_string(),
                    gltf_json::validation::Checked::Invalid => "(unknown)".to_string(),
                };
                return Err(format!(
                    "invalid glTF: meshes[{m}].primitives[{p}].attributes[\"{name}\"] = {i}: IndexOutOfBounds ({} accessors)",
                    root.accessors.len()
                ));
            }
        }
    }
    let mut errors = Vec::new();
    root.validate(&root, gltf_json::Path::new, &mut |path, error| {
        if errors.len() < 8 {
            errors.push(format!("{}: {error:?}", path()));
        }
    });
    if !errors.is_empty() {
        return Err(format!("invalid glTF: {}", errors.join("; ")));
    }
    Ok(root)
}

fn trees(root: &Root) -> Result<(), String> {
    let nodes = root.nodes.len();
    let children: usize = root.nodes.iter().map(|n| n.children.as_ref().map_or(0, Vec::len)).sum();
    if children > nodes {
        return Err(format!(
            "the nodes' children lists hold {children} entries together, more than there are nodes ({nodes})"
        ));
    }
    if let Some((i, s)) = root.scenes.iter().enumerate().find(|(_, s)| s.nodes.len() > nodes) {
        return Err(format!("scenes[{i}].nodes: {} entries, more than there are nodes ({nodes})", s.nodes.len()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_walks_premise_is_checked_on_the_parsed_root() {
        let root = |text: &str| -> Root { serde_json::from_str(text).expect("a glTF root") };
        let ok = root(r#"{"asset":{"version":"2.0"},"scenes":[{"nodes":[0]}],"nodes":[{"children":[1]},{}]}"#);
        assert_eq!(trees(&ok), Ok(()));
        let wide = root(r#"{"asset":{"version":"2.0"},"nodes":[{"children":[1,1,1]},{}]}"#);
        assert!(trees(&wide).unwrap_err().contains("hold 3 entries together, more than there are nodes (2)"));
        let scene = root(r#"{"asset":{"version":"2.0"},"scenes":[{"nodes":[0,0,0]}],"nodes":[{},{}]}"#);
        assert!(trees(&scene).unwrap_err().contains("scenes[0].nodes: 3 entries, more than there are nodes (2)"));
    }
}
