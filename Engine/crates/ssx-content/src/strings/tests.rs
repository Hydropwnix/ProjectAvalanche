use super::*;

fn parse(text: &str) -> Result<Table, Vec<Problem>> {
    parse_table(text.as_bytes(), "strings/de.toml", Lang::De, "pack")
}

#[test]
fn keys_name_base_and_added_strings() {
    let t =
        parse("TXT_Main_Menu = \"Hauptmenü\"\ntxt_ava_hub_title = \"LAWINE\"\nsam = \"Sam\"\n\"bio/long\" = \"x\"\n")
            .unwrap();
    let ids: Vec<&str> = t.entries.keys().map(ContentId::as_str).collect();
    assert_eq!(
        ids,
        ["ava:string/txt_ava_hub_title", "pack:string/bio/long", "pack:string/sam", "ssx:string/txt_main_menu"]
    );
    assert_eq!(t.entries.values().find(|e| e.key == "sam").unwrap().at, (3, 1));
    let e = parse("TXT_A = \"1\"\ntxt_a = \"2\"\nSam = \"x\"\n[t]\nx = 1\n").unwrap_err();
    let msgs: Vec<&str> = e.iter().map(|p| p.2.as_str()).collect();
    assert!(msgs.iter().any(|m| m.contains("again (keys are case-insensitive)")), "{msgs:?}");
    assert!(msgs.iter().any(|m| m.contains("an id's path")), "{msgs:?}");
    assert!(msgs.iter().any(|m| m.contains("a TOML string")), "{msgs:?}");
    assert_eq!(e.iter().find(|p| p.2.contains("again")).unwrap().0, 2);
    assert_eq!(table_lang("strings/ja.toml"), Some(Ok(Lang::Ja)));
    assert!(matches!(table_lang("strings/xx.toml"), Some(Err(_))));
    assert!(matches!(table_lang("strings/de/x.toml"), Some(Err(_))));
    assert_eq!(table_lang("textures/x.toml"), None);
}

#[test]
fn texts_are_capped() {
    assert!(check_text(&"x".repeat(caps::TEXT_BYTES)).is_ok());
    assert!(check_text(&"x".repeat(caps::TEXT_BYTES + 1)).unwrap_err().contains("bytes of text"));
    assert!(check_text(&"a\n".repeat(caps::LINES + 1)).unwrap_err().contains("lines"));
    assert!(check_text("bell\u{7}").unwrap_err().contains("U+0007"));
    assert!(check_text("tab\tand\r\nlines").is_ok());
    let big = format!("k = \"{}\"\n", "x".repeat(caps::TEXT_BYTES + 1));
    assert!(parse(&big).is_err());
}

#[test]
fn markup_is_the_players_subset() {
    for ok in [
        "<font color=\"#FF8000\" size=\"20\">a</font> <b>b</b><br>c",
        "<p align=\"center\">x</p><textformat leading=\"4\">y</textformat>",
        "a < b, <3 and <> are text",
        "<font face='DIN Engschrift Std' kerning=\"1\" alpha=\"#80\">x</font>",
    ] {
        assert_eq!(check_markup(ok), Ok(()), "{ok}");
    }
    for (bad, why) in [
        ("<script>x</script>", "not a tag"),
        ("<font size=\"100000\">x</font>", "from 1 to 255"),
        ("<font color=\"red\">x</font>", "hex digits"),
        ("<font onload=\"x\">x</font>", "no attribute"),
        ("<b>x</i>", "closes <b>"),
        ("<b>x", "not closed"),
        ("</b>", "closes nothing"),
        ("<font size=12>x</font>", "quote"),
        ("<b", "without `>`"),
    ] {
        let e = check_markup(bad).unwrap_err();
        assert!(e.contains(why), "{bad}: {e}");
    }
    assert!(check_markup(&"<b>".repeat(caps::DEPTH + 1)).unwrap_err().contains("nested"));
    assert!(check_markup(&"<br>".repeat(caps::TAGS + 1)).unwrap_err().contains("tags"));
    assert!(check_markup(&format!("<font face=\"{}\">", "x".repeat(300))).is_err());
}

#[test]
fn placeholders_and_tokens_follow_the_base() {
    let base = "Explore %2s of %3s, World Tour %1s/9 ^A^";
    assert_eq!(check_against("%1s/9 Welt, %2s von %3s ^A^", Some(base)), Ok(()));
    assert!(check_against("%1s %2s ^A^", Some(base)).unwrap_err().contains("placeholders"));
    assert!(check_against("%1d %2s %3s ^A^", Some(base)).unwrap_err().contains("placeholders"));
    assert!(check_against("%1[999999999]s %2s %3s ^A^", Some(base)).unwrap_err().contains("placeholders"));
    assert!(check_against("%1s %2s %3s ^B^", Some(base)).unwrap_err().contains("button tokens"));
    assert_eq!(check_against("100% %d %%", Some("100%")), Ok(()));
    assert!(check_against("%1s", None).unwrap_err().contains("adds"));
    assert_eq!(check_against("50%", None), Ok(()));
}
