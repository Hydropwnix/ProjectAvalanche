use super::*;

fn bytes(s: &str) -> Asset {
    Asset::Bytes(Arc::from(s.as_bytes()))
}

fn record(s: &str) -> Asset {
    Asset::Record(toml::from_str(s).unwrap())
}

fn table(s: &str) -> toml::Table {
    toml::from_str(s).unwrap()
}

fn id(s: &str) -> ContentId {
    s.parse().unwrap()
}

fn base() -> MemorySource {
    MemorySource::new("ssx")
        .with(id("ssx:texture/worlds/textures/rock_02"), bytes("disc rock"))
        .with(id("ssx:texture/worlds/textures/snow"), bytes("disc snow"))
        .with(id("ssx:board/elise_default"), record("name = \"Elise\"\n[stats]\nspeed = 3\nedge = 4"))
        .with(id("ssx:character/elise"), Asset::Value(Arc::new(42u32)))
}

#[test]
fn highest_layer_wins_per_id() {
    let mut r = Registry::new();
    r.mount(Layer::Base, 0, Box::new(base()));
    r.mount(
        Layer::ClientMods,
        0,
        Box::new(MemorySource::new("rocks").with(id("ssx:texture/worlds/textures/rock_02"), bytes("mod rock"))),
    );
    let rock = r.load(&id("ssx:texture/worlds/textures/rock_02")).unwrap();
    assert_eq!(
        (rock.asset.bytes(), rock.layer, rock.source.as_str()),
        (Some(&b"mod rock"[..]), Layer::ClientMods, "rocks")
    );
    let snow = r.load(&id("ssx:texture/worlds/textures/snow")).unwrap();
    assert_eq!((snow.asset.bytes(), snow.layer), (Some(&b"disc snow"[..]), Layer::Base));
    assert_eq!(r.load(&id("ssx:character/elise")).unwrap().asset.value::<u32>(), Some(&42));
    assert_eq!(r.load(&id("ssx:texture/none")).unwrap_err(), LoadError::Missing(id("ssx:texture/none")));
    r.mount(
        Layer::Server,
        -5,
        Box::new(MemorySource::new("srv").with(id("ssx:texture/worlds/textures/rock_02"), bytes("srv"))),
    );
    assert_eq!(r.winner(&id("ssx:texture/worlds/textures/rock_02")).unwrap().name, "srv");
    assert_eq!(r.unmount(Layer::Server), 1);
    assert_eq!(r.winner(&id("ssx:texture/worlds/textures/rock_02")).unwrap().name, "rocks");
}

#[test]
fn priority_then_mount_order_within_a_layer() {
    let mut r = Registry::new();
    r.mount(Layer::Base, 0, Box::new(base()));
    let tex = "ssx:texture/worlds/textures/snow";
    r.mount(Layer::ClientMods, 5, Box::new(MemorySource::new("high").with(id(tex), bytes("high"))));
    r.mount(Layer::ClientMods, 0, Box::new(MemorySource::new("first").with(id(tex), bytes("first"))));
    r.mount(Layer::ClientMods, 0, Box::new(MemorySource::new("second").with(id(tex), bytes("second"))));
    assert_eq!(r.winner(&id(tex)).unwrap().name, "high");
    let names: Vec<&str> = r.mounts().map(|m| m.name).collect();
    assert_eq!(names, ["ssx", "first", "second", "high"]);
    let report = r.report();
    assert_eq!(
        report.conflicts,
        [Conflict {
            id: id(tex),
            layer: Layer::ClientMods,
            winner: "high".into(),
            losers: vec!["second".into(), "first".into()]
        }]
    );
    assert_eq!(report.overrides.len(), 1);
    assert_eq!(report.overrides[0].winner, (Layer::ClientMods, "high".to_string()));
    assert_eq!(report.overrides[0].over, [(Layer::Base, "ssx".to_string())]);
}

#[test]
fn patches_merge_in_layer_and_load_order() {
    let mut r = Registry::new();
    r.mount(Layer::Base, 0, Box::new(base()));
    let board = "ssx:board/elise_default";
    r.mount(
        Layer::ClientMods,
        0,
        Box::new(MemorySource::new("fast").with_patch(id(board), table("[stats]\nspeed = 9"))),
    );
    r.mount(
        Layer::ClientMods,
        0,
        Box::new(MemorySource::new("grippy").with_patch(id(board), table("[stats]\nedge = 7"))),
    );
    r.mount(
        Layer::Port,
        0,
        Box::new(MemorySource::new("ava").with_patch(id(board), table("[stats]\nspeed = 1\nnew = true"))),
    );
    let got = r.load(&id(board)).unwrap();
    let stats = got.asset.record().unwrap()["stats"].as_table().unwrap();
    assert_eq!(stats["speed"].as_integer(), Some(9), "the client mod's patch comes after the port's");
    assert_eq!(stats["edge"].as_integer(), Some(7), "two mods changing different fields both apply");
    assert_eq!(stats["new"].as_bool(), Some(true));
    assert_eq!(got.asset.record().unwrap()["name"].as_str(), Some("Elise"));
    assert_eq!(got.patched_by, ["ava", "fast", "grippy"]);
    assert_eq!(got.source, "ssx");
    r.mount(
        Layer::Server,
        0,
        Box::new(MemorySource::new("srv").with(id(board), record("name = \"Neon\"\n[stats]\nspeed = 2"))),
    );
    let got = r.load(&id(board)).unwrap();
    let rec = got.asset.record().unwrap();
    assert_eq!(
        (rec["name"].as_str(), rec["stats"]["speed"].as_integer(), got.source.as_str()),
        (Some("Neon"), Some(9), "srv")
    );
}

#[test]
fn patch_problems() {
    let mut r = Registry::new();
    r.mount(Layer::Base, 0, Box::new(base()));
    r.mount(
        Layer::ClientMods,
        0,
        Box::new(
            MemorySource::new("bad")
                .with_patch(id("ssx:character/elise"), table("x = 1"))
                .with_patch(id("ssx:board/nobody"), table("x = 1")),
        ),
    );
    let e = r.load(&id("ssx:character/elise")).unwrap_err();
    assert_eq!(e, LoadError::NotARecord { id: id("ssx:character/elise"), winner: "ssx".into(), by: "bad".into() });
    assert_eq!(r.report().dangling_patches, [DanglingPatch { id: id("ssx:board/nobody"), source: "bad".into() }]);
}

#[test]
fn lists_merge_sorted_and_unique() {
    let mut r = Registry::new();
    r.mount(Layer::Base, 0, Box::new(base()));
    r.mount(
        Layer::ClientMods,
        0,
        Box::new(
            MemorySource::new("m")
                .with(id("m:texture/a"), bytes("a"))
                .with(id("ssx:texture/worlds/textures/snow"), bytes("s")),
        ),
    );
    let got: Vec<String> = r.list(Kind::Texture).iter().map(ToString::to_string).collect();
    assert_eq!(got, ["m:texture/a", "ssx:texture/worlds/textures/rock_02", "ssx:texture/worlds/textures/snow"]);
    assert!(r.list(Kind::Sky).is_empty());
    assert!(r.report().conflicts.is_empty());
}
