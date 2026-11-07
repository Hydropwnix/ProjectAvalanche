use std::path::Path;

use anyhow::{Context, Result};
use ssx_content::model::Mesh;
use ssx_content::model::collision::{CollisionTriangle, build};
use ssx_content::model::write::{WriteModel, glb};

fn manifest(name: &str) -> String {
    format!(
        r#"# The manifest of the package "{name}" (Project Avalanche; every key: Docs/architecture/content.md §2, the
# models, objects and materials: §13). Check it with `ssx-assets mod check <folder>`, pack it with `mod pack`.

[resource]
name = "{name}"
version = "0.1.0"
title = "{name}"
authors = []
description = ""
api = 1
type = "mod"
priority = 0

# What the package adds, in its own namespace ("{name}:<kind>/<path>").
[add]
# A glTF 2.0 model: the nodes are LOD 0, `<node>_lod1` … are the coarser LODs, everything below a node named
# `collision` is the collision (each mesh, its mesh or its material with the custom property `ava_surface`).
"model/prop" = "models/prop.glb"
# The material its glTF material "prop" is bound to (object.toml [materials]).
"material/prop" = "materials/prop.toml"
# The game object: the model, its LOD thresholds, its materials, collision and the record's other fields.
"object/prop" = "objects/prop.toml"

# Draw the object in a disc object's place, at that object's placements on every track (its collision too): name the
# disc object by id (`ssx-assets content ids --kind object` lists them; `ssx-assets content export <id>` writes one as
# glTF + toml from your own disc for your own reference — exported files may not be shipped, `mod check` refuses them).
# New placements on a track come with maps (phase M8).
[override]
# "ssx:object/rock_boulder_m_gameobject" = "objects/prop.toml"
"#
    )
}

fn object_toml(name: &str) -> String {
    format!(
        r#"# A game object (Docs/architecture/content.md §13.7).
# The model: its LODs and, unless `collision` names another model or "none", its `collision` node.
model = "{name}:model/prop"
# LODThresholds, one per LOD, LOD 0 first: the screen-size metric from which a LOD shows (the disc's props use e.g.
# [0.4, 0.2, 0.05]; 0.02 is a common last one).
lod_thresholds = [0.02]
# collision = "none"
# For parts whose glTF material is not bound below and has no name: a material id.
# material = "{name}:material/prop"
# A controller script: one of the game's own Lua scripts by id (ssx:script/…).
# controller = "ssx:script/scripts/..."

# glTF material name = material id (a package's material.toml or a disc material, ssx:material/…). An unbound glTF
# material is mapped onto the object family by the port's rule.
[materials]
"prop" = "{name}:material/prop"

# The audio object's type and sub-type (the disc's props: GenericObject).
# [audio]
# type = "GenericObject"
# sub_type = "Rock"

# A dynamic cylinder for a controlled object (the marker flares'): all of these.
# [rigid_body]
# offset = [0.0, 0.5, 0.0]
# axis = [0.0, 1.0, 0.0]
# half_height = 0.5
# radius = 0.1
# fat = 0.0
# inv_mass = 1.0
# inv_inertia = [12.0, 12.0, 50.0]
# max_speed = 100.0
# max_angular_speed = 50.0
# linear_damping = 0.1
# angular_damping = 0.1
# material = [0.2, 0.2, 0.1]
"#
    )
}

const MATERIAL_TOML: &str = r#"# A material of the object family (Docs/architecture/content.md §13.6).
family = "object"
# The game's effect: object_default, object_default_anim, object_alpha, object_alpha_anim, object_foliage,
# object_foliage_anim, object_advanced, object_texture_anim, object_shadow.
effect = "object_default"
# prelight (the default of the disc's materials), forward or translucent.
# technique = "prelight"
# Draw both sides whatever the effect's culling.
# double_sided = false

# The effect's textures by id: diffuseTexture, normalTexture, specularTexture (object_advanced also scatterNormalMap,
# sparkleNormalMap). A texture of this package: [add] "texture/<name>" = "textures/<file>".
[textures]
# diffuseTexture = "ssx:texture/worlds/textures/..."

# The effect's parameters: specularIntensity, specularEccentricity, glowEnable, glowIntensity, normalTile,
# specularTile, diffuseTile (vectors of 2 to 4 numbers), and object_advanced's snow parameters. A texture's tile
# multiplies its coordinates and is 0 when left out (one texel everywhere): set it with the texture.
[parameters]
specularIntensity = 0.2
specularEccentricity = 8.0
glowEnable = 0.0
glowIntensity = 0.0
# diffuseTile = [1.0, 1.0]
"#;

fn starter() -> Result<Vec<u8>> {
    let faces: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
        ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
        ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
        ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
        ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
    ];
    let mut mesh = Mesh { name: "prop".into(), ..Mesh::default() };
    let (mut normals, mut uv, mut colours, mut collision) = (vec![], vec![], vec![], vec![]);
    for (n, u, v) in faces {
        let base = mesh.positions.len() as u32;
        for (su, sv) in [(-0.5f32, -0.5f32), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)] {
            let p: [f32; 3] =
                std::array::from_fn(|k| n[k] * 0.5 + u[k] * su + v[k] * sv + if k == 1 { 0.5 } else { 0.0 });
            mesh.positions.push(p);
            normals.push(n);
            uv.push([su + 0.5, 0.5 - sv]);
            colours.push([255, 128, 255, 255]);
        }
        mesh.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        let c = |i: u32| mesh.positions[i as usize];
        for t in [[base, base + 1, base + 2], [base, base + 2, base + 3]] {
            collision.push(CollisionTriangle { corners: t.map(c), surface_id: 30, group: 0 });
        }
    }
    (mesh.normals, mesh.uv0, mesh.colours) = (Some(normals), Some(uv), Some(colours));
    let (collision, _) = build(&collision);
    let model = WriteModel {
        name: "prop",
        lods: vec![vec![(&mesh, Some(0))]],
        materials: vec!["prop".into()],
        collision: Some(&collision),
    };
    glb(&model).map_err(anyhow::Error::msg)
}

pub(super) fn lay_out(root: &Path, name: &str) -> Result<()> {
    for d in ["models", "objects", "materials"] {
        std::fs::create_dir(root.join(d)).with_context(|| format!("creating {d}"))?;
    }
    std::fs::write(root.join(ssx_content::manifest::MANIFEST_FILE), manifest(name))?;
    std::fs::write(root.join("models/prop.glb"), starter()?)?;
    std::fs::write(root.join("objects/prop.toml"), object_toml(name))?;
    std::fs::write(root.join("materials/prop.toml"), MATERIAL_TOML)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::{Template, new_package};

    #[test]
    fn the_prop_template_passes_the_package_check() {
        let out = std::env::temp_dir().join(format!("ssx-prop-template-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        let root = new_package("my-props", Template::Prop, &out, std::path::Path::new("")).unwrap();
        let c = ssx_content::check::check_package(&root, &ssx_content::texture::Limits::default());
        let errors: Vec<String> = c.findings.iter().map(ToString::to_string).filter(|f| f.contains("error")).collect();
        assert!(errors.is_empty(), "{errors:#?}");
        let m = &c.models[&"my-props:model/prop".parse().unwrap()];
        assert_eq!((m.lods.as_slice(), m.collision_triangles), (&[(1, 24, 12)][..], 12));
        assert_eq!(m.materials, [Some("prop".to_string())]);
        assert!(c.objects.contains_key(&"my-props:object/prop".parse().unwrap()));
        let _ = std::fs::remove_dir_all(&out);
    }
}
