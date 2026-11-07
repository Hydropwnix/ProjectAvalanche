use super::*;
use crate::data::{Mode, check_text};

fn schema_ok(rs: &RecordSchema, text: &str) -> bool {
    check_text(rs, text.as_bytes(), Mode::Whole, None).is_ok()
}

#[test]
fn every_field_of_the_decoders_is_described() {
    let all = [character(), item(false), item(true), marker()];
    for rs in &all {
        for (path, f) in rs.flat() {
            assert!(!matches!(f.ty, FieldType::Scalar(Scalar::Fixed(_))), "{}: {path} has no type", rs.name);
        }
    }
    assert_eq!(character().fields.len(), fields_of::<crate::rider::character::Raw>().len());
    assert_eq!(item(true).fields.len() + 1, item(false).fields.len());
    assert!(fields_of::<crate::rider::character::RawCareer>().contains(&"0x940b9b22"), "renames are the decoder's");
    assert!(HITS.iter().all(|h| crate::marker::Hit::parse(h).is_some()));
}

const CHARACTER: &str = r#"
stands_in_for = "ssx:character/elise"
face = "ssx:character/elise"
voice = "ssx:character/tyler"
default_board = "pack:board/neon"
[voice_lines]
"ssx:sound/speech/eng/x/vox_character_end_run_positive" = "pack:sound/sam/positive"
[outfit]
headgear = "none"
protection = "pack:gear/armour"
[career]
title = "pack:string/sam"
display_name = "Sam"
description = "pack:string/bio"
cost = 500
locked = true
stat_bonuses = [[0, 5], [-1, 0], [36, 255]]
portrait = "pack:texture/portraits/sam"
round_icon = "pack:texture/icons/sam"
icon = "pack:texture/icons/sam_tag"
default = true
"0x940b9b22" = 60
[materials]
jackets = "pack:material/jacket"
"#;

const GEAR: &str = r#"
slot = "jacket"
title = "pack:string/jacket"
rider = "ssx:character/elise"
locked = false
default = false
level = 9
quality = 4
cost = 100000000
attributes = [[0, 40], [1, 30]]
icon = "pack:texture/icons/jacket"
"0xb6ef00c6" = 0
[materials]
jackets = "pack:material/jacket"
"#;

const MARKER: &str = r##"
shape = "gate"
size = 100
color = "#FF6030E0"
glow = 8.0
facing = "camera"
draw_distance = 1.0
icon = "pack:texture/gate"
sound = "ssx:sound/tuna/hudsfx_tuna/geotag_collected"
hit = "gate"
[animation]
spin = -720.0
bob = 10
bob_period = 0.1
pulse = 1.0
pulse_period = 60
"##;

#[test]
fn what_a_reader_accepts_its_schema_accepts() {
    assert!(crate::rider::parse_character(CHARACTER.as_bytes(), "c").is_ok());
    assert!(schema_ok(&character(), CHARACTER));
    assert!(crate::rider::parse_item(GEAR.as_bytes(), false).is_ok());
    assert!(schema_ok(&item(false), GEAR));
    let board: String = GEAR.lines().filter(|l| !l.starts_with("slot")).map(|l| format!("{l}\n")).collect();
    assert!(crate::rider::parse_item(board.as_bytes(), true).is_ok());
    assert!(schema_ok(&item(true), &board));
    assert!(crate::marker::parse_toml(MARKER.as_bytes()).is_ok());
    assert!(schema_ok(&marker(), MARKER));
    let rgb = MARKER.replace("color = \"#FF6030E0\"", "color = [255, 96, 48]");
    assert!(crate::marker::parse_toml(rgb.as_bytes()).is_ok() && schema_ok(&marker(), &rgb));
}

#[test]
fn what_a_schema_refuses_its_reader_refuses() {
    let career = "[career]\ntitle = \"pack:string/x\"\ndisplay_name = \"Sam\"\n";
    for bad in [
        format!("{career}cost = 100000001\n"),
        format!("{career}cost = 1\nstat_bonuses = [[37, 0]]\n"),
        format!("{career}cost = 1\nstat_bonuses = [[0, 256]]\n"),
        "[outfit]\nwingsuit = \"none\"\n".into(),
        "stands_in_for = \"ssx:board/x\"\n".into(),
        "standsinfor = \"ssx:character/elise\"\n".into(),
    ] {
        assert!(!schema_ok(&character(), &bad), "the schema takes {bad}");
        assert!(crate::rider::parse_character(bad.as_bytes(), "c").is_err(), "the reader takes {bad}");
    }
    let gear = "title = \"pack:string/x\"\ncost = 1\n";
    for bad in [
        format!("slot = \"board\"\n{gear}"),
        format!("slot = \"jacket\"\n{gear}level = 10\n"),
        format!("slot = \"jacket\"\n{gear}quality = -1\n"),
        format!("slot = \"jacket\"\n{gear}attributes = [[0, 1], [1]]\n"),
    ] {
        assert!(!schema_ok(&item(false), &bad), "the schema takes {bad}");
        assert!(crate::rider::parse_item(bad.as_bytes(), false).is_err(), "the reader takes {bad}");
    }
    assert!(!schema_ok(&item(true), &format!("slot = \"jacket\"\n{gear}")));
    for bad in [
        "shape = \"gate\"\nsize = 100.5\n",
        "shape = \"gate\"\nglow = -1\n",
        "shape = \"gate\"\nfacing = \"up\"\n",
        "shape = \"gate\"\nhit = \"cube\"\n",
        "shape = \"cube\"\n",
        "shape = \"gate\"\ncolor = [1, 2]\n",
        "shape = \"gate\"\n[animation]\nspin = 721.0\n",
        "shape = \"gate\"\nsize = nan\n",
    ] {
        assert!(!schema_ok(&marker(), bad), "the schema takes {bad}");
        assert!(crate::marker::parse_toml(bad.as_bytes()).is_err(), "the reader takes {bad}");
    }
}

const ANIM: &str = "[anim]\ngltf = \"a.glb\"\nanimation = \"x\"\nadditive = true\n\n[[anim.float]]\ndof = \"TrajectoryVelocityRig.AITrajectory.height\"\ninterpolation = \"cubic\"\ntimes = [0.0, 0.5]\nvalues = [0.0, 1.0, 9000.0, -9000.0, 2.0, 0.0]\n";

#[test]
fn the_anim_schema_follows_its_reader() {
    let rs = anim();
    for (path, f) in rs.flat() {
        assert!(!matches!(f.ty, FieldType::Scalar(Scalar::Fixed(_))), "anim: {path} has no type");
    }
    assert!(crate::anim::parse_toml(ANIM.as_bytes()).is_ok());
    assert!(schema_ok(&rs, ANIM));
    assert!(crate::anim::parse_toml(crate::anim::template::ANIM_TOML.as_bytes()).is_ok());
    assert!(schema_ok(&rs, crate::anim::template::ANIM_TOML));
    for bad in [
        ANIM.replace("[0.0, 0.5]", "[0.0, 61.0]"),
        ANIM.replace("\"cubic\"", "\"smooth\""),
        ANIM.replace("additive = true", "additive = 1"),
        ANIM.replace("9000.0", "20000.0"),
        ANIM.replace("animation", "animations"),
    ] {
        assert!(!schema_ok(&rs, &bad), "the schema takes {bad}");
        assert!(crate::anim::parse_toml(bad.as_bytes()).is_err(), "the reader takes {bad}");
    }
}

#[test]
fn the_trick_schema_follows_its_reader() {
    let rs = trick();
    for (path, f) in rs.flat() {
        assert!(!matches!(f.ty, FieldType::Scalar(Scalar::Fixed(_))), "trick: {path} has no type");
    }
    let good = crate::trick::template::trick_toml("p");
    assert!(crate::trick::parse_toml(good.as_bytes()).is_ok());
    assert!(schema_ok(&rs, &good));
    for bad in [
        good.replace("1300", "2401"),
        good.replace("\"indy\"", "\"airwalk\""),
        good.replace("\"up-up\"", "\"up-left\""),
        good.replace("\"both\"", "\"left\""),
        good.replace("p:string/air_walk\"", "p:anim/air_walk\""),
        good.replace("[trick.clips]", "[trick.clipz]"),
    ] {
        assert!(!schema_ok(&rs, &bad), "the schema takes {bad}");
        assert!(crate::trick::parse_toml(bad.as_bytes()).is_err(), "the reader takes {bad}");
    }
}

#[test]
fn the_gamemode_schema_follows_its_reader() {
    let rs = gamemode();
    for (path, f) in rs.flat() {
        assert!(!matches!(f.ty, FieldType::Scalar(Scalar::Fixed(_))), "gamemode: {path} has no type");
    }
    let good = crate::gamemode::template::gamemode_toml("p:map/gully");
    let [g, si, b] = crate::gamemode::template::TARGETS_S;
    let (three, two) = (format!("[{g:.1}, {si:.1}, {b:.1}]"), format!("[{g:.1}, {si:.1}]"));
    assert!(good.contains(&three), "the template's targets");
    assert!(crate::gamemode::parse_toml(good.as_bytes()).is_ok());
    assert!(schema_ok(&rs, &good));
    for bad in [
        good.replace("\"race_ghosts\"", "\"tag\""),
        good.replace("\"fail\"", "\"later\""),
        good.replace("90.0", "4000.0"),
        good.replace("p:map/gully", "p:trick/gully"),
        good.replace("[[gamemode.map]]", "[[gamemode.maps]]"),
        good.replace(&three, &two),
    ] {
        assert!(!schema_ok(&rs, &bad), "the schema takes {bad}");
        assert!(crate::gamemode::parse_toml(bad.as_bytes()).is_err(), "the reader takes {bad}");
    }
}
