use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use ssx_content::model::Mesh;
use ssx_content::model::write_skinned::{WriteJoint, WriteMaterial, WritePart, WriteSkinned, glb};

use crate::character::{Srt, model_matrices};
use crate::content::GameContent;
use crate::content::characters::skeleton::RiderSkeleton;

type V3 = [f32; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: V3, s: f32) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn unit(a: V3) -> Option<V3> {
    let l = dot(a, a).sqrt();
    (l > 1e-6 && l.is_finite()).then(|| scale(a, 1.0 / l))
}

#[derive(Default)]
struct Built {
    mesh: Mesh,
    joints: Vec<[usize; 4]>,
    weights: Vec<[f32; 4]>,
}

impl Built {
    fn quad(&mut self, corners: [V3; 4], normal: V3, skin: [([usize; 4], [f32; 4]); 4]) {
        let base = self.mesh.positions.len() as u32;
        for (k, (c, uv)) in corners.iter().zip([[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]).enumerate() {
            self.mesh.positions.push(*c);
            self.mesh.normals.get_or_insert_with(Vec::new).push(normal);
            self.mesh.uv0.get_or_insert_with(Vec::new).push(uv);
            self.joints.push(skin[k].0);
            self.weights.push(skin[k].1);
        }
        self.mesh.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    fn bone(&mut self, a: V3, b: V3, r: f32, joint: usize) {
        let Some(axis) = unit(sub(b, a)) else { return };
        let helper = if axis[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] };
        let (Some(u), Some(v)) = (unit(cross(axis, helper)), unit(cross(axis, cross(axis, helper)))) else { return };
        let (a, b) = (sub(a, scale(axis, r)), add(b, scale(axis, r)));
        let corner = |end: V3, su: f32, sv: f32| add(end, add(scale(u, su * r), scale(v, sv * r)));
        let on = ([joint, 0, 0, 0], [1.0, 0.0, 0.0, 0.0]);
        let ring = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];
        for i in 0..4 {
            let (p, q) = (ring[i], ring[(i + 1) % 4]);
            let n = unit(add(scale(u, p.0 + q.0), scale(v, p.1 + q.1))).unwrap_or(u);
            let corners = [corner(a, p.0, p.1), corner(a, q.0, q.1), corner(b, q.0, q.1), corner(b, p.0, p.1)];
            self.quad(facing(corners, n), n, [on; 4]);
        }
        for (end, n) in [(a, scale(axis, -1.0)), (b, axis)] {
            self.quad(facing(ring.map(|(su, sv)| corner(end, su, sv)), n), n, [on; 4]);
        }
    }
}

fn facing(mut c: [V3; 4], n: V3) -> [V3; 4] {
    if dot(n, cross(sub(c[1], c[0]), sub(c[2], c[0]))) < 0.0 {
        c.reverse();
    }
    c
}

struct Bones<'a> {
    sk: &'a RiderSkeleton,
    world: Vec<V3>,
}

impl<'a> Bones<'a> {
    fn new(sk: &'a RiderSkeleton) -> Bones<'a> {
        let world = model_matrices(&sk.rig, &sk.bind).iter().map(|m| [m[3][0], m[3][1], m[3][2]]).collect();
        Bones { sk, world }
    }

    fn at(&self, name: &str) -> Result<(usize, V3)> {
        let i = self.sk.skeleton.find(name).with_context(|| format!("the riders' skeleton has no joint {name:?}"))?;
        Ok((i, self.world[i]))
    }

    fn joints(&self) -> Vec<WriteJoint> {
        self.sk
            .rig
            .nodes
            .iter()
            .zip(&self.sk.bind)
            .map(|(n, s): (_, &Srt)| WriteJoint {
                name: n.name.clone(),
                parent: n.parent,
                translation: s.translation,
                rotation: ssx_formats::ant_anim::normalize(s.rotation),
                scale: s.scale,
            })
            .collect()
    }
}

const BONES: &[(&str, &str, &str, f32)] = &[
    ("head", "Neck", "Head", 0.05),
    ("head", "Head", "HeadEnd", 0.1),
    ("jackets", "Hips", "Spine1", 0.13),
    ("jackets", "Spine1", "Spine3", 0.14),
    ("jackets", "Spine3", "Neck", 0.12),
    ("jackets", "LeftShoulder", "LeftArm", 0.06),
    ("jackets", "LeftArm", "LeftForeArm", 0.055),
    ("jackets", "LeftForeArm", "LeftHand", 0.045),
    ("jackets", "RightShoulder", "RightArm", 0.06),
    ("jackets", "RightArm", "RightForeArm", 0.055),
    ("jackets", "RightForeArm", "RightHand", 0.045),
    ("pants", "LeftUpLeg", "LeftLeg", 0.08),
    ("pants", "LeftLeg", "LeftFoot", 0.06),
    ("pants", "RightUpLeg", "RightLeg", 0.08),
    ("pants", "RightLeg", "RightFoot", 0.06),
    ("gloves", "LeftHand", "RM_LeftHandMiddle3", 0.04),
    ("gloves", "RightHand", "RM_RightHandMiddle3", 0.04),
    ("boots", "LeftFoot", "LeftToeBase", 0.055),
    ("boots", "LeftToeBase", "LeftFootEnd", 0.045),
    ("boots", "RightFoot", "RightToeBase", 0.055),
    ("boots", "RightToeBase", "RightFootEnd", 0.045),
];

const COLOURS: &[(&str, [f32; 4])] = &[
    ("head", [0.80, 0.62, 0.50, 1.0]),
    ("jackets", [0.10, 0.45, 0.80, 1.0]),
    ("pants", [0.15, 0.15, 0.20, 1.0]),
    ("gloves", [0.85, 0.20, 0.15, 1.0]),
    ("boots", [0.30, 0.25, 0.20, 1.0]),
    ("snowboard", [0.95, 0.75, 0.10, 1.0]),
];

fn mannequin(b: &Bones, slots: &[&str]) -> Result<Vec<(String, Built)>> {
    let mut parts: BTreeMap<&str, Built> = BTreeMap::new();
    for &(slot, from, to, r) in BONES.iter().filter(|(s, ..)| slots.contains(s)) {
        let ((j, a), (_, z)) = (b.at(from)?, b.at(to)?);
        parts.entry(slot).or_default().bone(a, z, r, j);
    }
    Ok(parts.into_iter().map(|(s, p)| (s.to_string(), p)).collect())
}

const BOARD_JOINTS: [&str; 11] = [
    "Board",
    "RM_RightBoardFlex2",
    "RM_RightBoardFlex3",
    "RM_RightBoardFlex4",
    "RM_RightBoardFlex5",
    "RightBoardFlex6",
    "RM_LeftBoardFlex2",
    "RM_LeftBoardFlex3",
    "RM_LeftBoardFlex4",
    "RM_LeftBoardFlex5",
    "LeftBoardFlex6",
];

fn board(b: &Bones) -> Result<Built> {
    let joints: Vec<(usize, V3)> = BOARD_JOINTS.iter().map(|n| b.at(n)).collect::<Result<_>>()?;
    let centre = joints[0].1;
    let (Some(&(_, tail)), Some(&(_, nose))) = (joints.get(5), joints.get(10)) else { anyhow::bail!("board joints") };
    let along = unit(sub(nose, tail)).context("the board's flex joints coincide")?;
    let up = unit(sub([0.0, 1.0, 0.0], scale(along, along[1]))).context("the board stands upright")?;
    let side = cross(along, up);
    let at = |p: V3| dot(sub(p, centre), along);
    let mut stops: Vec<(f32, usize)> = joints.iter().map(|&(j, p)| (at(p), j)).collect();
    stops.sort_by(|x, y| x.0.total_cmp(&y.0));
    let (lo, hi) = (stops[0].0 - 0.08, stops[stops.len() - 1].0 + 0.08);
    let skin = |t: f32| -> ([usize; 4], [f32; 4]) {
        let k = stops.iter().position(|s| s.0 >= t).unwrap_or(stops.len());
        if k == 0 || k == stops.len() {
            let j = stops[k.min(stops.len() - 1)].1;
            return ([j, 0, 0, 0], [1.0, 0.0, 0.0, 0.0]);
        }
        let (a, z) = (stops[k - 1], stops[k]);
        let w = ((t - a.0) / (z.0 - a.0).max(1e-4)).clamp(0.0, 1.0);
        if a.1 == z.1 { ([a.1, 0, 0, 0], [1.0, 0.0, 0.0, 0.0]) } else { ([a.1, z.1, 0, 0], [1.0 - w, w, 0.0, 0.0]) }
    };
    let (half_w, thick, n) = (0.13f32, 0.015f32, 24usize);
    let point = |t: f32, s: f32, h: f32| add(centre, add(scale(along, t), add(scale(side, s), scale(up, h))));
    let mut out = Built::default();
    for i in 0..n {
        let (t0, t1) = (lo + (hi - lo) * i as f32 / n as f32, lo + (hi - lo) * (i + 1) as f32 / n as f32);
        let (s0, s1) = (skin(t0), skin(t1));
        let faces = [
            ([point(t0, -half_w, 0.0), point(t1, -half_w, 0.0), point(t1, half_w, 0.0), point(t0, half_w, 0.0)], up),
            (
                [
                    point(t0, -half_w, -thick),
                    point(t0, half_w, -thick),
                    point(t1, half_w, -thick),
                    point(t1, -half_w, -thick),
                ],
                scale(up, -1.0),
            ),
            (
                [point(t0, half_w, -thick), point(t0, half_w, 0.0), point(t1, half_w, 0.0), point(t1, half_w, -thick)],
                side,
            ),
            (
                [
                    point(t0, -half_w, -thick),
                    point(t1, -half_w, -thick),
                    point(t1, -half_w, 0.0),
                    point(t0, -half_w, 0.0),
                ],
                scale(side, -1.0),
            ),
        ];
        for (corners, normal) in faces {
            let c = facing(corners, normal);
            let w = c.map(|p| if at(p) < (t0 + t1) / 2.0 { s0 } else { s1 });
            out.quad(c, normal, w);
        }
    }
    for (t, normal) in [(lo, scale(along, -1.0)), (hi, along)] {
        let c = facing(
            [point(t, -half_w, -thick), point(t, half_w, -thick), point(t, half_w, 0.0), point(t, -half_w, 0.0)],
            normal,
        );
        out.quad(c, normal, [skin(t); 4]);
    }
    Ok(out)
}

fn write_part<'a>(sk: &RiderSkeleton, slot: &'a str, p: &'a Built, material: usize) -> WritePart<'a> {
    let mut palette: Vec<usize> = p
        .joints
        .iter()
        .zip(&p.weights)
        .flat_map(|(j, w)| j.iter().zip(w).filter(|(_, w)| **w > 0.0).map(|(j, _)| *j))
        .collect();
    palette.sort_unstable();
    palette.dedup();
    let index = |j: usize, w: f32| if w > 0.0 { palette.iter().position(|&x| x == j).unwrap_or(0) as u16 } else { 0 };
    let joints = p.joints.iter().zip(&p.weights).map(|(j, w)| std::array::from_fn(|i| index(j[i], w[i]))).collect();
    WritePart {
        slot,
        mesh: &p.mesh,
        material: Some(material),
        inverse_binds: palette.iter().map(|&j| sk.skeleton.joints[j].inverse_bind).collect(),
        palette,
        joints: std::borrow::Cow::Owned(joints),
        weights: &p.weights,
    }
}

fn model_glb(b: &Bones, parts: &[(String, Built)]) -> Result<Vec<u8>> {
    let joints = b.joints();
    let materials = parts
        .iter()
        .map(|(slot, _)| WriteMaterial {
            name: slot.clone(),
            base_colour: COLOURS.iter().find(|(s, _)| s == slot).map(|(_, c)| *c),
            metallic_roughness: Some((0.0, 0.8)),
        })
        .collect();
    let write = parts.iter().enumerate().map(|(i, (slot, p))| write_part(b.sk, slot, p, i)).collect();
    glb(&WriteSkinned { joints: &joints, parts: write, materials }).map_err(anyhow::Error::msg)
}

fn portrait_png() -> Result<Vec<u8>> {
    let (w, h) = (88u32, 256u32);
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let t = y as f32 / (h - 1) as f32;
        for x in 0..w {
            let band = if (x as f32 - w as f32 / 2.0).abs() < 18.0 && (40..200).contains(&y) { 40.0 } else { 0.0 };
            let c = |a: f32, b: f32| (a + (b - a) * t + band).clamp(0.0, 255.0) as u8;
            data.extend([c(40.0, 10.0), c(120.0, 60.0), c(200.0, 120.0), 255]);
        }
    }
    let mut out = Vec::new();
    let mut e = png::Encoder::new(&mut out, w, h);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()?.write_image_data(&data)?;
    Ok(out)
}

pub(super) fn skeleton(data: &Path) -> Result<std::sync::Arc<RiderSkeleton>> {
    let big = crate::setup::big_root(data);
    GameContent::of(&big).riders.skeletons.get(&big, None).with_context(|| {
        format!(
            "the riders' skeleton could not be read from {} — the template's model is skinned to it, read from your own \
             disc (run `ssx-assets setup` first, or name the data folder with --data)",
            data.display()
        )
    })
}

fn header(name: &str, what: &str) -> String {
    format!(
        r#"# The manifest of the package "{name}" (Project Avalanche; every key: Docs/architecture/content.md §2, characters,
# gear and boards: §17). Check it with `ssx-assets mod check <folder>`, pack it with `mod pack`.
# {what}

[resource]
name = "{name}"
version = "0.1.0"
title = "{name}"
authors = []
description = ""
api = 1
type = "mod"
priority = 0
"#
    )
}

fn display_name(name: &str) -> String {
    let mut s: String = name.chars().filter(char::is_ascii_alphanumeric).take(32).collect();
    if !s.starts_with(|c: char| c.is_ascii_alphabetic()) {
        s = format!("Rider{s}").chars().take(32).collect();
    }
    let mut c = s.chars();
    c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_else(|| "Rider".into())
}

pub(super) fn character(root: &Path, name: &str, data: &Path) -> Result<()> {
    let sk = skeleton(data)?;
    let b = Bones::new(&sk);
    let parts = mannequin(&b, &["head", "jackets", "pants", "gloves", "boots"])?;
    for d in ["characters", "textures/portraits"] {
        std::fs::create_dir_all(root.join(d)).with_context(|| format!("creating {d}"))?;
    }
    std::fs::write(root.join(format!("characters/{name}.glb")), model_glb(&b, &parts)?)?;
    std::fs::write(root.join(format!("textures/portraits/{name}.png")), portrait_png()?)?;
    let shown = display_name(name);
    let manifest = format!(
        r#"{}
[add]
# The rider: its model (a skinned glTF on the riders' skeleton, one node per boarder slot — head, hair, eyes, headgear,
# jackets, pants, gloves, boots, goggles, headlamp, protection, thermal, oxygentank, iceaxe, wingsuit — or `ava_slot`)
# and its record (character.toml).
"character/{name}" = {{ model = "characters/{name}.glb", data = "characters/{name}.toml" }}
# Its portrait in the rider select and the store (88 × 256 like the disc's).
"texture/portraits/{name}" = "textures/portraits/{name}.png"

# How a disc rider looks: `ssx-assets content export ssx:character/elise --out <dir>` writes Elise's model, skeleton and
# record from your own disc for your own reference (exported files may not be shipped: `mod check` refuses them).
[override]
# "ssx:character/elise" = {{ model = "characters/{name}.glb" }}
"#,
        header(name, "A rider of its own: a mannequin skinned to the riders' skeleton, its record and portrait.")
    );
    std::fs::write(root.join(ssx_content::manifest::MANIFEST_FILE), manifest)?;
    let record = format!(
        r#"# A character (Docs/architecture/content.md §17.4).
# The disc rider whose place it takes where the game needs one by name: the simulation (its stats), the scenes, the
# animations, other players' screens in a session.
stands_in_for = "ssx:character/elise"
# The disc rider whose face offsets its skin binds to (default: the stand-in), and whose speech it speaks.
# face = "ssx:character/elise"
# voice = "ssx:character/elise"
# Its own lines by speech event (a package's sounds): "ssx:sound/speech/<lang>/<event>" = "{name}:sound/<path>".
[voice_lines]
# The board it starts with (its suit and, without this, its board are its stand-in's defaults) — a board id.
# default_board = "{name}:board/<path>"

# Boarder slots without an item: a package's gear look for the slot, or "none".
[outfit]
# headgear = "none"

# The rider record: the rider select, the store and the save read it.
[career]
# Its name and description as string ids (string tables come with phase M6; until then the name shows as display_name).
title = "{name}:string/name"
# description = "{name}:string/bio"
# The name as written: 1 to 32 characters A-Z a-z 0-9 space . _ -, a letter first; another rider's is refused.
display_name = "{shown}"
# The credits it is bought for, and whether it is locked (Rider_IsLocked): an unlocked rider is offered without a
# purchase (0x825251A0), a locked one is bought for its cost (or unlocked by the career).
cost = 1000
locked = false
# Listed by the store when not owned, as every disc rider (a locked rider is bought there).
default = true
# [GearStat, bonus] pairs the loadout adds: simulation (a session's sim fields are its stand-in's).
stat_bonuses = []
portrait = "{name}:texture/portraits/{name}"
# round_icon = "{name}:texture/<path>"
# icon = "{name}:texture/<path>"

# glTF materials bound to material ids by name (unbound: mapped onto the character family by the part's slot).
[materials]
# jackets = "{name}:material/<path>"
"#
    );
    std::fs::write(root.join(format!("characters/{name}.toml")), record)?;
    Ok(())
}

pub(super) fn gear(root: &Path, name: &str, data: &Path) -> Result<()> {
    let sk = skeleton(data)?;
    let b = Bones::new(&sk);
    let parts = mannequin(&b, &["jackets", "pants"])?;
    std::fs::create_dir_all(root.join("gear")).context("creating gear")?;
    std::fs::write(root.join(format!("gear/{name}.glb")), model_glb(&b, &parts)?)?;
    let manifest = format!(
        r#"{}
[add]
# The item: its model (one node per slot it fills: a jacket's jackets and pants) and its record (gear.toml).
"gear/{name}" = {{ model = "gear/{name}.glb", data = "gear/{name}.toml" }}

# A disc item's look: `ssx-assets content ids --kind gear` lists them, `content export <id>` writes one from your disc.
[override]
"#,
        header(name, "A suit of its own: a jacket and pants skinned to the riders' skeleton, and its record.")
    );
    std::fs::write(root.join(ssx_content::manifest::MANIFEST_FILE), manifest)?;
    let record = format!(
        r#"# A gear item (Docs/architecture/content.md §17.5).
# The GearItemType: wingsuit, iceaxe, armour, thermal, oxygen, headlamp, pulsegoggles, jacket (a suit: jackets and
# pants), pants.
slot = "jacket"
title = "{name}:string/name"
# The rider it is for (none: the store offers it to every rider).
# rider = "ssx:character/elise"
locked = false
# The rider level (0 to 9) and the quality (0 to 4) the store sorts it by.
level = 0
quality = 0
cost = 500
# [GearStat, value] pairs: simulation (a session's sim fields are the disc's).
attributes = []
# icon = "{name}:texture/<path>"
[materials]
"#
    );
    std::fs::write(root.join(format!("gear/{name}.toml")), record)?;
    Ok(())
}

pub(super) fn board_package(root: &Path, name: &str, data: &Path) -> Result<()> {
    let sk = skeleton(data)?;
    let b = Bones::new(&sk);
    let parts = vec![("snowboard".to_string(), board(&b)?)];
    std::fs::create_dir_all(root.join("boards")).context("creating boards")?;
    std::fs::write(root.join(format!("boards/{name}.glb")), model_glb(&b, &parts)?)?;
    let manifest = format!(
        r#"{}
[add]
# The board: its model (skinned to Board and the joints below it, or rigid: then wholly on Board) and its record.
"board/{name}" = {{ model = "boards/{name}.glb", data = "boards/{name}.toml" }}

# A disc board's look (every rider's board is the same `common_board_mesh` on the disc): name a disc board by id.
[override]
"#,
        header(name, "A board of its own: skinned to the board's flex joints, and its record.")
    );
    std::fs::write(root.join(ssx_content::manifest::MANIFEST_FILE), manifest)?;
    let record = format!(
        r#"# A board (Docs/architecture/content.md §17.5); a board.toml has no slot.
title = "{name}:string/name"
locked = false
level = 0
quality = 0
cost = 750
# [GearStat, value] pairs (BoardSpeed 0 … BoardCarving 4): simulation (a session's sim fields are the disc's).
attributes = []
# icon = "{name}:texture/<path>"
[materials]
"#
    );
    std::fs::write(root.join(format!("boards/{name}.toml")), record)?;
    Ok(())
}
