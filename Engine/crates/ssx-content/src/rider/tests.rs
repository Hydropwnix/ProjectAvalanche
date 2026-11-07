use super::*;

const SAM: &str = r#"stands_in_for = "ssx:character/zoe"
voice = "ssx:character/elise"
default_board = "sam-pack:board/sam"
[voice_lines]
"ssx:sound/speech/eng/pilot_and_rider_eventdef_eng/vox_character_exit_chopper" = "sam-pack:sound/exit"
[outfit]
headlamp = "none"
gloves = "sam-pack:gear/gloves"
[career]
title = "sam-pack:string/sam_name"
display_name = "Sam"
cost = 12000
stat_bonuses = [[0, 5], [31, 7]]
portrait = "sam-pack:texture/portraits/sam"
"#;

#[test]
fn a_character_reads_and_round_trips_through_its_table() {
    let def = parse_character(SAM.as_bytes(), "sam").unwrap();
    assert_eq!(def.stands_in_for.as_ref().map(|i| i.as_str()), Some("ssx:character/zoe"));
    assert_eq!(def.outfit["headlamp"], OutfitSlot::Empty);
    let c = def.career.as_ref().unwrap();
    assert_eq!((c.display_name.as_str(), c.cost, c.stat_bonuses.as_slice()), ("Sam", 12000, &[(0, 5), (31, 7)][..]));
    let again = parse_character_table(&def.to_table()).unwrap();
    assert_eq!(again, def);
}

#[test]
fn a_hostile_character_is_refused_with_its_lines() {
    let bad = r#"stands_in_for = "sam-pack:character/other"
[voice_lines]
"ssx:sound/sfx/boardsfx/jump" = "sam-pack:sound/x"
[outfit]
tail = "none"
iceaxe = "sam-pack:gear/axe"
[career]
title = "sam-pack:string/n"
display_name = "Sam\nRiderUnlock_Zoe"
cost = 1000000000
stat_bonuses = [[40, 1], [0, 300]]
"#;
    let p = parse_character(bad.as_bytes(), "bad").unwrap_err();
    let lines: Vec<usize> = p.iter().map(|(l, _, _)| *l).collect();
    assert_eq!(lines, [1, 3, 6, 5, 9, 10, 11, 11], "{p:?}");
    assert!(parse_character(b"stands_in_for = 3\n", "x").unwrap_err()[0].2.contains("invalid type"));
    assert!(parse_character(b"colour = \"red\"\n", "x").unwrap_err()[0].2.contains("unknown field"));
}

#[test]
fn items_read_by_their_decoded_names() {
    let g = parse_item(
        b"slot = \"headlamp\"\ntitle = \"p:string/lamp\"\ncost = 500\nattributes = [[20, 64], [21, 75]]\n",
        false,
    )
    .unwrap();
    assert_eq!((g.kind, g.slot, g.attributes.as_slice()), (ItemKind::Gear, "headlamp", &[(20, 64), (21, 75)][..]));
    assert_eq!(parse_item_table(&g.to_table(), false).unwrap(), g);
    let suit = parse_item(b"slot = \"jacket\"\ntitle = \"p:string/s\"\ncost = 0\n", false).unwrap();
    assert_eq!(suit.kind, ItemKind::Suit);
    let b = parse_item(b"title = \"p:string/b\"\ncost = 9000\nattributes = [[0, 10]]\n", true).unwrap();
    assert_eq!((b.kind, b.slot), (ItemKind::Board, "board"));
    assert!(
        parse_item(b"slot = \"board\"\ntitle = \"p:string/b\"\ncost = 1\n", false).unwrap_err()[0]
            .2
            .contains("board.toml")
    );
    assert!(
        parse_item(b"slot = \"wingsuit\"\ntitle = \"p:string/b\"\ncost = 1\n", true).unwrap_err()[0]
            .2
            .contains("no slot")
    );
    assert!(parse_item(b"title = \"p:string/b\"\ncost = 1\n", false).unwrap_err()[0].2.contains("slot"));
    let lv = parse_item(b"slot = \"armour\"\ntitle = \"p:string/b\"\ncost = 1\nlevel = 10\n", false).unwrap_err();
    assert_eq!(lv[0].0, 4);
}

#[test]
fn a_part_takes_its_slot_from_its_node() {
    assert_eq!(slot_of(None, "Jackets.001"), Some("jackets"));
    assert_eq!(slot_of(Some("head"), "Body"), Some("head"));
    assert_eq!(slot_of(Some("tail"), "head"), None, "an ava_slot that is not a slot");
    assert_eq!(slot_of(None, "Cube"), None);
    assert!(
        character::display_name_ok("Sam") && !character::display_name_ok("1Sam") && !character::display_name_ok("")
    );
}
