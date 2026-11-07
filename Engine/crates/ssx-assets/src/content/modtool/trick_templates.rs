use std::path::Path;

use anyhow::Result;
use ssx_content::trick::template::{ANIM_PATH, ANIM_TOML, GLTF_PATH, NAME, STRINGS_EN, TRICK_PATH, trick_toml};

pub const GLB: &[u8] = include_bytes!("../../../../../assets/templates/trick/anims/air_walk.glb");

fn manifest(name: &str) -> String {
    format!(
        r#"# The manifest of the package "{name}" (Project Avalanche; every key: Docs/architecture/content.md §2, §25).
# Check it with `ssx-assets mod check <folder>`, pack it with `ssx-assets mod pack <folder>`.

[resource]
name = "{name}"
version = "0.1.0"
title = "{name}"
authors = []
description = "An added trick: the Air Walk on up-up"
api = 1
type = "mod"
priority = 0

# An added trick (its record names its base grab, input, points, names and clips) and its clip (the package's own:
# a trick plays the clips of its package).
[add]
"trick/{NAME}" = "{TRICK_PATH}"
"anim/{NAME}" = "{ANIM_PATH}"
"#
    )
}

const README: &str = r#"# An added trick

`tricks/air_walk.toml` adds the Air Walk (Docs/architecture/content.md §25): it plays in the places of the disc's
Indy (`base`: its enter, loop, tweak and exit sequences, their timing, blends and landing windows, at each Tricky
level), with the package's clip `anim/air_walk` in all 12 of the Indy's clip places (`[trick.clips]`; give each
place a clip of its own for a finished trick). Its input is up-up — Y twice, or the right stick up twice — one of the
three the disc leaves free (up-up, left-left, right-right; in the goofy stance a left-left trick is right-right, as
every disc trick mirrors). Its points are its TRICK_POINTS (1 to 2400: the Indy's are 1000, the signature's 2400) and
its names are the package's strings (`strings/en.toml`), shown on the HUD as the disc's tricks' are.

A clip of your own: export a disc grab's clip (`ssx-assets content export ssx:anim/anim_all/1_trick_air_indy --out .`)
for the skeleton and the keys to start from, animate it, export the animation to glTF and name it in an `anim.toml`
(not additive: the grabs' clips are not). `ssx-assets mod check <folder>` binds each place's clip as the game does
and says what the clips leave out (this template's clip drives six joints; on the rest the pose of the layers below
shows through, as on a disc clip with fewer channels).

An added trick is simulation content (its poses and points reach the ride): in single player it applies, and a
result ridden with it carries the session's sim content id; a server whose policy is `client_mods = "presentation"`
switches it off. Its counts go into the profile's `<save>.mods.toml`, beside the disc's stats.

With `--mod-dev <folder>` the game reads a changed trick at the next ride start.
"#;

pub(super) fn trick(root: &Path, name: &str) -> Result<()> {
    std::fs::write(root.join(ssx_content::manifest::MANIFEST_FILE), manifest(name))?;
    for dir in ["anims", "tricks", "strings"] {
        std::fs::create_dir_all(root.join(dir))?;
    }
    std::fs::write(root.join(GLTF_PATH), GLB)?;
    std::fs::write(root.join(ANIM_PATH), ANIM_TOML)?;
    std::fs::write(root.join(TRICK_PATH), trick_toml(name))?;
    std::fs::write(root.join("strings").join("en.toml"), STRINGS_EN)?;
    std::fs::write(root.join("README.md"), README)?;
    Ok(())
}
