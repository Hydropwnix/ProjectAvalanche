use super::*;

fn server(name: &str, players: u32, max: u32, password: bool, mode: &str, drop: &str, ping_ms: u64) -> Entry {
    let a = ServerAddress::parse(&format!("{}.example:1", name.to_lowercase().replace(' ', "-")), None).unwrap();
    let mut e = Entry::new(a, None);
    e.state = State::Info(Box::new(ServerInfo {
        info_version: ssx_net_proto::info::INFO_VERSION,
        name: name.into(),
        description: String::new(),
        protocol_version: ssx_net_proto::PROTOCOL_VERSION,
        api_level: 0,
        build: "b".into(),
        players,
        max_players: max,
        password,
        mode: mode.into(),
        drop: drop.into(),
        map: None,
        rewinds: "on".into(),
        ghosts: false,
        ghost_style: "character".into(),
        identity_key: "0".repeat(64),
        resources: ssx_net_proto::resources::ResourceSummary {
            content_id: String::new(),
            client_mods: "presentation".into(),
            client_scripts: false,
            count: 0,
            download: 0,
            manifest: "ab".repeat(32),
            manifest_bytes: 200,
        },
        region: String::new(),
        tags: Vec::new(),
        icon: String::new(),
    }));
    e.ping = Some(Duration::from_millis(ping_ms));
    e
}

fn names(entries: &[Entry], f: &Filters, search: &str) -> Vec<String> {
    let terms = parse_search(search);
    let title = |d: &str| if d == "drop_b" { "Aoraki".to_string() } else { "Zermatt".to_string() };
    let p = ViewParams { filters: f, search: &terms, drop_title: &title };
    view(entries, &p).into_iter().map(|i| entries[i].name()).collect()
}

fn list() -> Vec<Entry> {
    let mut e = vec![
        server("Bravo", 16, 16, false, "freeride", "drop_a", 30),
        server("alpha", 0, 8, true, "race", "drop_b", 80),
        server("Charlie", 3, 8, false, "freeride", "drop_b", 10),
    ];
    e[2].players = Some(vec!["Elise".into(), "Mac".into()]);
    let mut pending = Entry::new(ServerAddress::parse("10.0.0.1:1", None).unwrap(), Some("Delta".into()));
    pending.state = State::Failed("nothing answers there".into());
    e.push(pending);
    let mut old = Entry::new(ServerAddress::parse("10.0.0.2:1", None).unwrap(), None);
    old.state = State::Incompatible {
        head: InfoHead { info_version: 2, protocol_version: 2, name: Some("Echo".into()) },
        why: "protocol 2".into(),
    };
    e.push(old);
    e
}

#[test]
fn search_terms_read_as_typed() {
    let t = parse_search(r#"  ~mode:race "night riders" Player:Elise tag:eu nope:x ~~a  "#);
    let shown: Vec<(bool, Option<Category>, &str)> =
        t.iter().map(|t| (t.negated, t.category, t.value.as_str())).collect();
    assert_eq!(
        shown,
        [
            (true, Some(Category::Mode), "race"),
            (false, None, "night riders"),
            (false, Some(Category::Player), "elise"),
            (false, Some(Category::Tag), "eu"),
            (false, None, "nope:x"),
            (true, None, "~a"),
        ]
    );
    assert!(parse_search("mode: ~ \"\"").is_empty(), "empty values are no terms");
    assert_eq!(parse_search(&"a ".repeat(40)).len(), MAX_TERMS);
}

#[test]
fn filters_sort_and_search() {
    let e = list();
    let all = Filters::default();
    assert_eq!(names(&e, &all, ""), ["alpha", "Bravo", "Charlie", "Delta", "Echo"]);
    let f = Filters { not_full: true, not_empty: true, ..Default::default() };
    assert_eq!(names(&e, &f, ""), ["Charlie", "Delta", "Echo"], "rows without an answer pass the count filters");
    let f = Filters { no_password: true, compatible: true, ..Default::default() };
    assert_eq!(names(&e, &f, ""), ["Bravo", "Charlie", "Delta"]);
    let f = Filters { mode: Some("freeride".into()), ..Default::default() };
    assert_eq!(names(&e, &f, ""), ["Bravo", "Charlie"], "a mode filter needs an answer");
    let by = |sort, descending| Filters { sort, descending, ..Default::default() };
    assert_eq!(names(&e, &by(SortKey::Players, true), ""), ["Bravo", "Charlie", "alpha", "Delta", "Echo"]);
    assert_eq!(names(&e, &by(SortKey::Ping, false), ""), ["Charlie", "Bravo", "alpha", "Delta", "Echo"]);
    assert_eq!(names(&e, &by(SortKey::Drop, false), ""), ["alpha", "Charlie", "Bravo", "Delta", "Echo"]);
    assert_eq!(names(&e, &by(SortKey::Name, true), ""), ["Echo", "Delta", "Charlie", "Bravo", "alpha"]);
    assert_eq!(names(&e, &all, "elise"), ["Charlie"], "a player's name");
    assert_eq!(names(&e, &all, "player:mac ~name:bravo"), ["Charlie"]);
    assert_eq!(names(&e, &all, "drop:aoraki"), ["alpha", "Charlie"], "the drop's title");
    assert_eq!(names(&e, &all, "~mode:freeride"), ["alpha", "Delta", "Echo"]);
    assert_eq!(names(&e, &all, "tag:eu"), Vec::<String>::new(), "no server states tags yet");
    assert_eq!(names(&e, &all, "~tag:eu").len(), 5);
    assert_eq!(names(&e, &all, "address:10.0.0"), ["Delta", "Echo"]);
}
