use super::*;
use crate::info::INFO_VERSION;
use crate::keys::ServerSecret;

fn answer() -> QueryAnswer {
    QueryAnswer {
        sequence: 0xDEAD_BEEF,
        protocol_version: crate::PROTOCOL_VERSION,
        players: 12,
        max_players: 16,
        mode: 1,
        password: true,
        info_hash: [0xA5; INFO_HASH_SIZE],
    }
}

fn noise(seed: u64, n: usize) -> Vec<u8> {
    let mut x = seed.max(1);
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

#[test]
fn every_message_round_trips() {
    for r in [QueryRequest::Challenge, QueryRequest::Query { token: [7; TOKEN_SIZE], sequence: 42 }] {
        assert_eq!(QueryRequest::decode(&r.encode()), Ok(r));
    }
    let c = Challenge { token: [9; TOKEN_SIZE] };
    assert_eq!(Challenge::decode(&c.encode()), Ok(c));
    let a = answer();
    assert_eq!(QueryAnswer::decode(&a.encode().expect("valid")), Ok(a));
    for p in [LanProbe { protocol: 0 }, LanProbe { protocol: 4 }, LanProbe { protocol: u32::MAX }] {
        assert_eq!(LanProbe::decode(&p.encode()), Ok(p));
    }
    for a in [LanAnswer { protocol: 4, port: 27450 }, LanAnswer { protocol: u32::MAX, port: u16::MAX }] {
        assert_eq!(LanAnswer::decode(&a.encode()), Ok(a));
    }
}

#[test]
fn no_answer_is_larger_than_its_request() {
    let challenge_request = QueryRequest::Challenge.encode();
    let query = QueryRequest::Query { token: [0; TOKEN_SIZE], sequence: u32::MAX }.encode();
    assert_eq!((challenge_request.len(), query.len()), (CHALLENGE_REQUEST_SIZE, QUERY_REQUEST_SIZE));
    let challenge = Challenge { token: [0xFF; TOKEN_SIZE] }.encode();
    assert!(challenge.len() <= challenge_request.len(), "{} > {}", challenge.len(), challenge_request.len());
    let largest = QueryAnswer {
        sequence: u32::MAX,
        protocol_version: u32::MAX,
        players: 64,
        max_players: 64,
        mode: 4,
        password: true,
        info_hash: [0xFF; INFO_HASH_SIZE],
    };
    let a = largest.encode().expect("valid");
    assert_eq!(a.len(), QUERY_ANSWER_SIZE);
    assert!(a.len() <= query.len(), "{} > {}", a.len(), query.len());
    let probe = LanProbe { protocol: 0 }.encode();
    let widest = LanAnswer { protocol: u32::MAX, port: u16::MAX }.encode();
    assert_eq!(widest.len(), MAX_LAN_ANSWER);
    assert!(widest.len() <= probe.len(), "{} > {}", widest.len(), probe.len());
}

#[test]
fn malformed_requests_are_refused() {
    let mut q = QueryRequest::Query { token: [1; TOKEN_SIZE], sequence: 1 }.encode();
    q[QUERY_REQUEST_SIZE - 1] = 1;
    assert!(QueryRequest::decode(&q).is_err(), "padding");
    let mut c = QueryRequest::Challenge.encode();
    c[CHALLENGE_REQUEST_SIZE - 1] = 1;
    assert!(QueryRequest::decode(&c).is_err(), "padding");
    for (at, v) in [(0, b'X'), (4, 2), (5, 2), (5, 3)] {
        let mut c = QueryRequest::Challenge.encode();
        c[at] = v;
        assert!(QueryRequest::decode(&c).is_err(), "byte {at} = {v}");
    }
    let mut long = QueryRequest::Challenge.encode();
    long.resize(QUERY_REQUEST_SIZE, 0);
    assert!(QueryRequest::decode(&long).is_err());
    for n in [0, 1, 31, 33, 63, 65, 1500] {
        assert!(QueryRequest::decode(&vec![0; n]).is_err(), "{n} bytes");
    }
}

#[test]
fn answers_outside_their_rules_are_refused() {
    let good = answer().encode().expect("valid");
    let flip = |at: usize, v: u8| {
        let mut b = good.clone();
        b[at] = v;
        b
    };
    for bad in [
        flip(HEAD + 13, 2),
        flip(HEAD + 12, 0),
        flip(HEAD + 12, 5),
        flip(HEAD + 8, 0x01),
        flip(HEAD + 10, 0x01),
        flip(5, KIND_QUERY),
        good[..QUERY_ANSWER_SIZE - 1].to_vec(),
        [good.clone(), vec![0]].concat(),
    ] {
        assert!(QueryAnswer::decode(&bad).is_err(), "{bad:?}");
    }
    assert!(QueryAnswer { players: 17, ..answer() }.encode().is_err());
    assert!(QueryAnswer { max_players: 0, players: 0, ..answer() }.encode().is_err());
    assert!(QueryAnswer { mode: 9, ..answer() }.encode().is_err());
}

#[test]
fn lan_texts_are_read_strictly() {
    let probe = |text: &str| {
        let mut b = text.as_bytes().to_vec();
        b.resize(LAN_PROBE_SIZE, 0);
        LanProbe::decode(&b)
    };
    assert_eq!(probe("AVALANCHE-CLIENT 4\n"), Ok(LanProbe { protocol: 4 }));
    for bad in [
        "AVALANCHE-CLIENT 04\n",
        "AVALANCHE-CLIENT 4",
        "AVALANCHE-CLIENT  4\n",
        "AVALANCHE-CLIENT +4\n",
        "AVALANCHE-CLIENT 4 \n",
        "AVALANCHE-CLIENT 4294967296\n",
        "avalanche-client 4\n",
        "MTA-CLIENT 4\n",
        "AVALANCHE-CLIENT \n",
    ] {
        assert!(probe(bad).is_err(), "{bad:?}");
    }
    let mut dirty = LanProbe { protocol: 4 }.encode();
    dirty[63] = b'x';
    assert!(LanProbe::decode(&dirty).is_err(), "padding");
    assert!(LanProbe::decode(&LanProbe { protocol: 4 }.encode()[..63]).is_err());
    for bad in [
        "AVALANCHE-SERVER 4 0\n",
        "AVALANCHE-SERVER 4 65536\n",
        "AVALANCHE-SERVER 4 027450\n",
        "AVALANCHE-SERVER 4 27450",
        "AVALANCHE-SERVER 4  27450\n",
        "AVALANCHE-SERVER 4 27450\n\0",
        "AVALANCHE-SERVER 4\n",
        "AVALANCHE-SERVER 4 27450 1\n",
    ] {
        assert!(LanAnswer::decode(bad.as_bytes()).is_err(), "{bad:?}");
    }
    assert!(LanAnswer::decode(&[0xFF; 20]).is_err());
}

#[test]
fn noise_never_panics_and_reads_back_exactly() {
    for seed in 1..400u64 {
        for n in [0, 1, 5, 6, 21, 22, 23, 31, 32, 33, 34, 51, 52, 53, 63, 64, 65, 128] {
            let mut b = noise(seed * 131 + n as u64, n);
            if seed % 3 == 0 && n >= 6 {
                b[..4].copy_from_slice(&QUERY_MAGIC);
                b[4] = QUERY_VERSION;
                b[5] = (seed % 5) as u8;
            }
            if let Ok(r) = QueryRequest::decode(&b) {
                assert_eq!(r.encode(), b);
            }
            if let Ok(c) = Challenge::decode(&b) {
                assert_eq!(c.encode(), b);
            }
            if let Ok(a) = QueryAnswer::decode(&b) {
                assert_eq!(a.encode().expect("a decoded answer encodes"), b);
            }
            if let Ok(p) = LanProbe::decode(&b) {
                assert_eq!(p.encode(), b);
            }
            if let Ok(a) = LanAnswer::decode(&b) {
                assert_eq!(a.encode(), b);
            }
        }
    }
}

#[test]
fn the_info_hash_ignores_the_players_only() {
    let info = ServerInfo {
        info_version: INFO_VERSION,
        name: "Alps Night Riders".into(),
        description: String::new(),
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
        region: String::new(),
        tags: Vec::new(),
        icon: String::new(),
    };
    let h = info_hash(&info);
    assert_eq!(h, info_hash(&ServerInfo { players: 15, ..info.clone() }));
    assert_ne!(h, info_hash(&ServerInfo { name: "Alps Day Riders".into(), ..info.clone() }));
    let mut changed = info.clone();
    changed.resources.manifest = "cd".repeat(32);
    assert_ne!(h, info_hash(&changed));
    let map = crate::instance::MapSummary {
        id: "slope:map/course".into(),
        title: "Template slope".into(),
        event_type: 8,
        gamemode: None,
    };
    assert_ne!(h, info_hash(&ServerInfo { drop: String::new(), map: Some(map), ..info.clone() }));
    assert_ne!(h, info_hash(&ServerInfo { password: true, ..info }));
    assert_eq!(mode_code("freeride"), Some(1));
    assert_eq!(mode_code("survive"), Some(4));
    assert_eq!((mode_code("chess"), mode_name(0), mode_name(5)), (None, None, None));
    for m in MODES {
        assert_eq!(mode_code(m).and_then(mode_name), Some(m));
    }
}
