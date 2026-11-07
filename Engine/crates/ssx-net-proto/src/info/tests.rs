use super::*;
use crate::instance::{GamemodeSummary, MAX_MAP_ID, MAX_MAP_TITLE_CHARS, MapSummary};
use crate::keys::ServerSecret;

fn info() -> ServerInfo {
    ServerInfo {
        info_version: INFO_VERSION,
        name: "Alps Night Riders".into(),
        description: "Freeride on Vorassay, all welcome. Zoë's server €".into(),
        protocol_version: crate::PROTOCOL_VERSION,
        api_level: crate::API_LEVEL,
        build: crate::BUILD_ID.into(),
        players: 3,
        max_players: 16,
        password: false,
        mode: "freeride".into(),
        drop: "exp_alps_vorassay_drop_3".into(),
        map: None,
        rewinds: "on".into(),
        ghosts: false,
        ghost_style: "character".into(),
        identity_key: ServerSecret::generate().public().to_hex(),
        resources: crate::resources::summary(),
        region: "eu-central".into(),
        tags: vec!["race".into(), "no-rewinds".into()],
        icon: "ab".repeat(32),
    }
}

#[test]
fn strict_reading() {
    let good = info();
    assert_eq!(ServerInfo::from_json(good.to_json().as_bytes()), Ok(good.clone()));
    let json = good.to_json();
    let cases = [
        json.replacen(
            &format!("\"info_version\":{INFO_VERSION}"),
            &format!("\"info_version\":{}", INFO_VERSION - 1),
            1,
        ),
        json.replacen('}', ",\"extra\":\"race\"}", 1),
        json.replacen("eu-central", "EU", 1),
        json.replacen("\"race\",", "\"Race\",", 1),
        json.replacen("\"race\",", "\"no-rewinds\",", 1),
        json.replacen("[\"race\",", "[\"a\",\"b\",\"c\",\"d\",\"e\",\"f\",\"g\",\"race\",", 1),
        json.replacen(&"ab".repeat(32), &"AB".repeat(32), 1),
        json.replacen(&"ab".repeat(32), &"ab".repeat(31), 1),
        json.replacen(&"ab".repeat(32), &"zz".repeat(32), 1),
        json.replacen(",\"icon\"", ",\"icon_\"", 1),
        json.replacen(",\"map\":null", "", 1),
        json.replacen("\"mode\":\"freeride\"", "\"mode\":\"tag\"", 1),
        json.replacen("exp_alps_vorassay_drop_3", "../x", 1),
        json.replacen(",\"password\":false", "", 1),
        json.replacen("\"players\":3", "\"players\":17", 1),
        json.replacen("Alps Night Riders", "Alps\\u0000", 1),
        json.replacen("Alps Night Riders", " Alps", 1),
        json.replacen("Alps Night Riders", &"N".repeat(41), 1),
        json.replacen(&good.identity_key, &"0".repeat(64), 1),
        json.replacen('}', &format!(",\"pad\":\"{}\"}}", "x".repeat(MAX_INFO_BYTES)), 1),
        json.replacen("\"players\":3", "\"players\":3,\"players\":4", 1),
        json.replacen("\"rewinds\":\"on\"", "\"rewinds\":\"0\"", 1),
        json.replacen("\"rewinds\":\"on\"", "\"rewinds\":3", 1),
        json.replacen("\"ghost_style\":\"character\"", "\"ghost_style\":\"ghost\"", 1),
        json.replacen("all welcome.", "all\\nwelcome.", 1),
        json.replacen("all welcome.", &"w".repeat(MAX_DESCRIPTION_CHARS), 1),
        json.replacen("\"ghosts\":false", "\"ghosts\":0", 1),
        json.replacen(",\"resources\":{", ",\"x\":{", 1),
        json.replacen("\"client_mods\":\"presentation\"", "\"client_mods\":\"all\"", 1),
        json.replacen("\"download\":20635", "\"download\":8589934593", 1),
        json.replacen("\"download\":20635", "\"download\":-1", 1),
        json.replacen("\"count\":2", "\"count\":257", 1),
        json.replacen(&"4e".repeat(32), &"4E".repeat(32), 1),
        json.replacen(&"ab".repeat(32), "", 1),
        json.replacen("\"manifest_bytes\":2431", "\"manifest_bytes\":8388609", 1),
        json.replacen("\"client_scripts\":false", "\"client_scripts\":false,\"mods\":[]", 1),
    ];
    for bad in cases {
        assert!(ServerInfo::from_json(bad.as_bytes()).is_err(), "{bad}");
    }
    for mode in ["race", "trick", "survive"] {
        let round = json.replacen("\"mode\":\"freeride\"", &format!("\"mode\":\"{mode}\""), 1);
        assert_eq!(ServerInfo::from_json(round.as_bytes()).map(|i| i.mode), Ok(mode.to_string()));
    }
    let long = ServerInfo { description: "ÿ".repeat(MAX_DESCRIPTION_CHARS), ..good.clone() };
    let escaped = long.to_json().replace('ÿ', "\\u00ff");
    assert!(escaped.len() > 1500 && escaped.len() <= MAX_INFO_BYTES, "{}", escaped.len());
    assert_eq!(ServerInfo::from_json(escaped.as_bytes()), Ok(long));
}

#[test]
fn the_stable_members_say_why_another_version_cannot_be_used() {
    let future = r#"{"info_version":9,"protocol_version":7,"name":"Next Gen","tags":["x"],"whatever":{"a":1}}"#;
    let head = InfoHead::from_json(future.as_bytes()).expect("a head");
    assert_eq!((head.info_version, head.protocol_version, head.name.as_deref()), (9, 7, Some("Next Gen")));
    let e = ServerInfo::from_json(future.as_bytes()).expect_err("another version");
    assert!(e.contains("version 9") && e.contains(&format!("reads {INFO_VERSION}")), "{e}");
    let rude = r#"{"info_version":4,"protocol_version":3,"name":"<b>\u0007</b>"}"#;
    assert_eq!(InfoHead::from_json(rude.as_bytes()).map(|h| h.name), Ok(None));
    for bad in [&b"[]"[..], b"{\"info_version\":3}", b"{\"info_version\":-1,\"protocol_version\":3}", b"nul"] {
        assert!(InfoHead::from_json(bad).is_err(), "{}", String::from_utf8_lossy(bad));
    }
    assert!(InfoHead::from_json(&vec![b' '; MAX_INFO_BYTES + 1]).is_err());
}

#[test]
fn no_icon_and_no_region_read() {
    let bare = ServerInfo { region: String::new(), tags: Vec::new(), icon: String::new(), ..info() };
    assert_eq!(ServerInfo::from_json(bare.to_json().as_bytes()), Ok(bare));
    assert_eq!(icon_hash(&"0f".repeat(32)).map(|h| h.map(|h| (h[0], h[31]))), Ok(Some((0x0f, 0x0f))));
}

#[test]
fn a_map_reads_with_an_empty_drop_and_its_rules() {
    let map =
        MapSummary { id: "slope:map/course".into(), title: "Template slope".into(), event_type: 5, gamemode: None };
    let good = ServerInfo { mode: "race".into(), drop: String::new(), map: Some(map.clone()), ..info() };
    assert_eq!(ServerInfo::from_json(good.to_json().as_bytes()), Ok(good.clone()));
    let json = good.to_json();
    let member =
        "\"map\":{\"id\":\"slope:map/course\",\"title\":\"Template slope\",\"event_type\":5,\"gamemode\":null}";
    assert!(json.contains(member), "{json}");
    let cases = [
        json.replacen("\"drop\":\"\"", "\"drop\":\"exp_alps_vorassay_drop_3\"", 1),
        json.replacen(member, "\"map\":null", 1),
        json.replacen("slope:map/course", "slope:map/../x", 1),
        json.replacen("slope:map/course", "Slope:map/course", 1),
        json.replacen("slope:map/course", "slope:object/course", 1),
        json.replacen("slope:map/course", "slope:map/", 1),
        json.replacen("slope:map/course", &format!("slope:map/{}", "c".repeat(MAX_MAP_ID)), 1),
        json.replacen("Template slope", "", 1),
        json.replacen("Template slope", &"t".repeat(MAX_MAP_TITLE_CHARS + 1), 1),
        json.replacen("Template slope", "Template\\u001bslope", 1),
        json.replacen("\"event_type\":5", "\"event_type\":9", 1),
        json.replacen("\"event_type\":5", "\"event_type\":5,\"rules\":\"x\"", 1),
        json.replacen(",\"gamemode\":null", "", 1),
        json.replacen("\"gamemode\":null", "\"gamemode\":\"gully:gamemode/x\"", 1),
        json.replacen("\"gamemode\":null", "\"gamemode\":{\"id\":\"gully:map/x\",\"title\":\"G\"}", 1),
        json.replacen("\"gamemode\":null", "\"gamemode\":{\"id\":\"gully:gamemode/x\",\"title\":\"\"}", 1),
        json.replacen("\"gamemode\":null", "\"gamemode\":{\"id\":\"gully:gamemode/x\",\"title\":\"a\\u0007\"}", 1),
        json.replacen("\"gamemode\":null", "\"gamemode\":{\"id\":\"gully:gamemode/x\",\"title\":\"G\",\"x\":1}", 1),
    ];
    for bad in cases {
        assert!(ServerInfo::from_json(bad.as_bytes()).is_err(), "{bad}");
    }
    let under = GamemodeSummary { id: "gully:gamemode/x".into(), title: "Gully Race".into() };
    let g = ServerInfo { map: Some(MapSummary { gamemode: Some(under), ..map.clone() }), ..good.clone() };
    assert_eq!(ServerInfo::from_json(g.to_json().as_bytes()), Ok(g));
    let wide_gamemode = GamemodeSummary {
        id: format!("a:gamemode/{}", "c".repeat(MAX_MAP_ID - 11)),
        title: "ÿ".repeat(MAX_MAP_TITLE_CHARS),
    };
    let long = MapSummary {
        title: "ÿ".repeat(MAX_MAP_TITLE_CHARS),
        id: format!("a:map/{}", "c".repeat(250)),
        gamemode: Some(wide_gamemode),
        ..map
    };
    let wide = ServerInfo { map: Some(long), description: "ÿ".repeat(MAX_DESCRIPTION_CHARS), ..good };
    let escaped = wide.to_json().replace('ÿ', "\\u00ff");
    assert!(escaped.len() <= MAX_INFO_BYTES, "{}", escaped.len());
    assert_eq!(ServerInfo::from_json(escaped.as_bytes()), Ok(wide));
}
