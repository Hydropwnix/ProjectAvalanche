use super::*;
use crate::info::INFO_VERSION;
use crate::instance::MapSummary;
use crate::resources::ResourceSummary;

fn summary() -> MapSummary {
    MapSummary { id: "slope:map/course".into(), title: "Template slope".into(), event_type: 5, gamemode: None }
}

fn beat() -> Heartbeat {
    Heartbeat {
        heartbeat_version: HEARTBEAT_VERSION,
        port: 27450,
        protocol_version: crate::PROTOCOL_VERSION,
        info_version: INFO_VERSION,
        api_level: crate::API_LEVEL,
        build: crate::BUILD_ID.into(),
        name: "Alps Night Riders".into(),
        mode: "freeride".into(),
        drop: "exp_alps_vorassay_drop_3".into(),
        map: None,
        players: 3,
        max_players: 16,
        password: false,
        region: "eu-central".into(),
        tags: vec!["race".into(), "vanilla".into()],
        contact: "admin@example.org".into(),
        leaving: false,
        resources: crate::resources::summary(),
    }
}

fn challenge() -> (MasterChallenge, OneTimeSecret) {
    let secret = OneTimeSecret::from_bytes(crate::keys::random_bytes());
    let c = MasterChallenge { issued_ms: 1234, nonce: [3; 16], ephemeral: secret.public(), tag: [4; 16] };
    (c, secret)
}

fn info_of(b: &Heartbeat, key: &ServerKey) -> ServerInfo {
    ServerInfo {
        info_version: b.info_version,
        name: b.name.clone(),
        description: String::new(),
        protocol_version: b.protocol_version,
        api_level: b.api_level,
        build: b.build.clone(),
        players: 1,
        max_players: b.max_players,
        password: b.password,
        mode: b.mode.clone(),
        drop: b.drop.clone(),
        map: b.map.clone(),
        rewinds: "on".into(),
        ghosts: false,
        ghost_style: "character".into(),
        identity_key: key.to_hex(),
        resources: b.resources.clone(),
        region: b.region.clone(),
        tags: b.tags.clone(),
        icon: String::new(),
    }
}

#[test]
fn a_heartbeat_opens_only_with_its_keys() {
    let server = ServerSecret::generate();
    let (c, secret) = challenge();
    let body = beat().seal(&server, &c).expect("sealed");
    assert!((MIN_HEARTBEAT_BODY..=MAX_HEARTBEAT_BODY).contains(&body.len()));
    let env = HeartbeatEnvelope::read(&body).expect("reads");
    assert_eq!((env.server, env.challenge), (*server.public().as_bytes(), c));
    let key = ServerKey::from_bytes(env.server).expect("a usable key");
    assert_eq!(env.open(&key, &secret), Ok(beat()));
    let (_, other) = challenge();
    assert!(matches!(env.open(&key, &other), Err(MasterError::Seal(_))));
    let victim = ServerSecret::generate().public();
    assert!(matches!(env.open(&victim, &secret), Err(MasterError::Seal(_))));
    for at in [body.len() - 1, body.len() - 40, MAX_HEARTBEAT_BODY.min(body.len()) - 20] {
        let mut changed = body.clone();
        changed[at] ^= 1;
        let e = HeartbeatEnvelope::read(&changed).expect("reads");
        assert!(e.open(&key, &secret).is_err(), "byte {at}");
    }
    let forger = ServerSecret::generate();
    let mut forged = beat().seal(&forger, &c).expect("sealed");
    forged[ENVELOPE..ENVELOPE + KEY_SIZE].copy_from_slice(victim.as_bytes());
    let e = HeartbeatEnvelope::read(&forged).expect("reads");
    assert_eq!(e.server, *victim.as_bytes());
    assert!(e.open(&victim, &secret).is_err());
}

#[test]
fn the_answer_comes_back_under_the_same_pair_only() {
    let server = ServerSecret::generate();
    let (c, secret) = challenge();
    let a = HeartbeatAnswer { listed: false, reason: "UDP 27451 not reachable: port forwarding?".into() };
    let body = a.seal(&server.public(), &secret).expect("sealed");
    assert!(body.len() <= MAX_ANSWER_BODY);
    assert_eq!(HeartbeatAnswer::open(&body, &server, &c), Ok(a));
    let (c2, _) = challenge();
    assert!(HeartbeatAnswer::open(&body, &server, &c2).is_err(), "another challenge");
    assert!(HeartbeatAnswer::open(&body, &ServerSecret::generate(), &c).is_err(), "another server");
    let reflected = beat().seal(&server, &c).expect("sealed");
    assert!(HeartbeatAnswer::open(&reflected, &server, &c).is_err());
    let mut relabelled = reflected.clone();
    relabelled[..4].copy_from_slice(b"SSXM");
    let cut = [&relabelled[..ENVELOPE], &relabelled[ENVELOPE + KEY_SIZE + MASTER_CHALLENGE_SIZE..]].concat();
    assert!(HeartbeatAnswer::open(&cut, &server, &c).is_err(), "the domains differ");
    let long = HeartbeatAnswer { listed: true, reason: format!("a\u{7}b{}", "é".repeat(400)) };
    let opened =
        HeartbeatAnswer::open(&long.seal(&server.public(), &secret).expect("sealed"), &server, &c).expect("opens");
    assert!(opened.reason.len() <= MAX_REASON_BYTES && opened.reason.starts_with("a b"));
}

#[test]
fn heartbeat_fields_keep_their_rules() {
    assert_eq!(beat().validate(), Ok(()));
    let cases: Vec<(&str, Heartbeat)> = vec![
        ("version", Heartbeat { heartbeat_version: 1, ..beat() }),
        ("resources", Heartbeat { resources: ResourceSummary { count: 300, ..beat().resources }, ..beat() }),
        ("port 0", Heartbeat { port: 0, ..beat() }),
        ("port 65535", Heartbeat { port: u16::MAX, ..beat() }),
        ("name", Heartbeat { name: " padded".into(), ..beat() }),
        ("name", Heartbeat { name: "N".repeat(41), ..beat() }),
        ("name", Heartbeat { name: "日本".into(), ..beat() }),
        ("mode", Heartbeat { mode: "chess".into(), ..beat() }),
        ("drop", Heartbeat { drop: "../etc".into(), ..beat() }),
        ("drop beside", Heartbeat { map: Some(summary()), ..beat() }),
        ("no level", Heartbeat { drop: String::new(), ..beat() }),
        (
            "map id",
            Heartbeat { drop: String::new(), map: Some(MapSummary { id: "x:map/..".into(), ..summary() }), ..beat() },
        ),
        (
            "map title",
            Heartbeat { drop: String::new(), map: Some(MapSummary { title: "a\u{7}".into(), ..summary() }), ..beat() },
        ),
        ("event", Heartbeat { drop: String::new(), map: Some(MapSummary { event_type: 9, ..summary() }), ..beat() }),
        ("players", Heartbeat { players: 17, ..beat() }),
        ("max", Heartbeat { max_players: 65, players: 0, ..beat() }),
        ("region", Heartbeat { region: "EU".into(), ..beat() }),
        ("region", Heartbeat { region: "r".repeat(33), ..beat() }),
        ("tags", Heartbeat { tags: (0..9).map(|i| format!("t{i}")).collect(), ..beat() }),
        ("tag twice", Heartbeat { tags: vec!["race".into(), "race".into()], ..beat() }),
        ("tag", Heartbeat { tags: vec!["".into()], ..beat() }),
        ("tag", Heartbeat { tags: vec!["<b>".into()], ..beat() }),
        ("contact", Heartbeat { contact: "a\nb".into(), ..beat() }),
        ("contact", Heartbeat { contact: "c".repeat(129), ..beat() }),
        ("build", Heartbeat { build: String::new(), ..beat() }),
    ];
    for (what, b) in cases {
        assert!(b.validate().is_err(), "{what}");
        let (c, _) = challenge();
        assert!(b.seal(&ServerSecret::generate(), &c).is_err(), "{what}: not sealed either");
    }
    let server = ServerSecret::generate();
    let (c, secret) = challenge();
    let key = server.public();
    let raw = |json: &str| {
        let k = SalsaBox::new(&ServerKey::from_bytes(c.ephemeral).expect("key").box_key(), server.box_secret());
        let mut head = key.as_bytes().to_vec();
        head.extend_from_slice(&c.to_bytes());
        let body = seal_box(&k, HEARTBEAT_MAGIC, &head, HEARTBEAT_DOMAIN, json.as_bytes()).expect("sealed");
        HeartbeatEnvelope::read(&body).and_then(|e| e.open(&key, &secret))
    };
    let json = String::from_utf8(serde_json::to_vec(&beat()).expect("json")).expect("utf-8");
    assert_eq!(raw(&json), Ok(beat()));
    for bad in [
        json.replacen('}', ",\"sim_content_id\":\"x\"}", 1),
        json.replacen(",\"leaving\":false", "", 1),
        json.replacen("\"port\":27450", "\"port\":\"27450\"", 1),
        json.replacen("\"port\":27450", "\"port\":70000", 1),
        "[]".into(),
        "{}".into(),
    ] {
        assert!(raw(&bad).is_err(), "{bad}");
    }
}

#[test]
fn envelopes_of_any_size_and_bytes_are_refused_without_a_panic() {
    let server = ServerSecret::generate();
    let (c, secret) = challenge();
    let key = server.public();
    for n in [0, 1, 4, 5, MIN_HEARTBEAT_BODY - 1, MAX_HEARTBEAT_BODY + 1, 100_000] {
        assert!(matches!(HeartbeatEnvelope::read(&vec![0; n]), Err(MasterError::Size { .. })), "{n}");
    }
    let mut x = 0x9E37_79B9_7F4A_7C15u64;
    for n in (MIN_HEARTBEAT_BODY..MAX_HEARTBEAT_BODY).step_by(97) {
        let mut b: Vec<u8> = (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x as u8
            })
            .collect();
        b[..4].copy_from_slice(&HEARTBEAT_MAGIC);
        b[4] = ENVELOPE_VERSION;
        let e = HeartbeatEnvelope::read(&b).expect("the layout reads");
        assert!(e.open(&key, &secret).is_err());
        assert!(HeartbeatAnswer::open(&b[..b.len().min(MAX_ANSWER_BODY)], &server, &c).is_err());
    }
}

fn listed(i: u32) -> ListedServer {
    ListedServer {
        address: format!("203.0.113.{}", i % 250 + 1),
        port: 27450 + (i / 250) as u16 * 2,
        key: ServerSecret::generate().public().to_hex(),
        name: format!("Server {i}"),
        mode: "freeride".into(),
        drop: if i % 3 == 1 { String::new() } else { "exp_alps_vorassay_drop_3".into() },
        map: (i % 3 == 1).then(summary),
        players: i % 16,
        max_players: 16,
        password: i.is_multiple_of(7),
        region: String::new(),
        tags: vec![],
        build: crate::BUILD_ID.into(),
        api_level: 0,
        info_version: INFO_VERSION,
        info_hash: to_hex(&[0xAB; 32]),
        resources: crate::resources::summary(),
    }
}

#[test]
fn the_list_is_read_strictly() {
    let p = crate::PROTOCOL_VERSION;
    let list = ServerList { list_version: LIST_VERSION, protocol_version: p, servers: (0..20).map(listed).collect() };
    let json = list.to_json();
    assert_eq!(ServerList::from_json(&json, p), Ok(list.clone()));
    assert!(ServerList::from_json(&json, p + 1).is_err(), "another protocol");
    let text = String::from_utf8(json).expect("utf-8");
    let first = &list.servers[0];
    for bad in [
        text.replacen(
            &format!("\"list_version\":{LIST_VERSION}"),
            &format!("\"list_version\":{}", LIST_VERSION + 1),
            1,
        ),
        text.replacen(",\"map\":null", "", 1),
        text.replacen("slope:map/course", "slope:map/Course", 1),
        text.replacen('}', ",\"contact\":\"x\"}", 1),
        text.replacen(&first.key, &first.key.to_uppercase(), 1),
        text.replacen(&first.key, &list.servers[1].key, 1),
        text.replacen("203.0.113.1\"", "::ffff:203.0.113.1\"", 1),
        text.replacen("203.0.113.1\"", "0.0.0.0\"", 1),
        text.replacen("203.0.113.1\"", "224.0.0.1\"", 1),
        text.replacen("203.0.113.1\"", "localhost\"", 1),
        text.replacen("203.0.113.2\"", "203.0.113.1\"", 1),
        text.replacen("Server 0", "Server\\u0007", 1),
        text.replacen("\"players\":0", "\"players\":17", 1),
        text.replacen("\"port\":27450", "\"port\":65535", 1),
        text.replacen("\"tags\":[]", "\"tags\":[\"A\"]", 1),
    ] {
        assert!(ServerList::from_json(bad.as_bytes(), p).is_err(), "{}", &bad[..bad.len().min(200)]);
    }
    let many = ServerList {
        list_version: LIST_VERSION,
        protocol_version: p,
        servers: (0..=MAX_LIST_SERVERS as u32).map(|i| ListedServer { key: to_hex(&[0; 32]), ..listed(i) }).collect(),
    };
    let e = ServerList::from_json(&many.to_json(), p).expect_err("one too many");
    assert!(e.contains("more than 10000"), "{e}");
    let mut big = vec![b' '; MAX_LIST_BYTES + 1];
    big[0] = b'{';
    assert!(ServerList::from_json(&big, p).expect_err("too big").contains("at most"));
}

#[test]
fn the_blocklist_is_read_strictly() {
    let b =
        Blocklist { blocklist_version: BLOCKLIST_VERSION, resources: vec![to_hex(&[0xAB; 32]), to_hex(&[0xCD; 32])] };
    assert_eq!(Blocklist::from_json(&b.to_json()), Ok(b.clone()));
    let text = String::from_utf8(b.to_json()).expect("utf-8");
    for bad in [
        text.replacen("\"blocklist_version\":1", "\"blocklist_version\":0", 1),
        text.replacen(&b.resources[0], &b.resources[0].to_uppercase(), 1),
        text.replacen(&b.resources[0], "zz", 1),
        text.replacen('}', ",\"keys\":[]}", 1),
    ] {
        assert!(Blocklist::from_json(bad.as_bytes()).is_err(), "{bad}");
    }
}

#[test]
fn a_heartbeat_against_its_info() {
    let key = ServerSecret::generate().public();
    let b = beat();
    assert_eq!(heartbeat_against_info(&b, &key, &info_of(&b, &key)), None);
    let other = ServerSecret::generate().public();
    assert!(heartbeat_against_info(&b, &key, &info_of(&b, &other)).is_some_and(|w| w.contains("identity key")));
    type Lie = Box<dyn Fn(&mut ServerInfo)>;
    let lies: Vec<(&str, Lie)> = vec![
        ("name", Box::new(|i| i.name = "Another".into())),
        ("protocol", Box::new(|i| i.protocol_version += 1)),
        ("API level", Box::new(|i| i.api_level += 1)),
        ("build", Box::new(|i| i.build = "other".into())),
        ("mode", Box::new(|i| i.mode = "race".into())),
        ("drop", Box::new(|i| i.drop = "exp_x".into())),
        ("max players", Box::new(|i| i.max_players = 8)),
        ("password", Box::new(|i| i.password = true)),
        ("info version", Box::new(|i| i.info_version = 2)),
        ("other resources", Box::new(|i| i.resources.download += 1)),
        ("other resources", Box::new(|i| i.resources.content_id = String::new())),
        ("region", Box::new(|i| i.region = "us-east".into())),
        ("tags", Box::new(|i| i.tags.reverse())),
    ];
    for (what, lie) in lies {
        let mut i = info_of(&b, &key);
        lie(&mut i);
        let said = heartbeat_against_info(&b, &key, &i).unwrap_or_default();
        assert!(said.contains(what), "{what}: {said}");
    }
    let on_map = Heartbeat { drop: String::new(), map: Some(summary()), ..beat() };
    assert_eq!(heartbeat_against_info(&on_map, &key, &info_of(&on_map, &key)), None);
    for lie in [
        MapSummary { id: "slope:map/other".into(), ..summary() },
        MapSummary { title: "Template slope!".into(), ..summary() },
        MapSummary { event_type: 6, ..summary() },
    ] {
        let i = ServerInfo { map: Some(lie), ..info_of(&on_map, &key) };
        assert!(heartbeat_against_info(&on_map, &key, &i).is_some_and(|w| w.contains("map")));
    }
    let mut moved = info_of(&b, &key);
    moved.players = 9;
    assert_eq!(heartbeat_against_info(&b, &key, &moved), None);
}

#[test]
fn master_addresses_are_read_strictly_and_written_one_way() {
    let ok = |t: &str| MasterUrl::parse(t).map(|u| u.to_string());
    assert_eq!(ok("https://Master.Example.org"), Ok("https://master.example.org".into()));
    assert_eq!(ok("HTTPS://master.example.org:443/"), Ok("https://master.example.org".into()));
    assert_eq!(ok("https://master.example.org:8443"), Ok("https://master.example.org:8443".into()));
    assert_eq!(ok("http://127.0.0.1:27900"), Ok("http://127.0.0.1:27900".into()));
    assert_eq!(ok("http://[::1]:27900"), Ok("http://[::1]:27900".into()));
    assert_eq!(ok("http://192.0.2.1"), Ok("http://192.0.2.1".into()));
    let u = MasterUrl::parse("https://[2001:db8::1]:8443").expect("v6");
    assert_eq!((u.host_text(), u.authority()), ("2001:db8::1".into(), "[2001:db8::1]:8443".into()));
    for bad in [
        "",
        "master.example.org",
        "ftp://master.example.org",
        "https://",
        "https://user@master.example.org",
        "https://master.example.org/v1/servers",
        "https://master.example.org?x=1",
        "https://master.example.org#a",
        "https://master%2eexample.org",
        "https://master example.org",
        "https://master.example.org:0",
        "https://master.example.org:65536",
        "https://master.example.org:080",
        "https://-master.example.org",
        "https://master..example.org",
        "https://1.2.3",
        "https://::1",
        "https://[::1",
        "https://0.0.0.0",
        "https://[ff02::1]",
        "https://mäster.example.org",
        "https://master.example.org\\x",
        "javascript://alert(1)",
    ] {
        assert!(MasterUrl::parse(bad).is_err(), "{bad:?}");
    }
    assert!(MasterUrl::parse(&format!("https://{}.org", "a".repeat(260))).is_err());
}

#[test]
fn the_widest_heartbeat_fits_its_cap() {
    let json_len = |s: &str| serde_json::to_string(s).map_or(0, |j| j.len());
    let widest = (0..=0x10_FFFF_u32)
        .filter_map(char::from_u32)
        .filter(|c| crate::name::server_name(&c.to_string()).is_ok_and(|n| n == c.to_string()))
        .max_by_key(|c| json_len(&c.to_string()))
        .expect("a drawable character");
    let widest_ascii = (0x20u8..=0x7E).map(char::from).max_by_key(|c| json_len(&c.to_string())).expect("ASCII");
    let mode = crate::query::MODES.iter().max_by_key(|m| m.len()).expect("modes");
    let mut b = beat();
    b.port = 65534;
    (b.protocol_version, b.info_version, b.api_level) = (u32::MAX, u32::MAX, u32::MAX);
    b.build = widest_ascii.to_string().repeat(MAX_BUILD_BYTES);
    b.name = widest.to_string().repeat(crate::name::MAX_SERVER_NAME_CHARS);
    b.mode = (*mode).into();
    b.drop = "d".repeat(crate::instance::MAX_DROP_NAME);
    (b.players, b.max_players) = (MAX_ANNOUNCED_PLAYERS, MAX_ANNOUNCED_PLAYERS);
    b.password = false;
    b.region = "r".repeat(MAX_REGION_CHARS);
    b.tags = (0..MAX_TAGS).map(|i| format!("{i}{}", "t".repeat(MAX_TAG_CHARS - 1))).collect();
    b.contact = widest_ascii.to_string().repeat(MAX_CONTACT_CHARS);
    b.leaving = false;
    b.drop = String::new();
    b.map = Some(MapSummary {
        id: format!("a:map/{}", "c".repeat(crate::instance::MAX_MAP_ID - 6)),
        title: "\u{1D11E}".repeat(crate::instance::MAX_MAP_TITLE_CHARS),
        event_type: 4,
        gamemode: Some(crate::instance::GamemodeSummary {
            id: format!("a:gamemode/{}", "c".repeat(crate::instance::MAX_MAP_ID - 11)),
            title: "\u{1D11E}".repeat(crate::instance::MAX_MAP_TITLE_CHARS),
        }),
    });
    b.resources = ResourceSummary {
        content_id: "4e".repeat(32),
        client_mods: "presentation".into(),
        client_scripts: false,
        count: crate::resources::MAX_RESOURCES,
        download: crate::resources::MAX_DOWNLOAD,
        manifest: "ab".repeat(32),
        manifest_bytes: crate::resources::MAX_MANIFEST_BYTES,
    };
    assert_eq!(b.validate(), Ok(()));
    let json = serde_json::to_vec(&b).expect("json");
    assert!(json.len() <= MAX_HEARTBEAT_JSON, "{} > {MAX_HEARTBEAT_JSON}", json.len());
    assert_eq!(json.len(), 2_394, "the widest heartbeat (measured; stated at MAX_HEARTBEAT_JSON)");
    let server = ServerSecret::generate();
    let (c, secret) = challenge();
    let body = b.seal(&server, &c).expect("sealed");
    assert!(body.len() <= MAX_HEARTBEAT_BODY);
    let key = ServerKey::from_bytes(*server.public().as_bytes()).expect("a usable key");
    assert_eq!(HeartbeatEnvelope::read(&body).expect("reads").open(&key, &secret), Ok(b));
}
