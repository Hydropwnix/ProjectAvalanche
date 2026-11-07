use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use ssx_net_join::JoinFailure;
use ssx_net_join::known::KnownError;
use ssx_net_proto::join::{CHALLENGE_SIZE, JoinAnswer, JoinRequest, MAX_JOIN_BODY, RefusalCode, seal_inner};
use ssx_net_proto::keys::{PlayerSecret, ReplySecret};
use ssx_net_proto::{API_LEVEL, BUILD_ID, PROTOCOL_VERSION};
use zeroize::Zeroizing;

use crate::common::*;

fn from(c: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(127, 0, 5, c))
}

fn manifest_content(server: SocketAddr, local: IpAddr) -> ssx_net_proto::join::JoinContent {
    let (status, body) = get(server, local, "/resources");
    assert_eq!(status, 200, "the manifest");
    ssx_net_proto::join::JoinContent { content_id: None, manifest: Some(ssx_content::ContentHash::of(&body).0) }
}

fn request(server: SocketAddr, local: IpAddr, name: &str, password: &str) -> (JoinRequest, PlayerSecret, ReplySecret) {
    let (status, challenge) = get(server, local, "/challenge");
    assert_eq!(status, 200);
    let challenge: [u8; CHALLENGE_SIZE] = challenge.try_into().expect("40 bytes");
    let (player, reply) = (PlayerSecret::generate(), ReplySecret::generate());
    let request = JoinRequest {
        protocol_version: PROTOCOL_VERSION,
        api_level: API_LEVEL,
        reply: reply.public(),
        challenge,
        player: player.public(),
        server_addr: server,
        name: name.into(),
        build: BUILD_ID.into(),
        password: Zeroizing::new(password.as_bytes().to_vec()),
        content: manifest_content(server, local),
    };
    (request, player, reply)
}

fn answer(server: SocketAddr, local: IpAddr, body: &[u8], reply: &ReplySecret) -> JoinAnswer {
    let (status, answer) = post(server, local, "/join", body);
    assert_eq!(status, 200, "a sealed answer: {}", String::from_utf8_lossy(&answer));
    JoinAnswer::open(&answer, reply).expect("the answer opens with the reply key")
}

fn refused(answer: &JoinAnswer) -> Option<RefusalCode> {
    match answer {
        JoinAnswer::Refused(r) => Some(r.code),
        JoinAnswer::Accepted(_) => None,
        JoinAnswer::Held(h) => panic!("a join held without scripts: {h:?}"),
    }
}

#[test]
fn an_accepted_join_connects() {
    let server = server();
    let addr = server.started.addr;
    let mut player = Player::new();
    let granted = player.join(addr, "  Elise   Riggs ", "").expect("joined");
    assert_eq!(granted.trust, ssx_net_join::Trust::New);
    let (mut app, entity) = udp_client(&granted.token, addr, LOCAL, None);
    assert!(step_until(&mut [&mut app], Duration::from_secs(5), |a| connected(a[0], entity)));
    let ok = step_until(&mut [&mut app], Duration::from_secs(2), |_| {
        let (_, body) = get(addr, from(99), "/info");
        ssx_net_proto::info::ServerInfo::from_json(&body).is_ok_and(|i| i.players == 1)
    });
    assert!(ok, "the info counts the rider");
    assert_eq!(player.join(addr, "Elise", "").expect("joined again").trust, ssx_net_join::Trust::Known);
    server.stop();
    server.join().expect("stops");
}

#[test]
fn malformed_and_oversized_bodies() {
    let server = server();
    let (addr, key) = (server.started.addr, server.started.identity_key);
    let (req, player, _) = request(addr, from(1), "Mac", "");
    let good = req.seal(&key, &player).expect("seal");
    for (i, body) in
        [Vec::new(), vec![0u8; 10], vec![0xAB; MAX_JOIN_BODY], good[..good.len() - 1].to_vec()].iter().enumerate()
    {
        assert_eq!(post(addr, from(2 + i as u8), "/join", body).0, 400, "body {i}");
    }
    for byte in [0usize, 5, 40, good.len() - 1] {
        let mut bad = good.clone();
        bad[byte] ^= 1;
        assert_eq!(post(addr, from(10 + (byte % 200) as u8), "/join", &bad).0, 400, "bit 0 of byte {byte}");
    }
    assert_eq!(post(addr, from(20), "/join", &vec![0u8; MAX_JOIN_BODY + 1]).0, 413);
    server.stop();
    server.join().expect("stops");
}

#[test]
fn a_replayed_join_and_a_foreign_challenge_are_refused() {
    let server = server();
    let (addr, key) = (server.started.addr, server.started.identity_key);
    let (req, player, reply) = request(addr, from(30), "Kaori", "");
    let body = req.seal(&key, &player).expect("seal");
    assert_eq!(refused(&answer(addr, from(30), &body, &reply)), None, "accepted");
    let again = answer(addr, from(30), &body, &reply);
    assert_eq!(refused(&again), Some(RefusalCode::Challenge), "the same body again: {again:?}");
    let (req, player, reply) = request(addr, from(31), "Kaori", "");
    let body = req.seal(&key, &player).expect("seal");
    assert_eq!(refused(&answer(addr, from(32), &body, &reply)), Some(RefusalCode::Challenge));
    server.stop();
    server.join().expect("stops");
}

#[test]
fn forged_signatures_are_refused() {
    let server = server();
    let (addr, key) = (server.started.addr, server.started.identity_key);
    let (req, player, reply) = request(addr, from(40), "Zoe", "");
    let mut inner = req.signed_bytes(&key, &player).expect("inner");
    let last = inner.len() - 1;
    inner[last] ^= 1;
    let a = answer(addr, from(40), &seal_inner(&inner, &key).expect("seal"), &reply);
    assert!(
        matches!(&a, JoinAnswer::Refused(r) if r.code == RefusalCode::Request && r.text.contains("signature")),
        "{a:?}"
    );
    let (mut req, _, reply) = request(addr, from(41), "Zoe", "");
    let (victim, attacker) = (PlayerSecret::generate(), PlayerSecret::generate());
    req.player = attacker.public();
    let mut inner = req.signed_bytes(&key, &attacker).expect("inner");
    let at = 8 + 32 + CHALLENGE_SIZE;
    inner[at..at + 32].copy_from_slice(victim.public().as_bytes());
    let a = answer(addr, from(41), &seal_inner(&inner, &key).expect("seal"), &reply);
    assert_eq!(refused(&a), Some(RefusalCode::Request), "{a:?}");
    server.stop();
    server.join().expect("stops");
}

#[test]
fn protocol_name_and_password() {
    let server = server_with(|c| c.password = Zeroizing::new("secret".into()), None);
    let (addr, key) = (server.started.addr, server.started.identity_key);
    let (mut req, player, reply) = request(addr, from(50), "Mac", "secret");
    req.protocol_version = PROTOCOL_VERSION + 1;
    let a = answer(addr, from(50), &req.seal(&key, &player).expect("seal"), &reply);
    assert!(
        matches!(&a, JoinAnswer::Refused(r) if r.code == RefusalCode::Protocol && r.text.contains("protocol")),
        "{a:?}"
    );
    let (req, player, reply) = request(addr, from(51), "Mac\u{3000}", "secret");
    assert_eq!(
        refused(&answer(addr, from(51), &req.seal(&key, &player).expect("seal"), &reply)),
        Some(RefusalCode::Name)
    );
    for (c, password) in [(52, ""), (53, "Secret"), (54, "secret ")] {
        let (req, player, reply) = request(addr, from(c), "Mac", password);
        let a = answer(addr, from(c), &req.seal(&key, &player).expect("seal"), &reply);
        assert_eq!(refused(&a), Some(RefusalCode::Password), "{password:?}");
    }
    let (req, player, reply) = request(addr, from(55), "Mac", "secret");
    assert_eq!(
        refused(&answer(addr, from(55), &req.seal(&key, &player).expect("seal"), &reply)),
        None,
        "the right password"
    );
    assert!(matches!(Player::new().join(addr, "日本", "secret"), Err(JoinFailure::Name(_))));
    server.stop();
    server.join().expect("stops");
}

#[test]
fn the_join_flood_limit() {
    let server = server();
    let (addr, key) = (server.started.addr, server.started.identity_key);
    let mut codes = Vec::new();
    let mut retry = 0;
    for _ in 0..5 {
        let (req, player, reply) = request(addr, from(60), "Psymon", "");
        let a = answer(addr, from(60), &req.seal(&key, &player).expect("seal"), &reply);
        if let JoinAnswer::Refused(r) = &a {
            retry = r.retry_after_secs;
        }
        codes.push(refused(&a));
        std::thread::sleep(Duration::from_millis(1100));
    }
    assert_eq!(codes, [None, None, None, None, Some(RefusalCode::Flood)], "4 joins in 30 s");
    assert!((1..=8).contains(&retry), "{retry} s");
    server.stop();
    server.join().expect("stops");
}

#[test]
fn a_full_server_refuses_but_the_same_player_may_return() {
    let server = server_with(|c| c.max_players = 2, None);
    let addr = server.started.addr;
    let (mut a, mut b, mut c) = (Player::new(), Player::new(), Player::new());
    let ga = a.join(addr, "A", "").expect("a");
    let gb = b.join(addr, "B", "").expect("b");
    assert!(matches!(c.join(addr, "C", ""), Err(JoinFailure::Refused(r)) if r.code == RefusalCode::Full));
    let (mut app_a, ea) = udp_client(&ga.token, addr, LOCAL, None);
    let (mut app_b, eb) = udp_client(&gb.token, addr, LOCAL, None);
    assert!(step_until(&mut [&mut app_a, &mut app_b], Duration::from_secs(5), |apps| connected(apps[0], ea)
        && connected(apps[1], eb)));
    let (req, player, reply) = request(addr, from(70), "D", "");
    let full = answer(addr, from(70), &req.seal(&server.started.identity_key, &player).expect("seal"), &reply);
    assert_eq!(refused(&full), Some(RefusalCode::Full), "{full:?}");
    assert!(a.join(addr, "A", "").is_ok(), "a connected player may join again (it replaces its connection)");
    server.stop();
    server.join().expect("stops");
}

#[test]
fn a_changed_server_key_is_refused() {
    let first = server();
    let addr = first.started.addr;
    let mut player = Player::new();
    player.join(addr, "Elise", "").expect("pinned");
    first.stop();
    first.join().expect("stops");
    let second = server_with(|c| c.port = addr.port(), None);
    match player.join(addr, "Elise", "") {
        Err(JoinFailure::Key(KnownError::KeyChanged { address, .. })) => assert_eq!(address, addr.to_string()),
        other => panic!("{:?}", other.map(|g| g.info)),
    }
    second.stop();
    second.join().expect("stops");
}

#[test]
fn a_cancelled_join_leaves_nothing_behind() {
    use ssx_net_join::browse::{Host, JoinRequest, JoinTask, ServerAddress};
    let server = server();
    let addr = server.started.addr;
    let home = temp_dir("cancel");
    let request = || JoinRequest {
        address: ServerAddress { host: Host::Ip(addr.ip()), port: addr.port() },
        name: "Elise".into(),
        password: zeroize::Zeroizing::new(Vec::new()),
        app_dir: home.clone(),
    };
    let wait = |task: &JoinTask| {
        let end = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(out) = task.poll() {
                return out;
            }
            assert!(std::time::Instant::now() < end, "the join's thread answers");
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    let with = || ssx_net_join::browse::ResourceJoin::without_content(home.join("ResourceCache"));
    let task = JoinTask::spawn(request(), None, with()).expect("thread");
    task.cancel();
    let out = wait(&task).err().expect("cancelled");
    assert!(out.cancelled, "{out:?}");
    let known = std::fs::read_to_string(home.join(ssx_net_join::KNOWN_SERVERS_FILE)).unwrap_or_default();
    assert!(!known.contains(&addr.to_string()), "no key pinned: {known}");
    let joined = match wait(&JoinTask::spawn(request(), None, with()).expect("thread")) {
        Ok(joined) => joined,
        Err(failed) => panic!("the next join is granted: {failed:?}"),
    };
    assert_eq!(joined.server, addr);
    let known = std::fs::read_to_string(home.join(ssx_net_join::KNOWN_SERVERS_FILE)).expect("pinned now");
    assert!(known.contains(&addr.to_string()));
    server.stop();
    server.join().expect("stops");
}
