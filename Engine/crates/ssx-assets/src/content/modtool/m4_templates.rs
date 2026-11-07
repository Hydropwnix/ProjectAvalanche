use std::path::Path;

use anyhow::Result;

fn manifest(name: &str, body: &str) -> String {
    format!(
        r#"# The manifest of the package "{name}" (Project Avalanche; every key: Docs/architecture/content.md §2, §15).
# Check it with `ssx-assets mod check <folder>`, pack it with `ssx-assets mod pack <folder>`.

[resource]
name = "{name}"
version = "0.1.0"
title = "{name}"
authors = []
description = ""
api = 1
type = "mod"
priority = 0
{body}"#
    )
}

fn face_png(side: u32, top: [u8; 3], bottom: [u8; 3]) -> Result<Vec<u8>> {
    let mut rgba = Vec::with_capacity((side * side * 4) as usize);
    for y in 0..side {
        let t = y as f32 / (side - 1).max(1) as f32;
        let c: Vec<u8> =
            (0..3).map(|k| (f32::from(top[k]) * (1.0 - t) + f32::from(bottom[k]) * t).round() as u8).collect();
        for _ in 0..side {
            rgba.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    let mut out = Vec::new();
    let mut e = png::Encoder::new(&mut out, side, side);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()?.write_image_data(&rgba)?;
    Ok(out)
}

pub(super) fn sky(root: &Path, name: &str) -> Result<()> {
    let body = r#"
# A sky of the package: a cubemap (six faces, `skies/dusk.toml`); also `cube = "x.ktx2"` (a KTX2 / DDS cube) or
# `equirect = "x.hdr"` (an equirectangular Radiance HDR or OpenEXR image), or `type = "dome"` with a model and a
# material of the sky family.
[add]
"sky/dusk" = "skies/dusk.toml"

# A lighting preset draws the sky its `sky` field names (default `ava:sky/scattering`, the game's own). This patch
# gives the disc's sunny day preset this package's sky; `ssx-assets content ids --kind lighting` lists the presets.
[patch]
"ssx:lighting/day_sunny_lighting" = "lighting/day_sunny.toml"
"#;
    std::fs::write(root.join(ssx_content::manifest::MANIFEST_FILE), manifest(name, body))?;
    std::fs::create_dir_all(root.join("skies"))?;
    std::fs::create_dir_all(root.join("lighting"))?;
    let (zenith, horizon, ground) = ([40, 30, 90], [230, 120, 70], [60, 50, 50]);
    for (file, top, bottom) in [
        ("px.png", zenith, horizon),
        ("nx.png", zenith, horizon),
        ("py.png", zenith, zenith),
        ("ny.png", ground, ground),
        ("pz.png", zenith, horizon),
        ("nz.png", zenith, horizon),
    ] {
        let png = if matches!(file, "py.png" | "ny.png") {
            face_png(64, top, bottom)?
        } else {
            side_face(zenith, horizon, ground)?
        };
        std::fs::write(root.join("skies").join(file), png)?;
    }
    std::fs::write(
        root.join("skies/dusk.toml"),
        r#"# A cubemap sky (Docs/architecture/content.md §15.1). Faces +X, -X, +Y, -Y, +Z, -Z.
type = "cubemap"
faces = ["skies/px.png", "skies/nx.png", "skies/py.png", "skies/ny.png", "skies/pz.png", "skies/nz.png"]
# how 8-bit faces are read: srgb (what image tools write) or linear
color = "srgb"
# linear multiplier and factor (the game's sky is about 1 to 20 in these units)
tint = [1.0, 1.0, 1.0]
exposure = 6.0
# degrees about the world's up axis
rotation = 0.0
# the game's sun (disc, moon or sun texture, glare), clouds and mid-ground mountains over the sky
sun = true
clouds = false
midground = true
# the materials' reflection cube from this sky (in place of the preset's)
reflection = false
"#,
    )?;
    std::fs::write(
        root.join("lighting/day_sunny.toml"),
        format!(
            "# A patch of the disc's preset: only the fields given change (Docs/architecture/content.md §15.2).\nsky = \"{name}:sky/dusk\"\n"
        ),
    )?;
    Ok(())
}

fn side_face(zenith: [u8; 3], horizon: [u8; 3], ground: [u8; 3]) -> Result<Vec<u8>> {
    let side = 64u32;
    let mut rgba = Vec::with_capacity((side * side * 4) as usize);
    for y in 0..side {
        let t = y as f32 / (side - 1) as f32;
        let (a, b, u) = if t < 0.5 { (zenith, horizon, t * 2.0) } else { (horizon, ground, (t - 0.5) * 2.0) };
        let c: Vec<u8> = (0..3).map(|k| (f32::from(a[k]) * (1.0 - u) + f32::from(b[k]) * u).round() as u8).collect();
        for _ in 0..side {
            rgba.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    let mut out = Vec::new();
    let mut e = png::Encoder::new(&mut out, side, side);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()?.write_image_data(&rgba)?;
    Ok(out)
}

pub(super) fn lighting(root: &Path, name: &str) -> Result<()> {
    let body = r#"
# Change fields of a disc lighting preset: a patch names only the fields it changes, by their decoded names
# (`ssx-assets content export ssx:lighting/<preset>` writes a whole preset to start from, for your own use).
[patch]
"ssx:lighting/day_sunny_lighting" = "lighting/day_sunny.toml"
"#;
    std::fs::write(root.join(ssx_content::manifest::MANIFEST_FILE), manifest(name, body))?;
    std::fs::create_dir_all(root.join("lighting"))?;
    std::fs::write(
        root.join("lighting/day_sunny.toml"),
        r#"# A patch of ssx:lighting/day_sunny_lighting (Docs/architecture/content.md §15.2): numbers, four numbers for
# vectors, texture ids for textures; a field unnamed on the disc as "0x<8 hex digits>".
# The sun's direction (dirLight_Direction, dirLight_Offset) and the night flag (isNightCondition) are simulation
# fields: a ride with them changed is not the base game's.
rampIntensity = 1.4
fog_color = [0.55, 0.6, 0.75, 1.0]
# atmosMieMultiplier = 2.0
# cloudDensityTexture = "ssx:texture/worlds/textures/<a cloud texture>"
# sky = "ava:sky/scattering"
"#,
    )?;
    Ok(())
}

pub(super) fn shader(root: &Path, name: &str) -> Result<()> {
    let body = r#"
# GPU code: a shader hook of the object family (WGSL). Declared here, so the Mods page says "contains GPU code";
# a server refuses a package with shader code (client mods only).
[shaders]
files = ["shaders/*.wgsl"]

[add]
"material/glow_wood" = "materials/glow_wood.toml"

# Draw the material in a disc material's place (the chalets' walls); or name it from your own object.toml.
[override]
"ssx:material/chalet_wood_c_mat" = "materials/glow_wood.toml"
"#;
    std::fs::write(root.join(ssx_content::manifest::MANIFEST_FILE), manifest(name, body))?;
    std::fs::create_dir_all(root.join("materials"))?;
    std::fs::create_dir_all(root.join("shaders"))?;
    std::fs::write(
        root.join("materials/glow_wood.toml"),
        r#"# An object material with a shader hook (Docs/architecture/content.md §13.5, §15.6).
family = "object"
effect = "object_default"
# the hook module: its `surface` and `vertex_offset` (both optional) run inside the family's shader
shader = "shaders/glow.wgsl"

[textures]
diffuseTexture = "ssx:texture/worlds/textures/chalet_wood_a_diff"
normalTexture = "ssx:texture/worlds/textures/chalet_wood_a_norm"

[parameters]
diffuseTile = [1.0, 1.0]
normalTile = [1.0, 1.0]
specularIntensity = 0.3
"#,
    )?;
    std::fs::write(root.join("shaders/glow.wgsl"), ssx_content::shader::OBJECT_TEMPLATE)?;
    Ok(())
}
