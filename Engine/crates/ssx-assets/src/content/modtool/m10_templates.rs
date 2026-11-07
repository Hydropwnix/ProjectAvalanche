use std::path::Path;

use anyhow::Result;
use ssx_content::anim::template::{ANIM_TOML, GLTF_PATH, TARGET};

pub const GLB: &[u8] = include_bytes!("../../../../../assets/templates/anim/anims/carve_sway.glb");
pub const TOML_PATH: &str = "anims/carve_sway.toml";

fn manifest(name: &str) -> String {
    format!(
        r#"# The manifest of the package "{name}" (Project Avalanche; every key: Docs/architecture/content.md §2, §23).
# Check it with `ssx-assets mod check <folder>`, pack it with `ssx-assets mod pack <folder>`.

[resource]
name = "{name}"
version = "0.1.0"
title = "{name}"
authors = []
description = "A stronger sway of the spine, neck, head and arms while carving"
api = 1
type = "mod"
priority = 0

# A clip replaces a disc clip of the riders' body by its id (`ssx-assets content ids --kind anim`) in every graph that
# plays it; `ssx-assets content export <id>` writes one from your disc to start from (for your own use).
[override]
"{TARGET}" = "{TOML_PATH}"
"#
    )
}

const README: &str = r#"# A clip package

`anims/carve_sway.toml` replaces `ssx:anim/anim_all/carv_regular_noise`, the additive noise the riding graph layers
on the regular carve (Docs/architecture/content.md §23): its joint tracks come from the animation `carve_sway` of
`anims/carve_sway.glb` — a sway of `Spine2`, `Neck`, `Head`, `LeftArm` and `RightArm`, 61 keys over 3 s. The glTF
names its joints as the riders' skeleton names them, each under the parent the skeleton gives it; the keys are
offsets (`additive = true`), as the clip it replaces is additive.

A clip of your own: export a disc clip (`ssx-assets content export ssx:anim/anim_all/carv_regular_lean --out .`)
for the skeleton and the keys to start from, animate it in a tool of your choice, export the animation to glTF and
name it in an `anim.toml`. `ssx-assets mod check <folder>` binds it as the game does: every joint a joint of the
riders' rig with the DOF of its path, every float track a float DOF (`[[anim.float]]`, named `<layout>.<name>`), the
same `additive` as the clip it replaces; it says which DOFs of the disc clip yours leaves alone and how its length
compares with the controller's.

A rider's clip moves the simulation (the graph's pose feeds the ride): in single player it applies, and a result
ridden with it carries the session's sim content id; a server whose policy is `client_mods = "presentation"`
switches it off.

With `--mod-dev <folder>` the game reads a changed clip at the next ride start.
"#;

pub(super) fn anim(root: &Path, name: &str) -> Result<()> {
    std::fs::write(root.join(ssx_content::manifest::MANIFEST_FILE), manifest(name))?;
    std::fs::create_dir_all(root.join("anims"))?;
    std::fs::write(root.join(GLTF_PATH), GLB)?;
    std::fs::write(root.join(TOML_PATH), ANIM_TOML)?;
    std::fs::write(root.join("README.md"), README)?;
    Ok(())
}
