use std::collections::BTreeMap;

use super::*;
use crate::anim::{ANIM_FILE, read_anim};
use crate::model::FileReader;

struct Files(BTreeMap<String, Vec<u8>>);

impl FileReader for Files {
    fn read(&self, path: &str, max: u64) -> Result<Vec<u8>, String> {
        let b = self.0.get(path).ok_or_else(|| format!("{path}: not in the package"))?;
        if b.len() as u64 > max { Err(format!("{path}: over {max} bytes")) } else { Ok(b.clone()) }
    }

    fn size(&self, path: &str) -> Result<u64, String> {
        Ok(self.0.get(path).ok_or_else(|| format!("{path}: not in the package"))?.len() as u64)
    }

    fn read_range(&self, path: &str, offset: u64, len: u64) -> Result<Vec<u8>, String> {
        let b = self.0.get(path).ok_or_else(|| format!("{path}: not in the package"))?;
        let (o, l) =
            (usize::try_from(offset).map_err(|e| e.to_string())?, usize::try_from(len).map_err(|e| e.to_string())?);
        b.get(o..o.saturating_add(l)).map(<[u8]>::to_vec).ok_or_else(|| format!("{path}: short"))
    }
}

fn messages(text: &str) -> String {
    match parse_toml(text.as_bytes()) {
        Ok(_) => String::new(),
        Err(p) => p.iter().map(|(l, c, m)| format!("{l}:{c} {m}")).collect::<Vec<_>>().join("\n"),
    }
}

#[test]
fn the_template_reads_as_made_and_its_committed_clip_is_the_generated_one() {
    let t = parse_toml(template::trick_toml("my-pack").as_bytes()).expect("the template's record reads");
    assert_eq!((BASES[t.base].0, t.input, t.hand, t.points), ("indy", Input::UpUp, Hand::Both, 1300));
    assert_eq!(t.value(), 32);
    assert_eq!(t.names[1].to_string(), "my-pack:string/air_walk_uber");
    assert!(t.clips.iter().all(|c| c.to_string() == "my-pack:anim/air_walk"));
    assert_eq!(t.base_clip(11), "1_trick_air_indy_super_uber_tweak_loop");
    let glb = template::glb().expect("the template is written");
    let mut files = BTreeMap::new();
    files.insert(template::GLTF_PATH.to_string(), glb.clone());
    let back = read_anim(template::ANIM_TOML.as_bytes(), ANIM_FILE, &Files(files)).expect("the clip reads");
    let made = template::clip();
    assert_eq!(back.joints.len(), made.joints.len());
    for (a, b) in back.joints.iter().zip(&made.joints) {
        assert_eq!((&a.joint, a.path, &a.keys, &a.parent), (&b.joint, b.path, &b.keys, &b.parent));
    }
    assert!(!back.additive && (back.duration - 1.0).abs() < 1e-6);
    let committed = include_bytes!("../../../../assets/templates/trick/anims/air_walk.glb");
    assert!(committed[..] == glb[..], "rewrite it: cargo run -p ssx-content --example trick_template");
}

#[test]
fn every_field_is_checked_and_named() {
    let good = template::trick_toml("p");
    assert_eq!(messages(&good), "");
    for (bad, wanted) in [
        (good.replace("\"indy\"", "\"airwalk\""), "base: one of flyingsquirrel"),
        (good.replace("\"up-up\"", "\"up-left\""), "input: up-left is the disc's Flying Squirrel (goofy Stiffy)"),
        (good.replace("\"up-up\"", "\"down-down\""), "input: one of up-up, left-left, right-right"),
        (good.replace("\"both\"", "\"left\""), "hand: rear, front or both"),
        (good.replace("1300", "0"), "points: 1 to 2400"),
        (good.replace("1300", "2401"), "points: 1 to 2400"),
        (good.replace("1300", "-5"), "points: 1 to 2400"),
        (good.replace("1300", "9223372036854775807"), "points: 1 to 2400"),
        (good.replace("p:string/air_walk_uber", "p:anim/air_walk_uber"), "name_uber: a `string` id"),
        (good.replace("enter = \"p:anim/air_walk\"\n", ""), "clips: no enter"),
        (good.replace("tweak_loop = \"p:anim/air_walk\"", "tweak_loop = \"p:anim/../x\""), "clips.tweak_loop:"),
        (good.replace("[trick.clips]", "[trick.clipz]"), "unknown field `clipz`"),
        (good.replace("points = 1300", "points = 1300\ntags = []"), "unknown field `tags`"),
        ("".to_string(), "a trick.toml has a [trick] table"),
        (good.replace("1300", "1300.5"), "points: invalid type: floating point"),
        (good.replace("1300", "99999999999999999999999"), "points: "),
        (good.replace("\"indy\"", "3"), "base: invalid type: integer"),
        (good.replace("enter = \"p:anim/air_walk\"", "enter = 7"), "clips.enter: invalid type: integer"),
    ] {
        let m = messages(&bad);
        assert!(m.contains(wanted), "{bad}\n→ {m}\n(wanted {wanted})");
    }
    let t = parse_toml(good.replace("hand = \"both\"\n", "").as_bytes()).expect("reads");
    assert_eq!(t.hand, Hand::Rear, "the Indy is a rear-hand grab (node+0x274)");
}

#[test]
fn a_hostile_key_is_shown_escaped_and_bounded() {
    let key = format!("\"\\u001b[31m{}\"", "x".repeat(10_000));
    let m = messages(&format!("[trick]\n{key} = 1\n"));
    assert!(!m.contains('\u{1b}') && m.chars().count() < 700, "{m}");
}

#[test]
fn the_sim_hash_takes_what_the_ride_reads_and_not_the_names() {
    let t = parse_toml(template::trick_toml("p").as_bytes()).expect("reads");
    let clips = [ContentHash([7; 32]); 12];
    let h = sim_hash(&t, &clips);
    let mut renamed = t.clone();
    renamed.names[0] = "p:string/other".parse().expect("an id");
    assert_eq!(sim_hash(&renamed, &clips), h);
    for changed in [
        TrickToml { points: 1301, ..t.clone() },
        TrickToml { hand: Hand::Rear, ..t.clone() },
        TrickToml { input: Input::LeftLeft, ..t.clone() },
        TrickToml { base: 0, ..t.clone() },
    ] {
        assert_ne!(sim_hash(&changed, &clips), h);
    }
    let mut other = clips;
    other[5] = ContentHash([8; 32]);
    assert_ne!(sim_hash(&t, &other), h);
}

#[test]
fn the_inputs_mirror_as_the_discs_do() {
    for i in INPUTS {
        assert_eq!(i.mirror().mirror(), i);
        assert_eq!(Input::parse(i.name()), Some(i));
    }
    assert_eq!(Input::LeftLeft.mirror(), Input::RightRight);
    let mirror = |n: &str| n.replace("left", "L").replace("right", "left").replace('L', "right");
    for (n, regular, goofy) in DISC_INPUTS {
        let m = DISC_INPUTS.iter().find(|d| d.0 == mirror(n)).expect("the mirror is an input");
        assert_eq!((m.1, m.2), (goofy, regular), "{n}");
    }
}
