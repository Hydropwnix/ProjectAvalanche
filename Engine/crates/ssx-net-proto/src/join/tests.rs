use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};

use super::*;
use crate::keys::{PlayerSecret, ReplySecret, ServerSecret};
use crate::{API_LEVEL, PROTOCOL_VERSION};

struct Noise(u64);

impl Noise {
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n)
            .map(|_| {
                self.0 ^= self.0 << 13;
                self.0 ^= self.0 >> 7;
                self.0 ^= self.0 << 17;
                (self.0 >> 24) as u8
            })
            .collect()
    }
}

fn request(player: &PlayerSecret, reply: &ReplySecret) -> JoinRequest {
    JoinRequest {
        protocol_version: PROTOCOL_VERSION,
        api_level: API_LEVEL,
        reply: reply.public(),
        challenge: [7; CHALLENGE_SIZE],
        player: player.public(),
        server_addr: SocketAddr::from((Ipv4Addr::LOCALHOST, 27450)),
        name: "Mac".into(),
        build: "project-avalanche/0.1.0".into(),
        password: Zeroizing::new(b"hunter2".to_vec()),
        content: JoinContent { content_id: Some([0x4E; 32]), manifest: Some([0xAB; 32]) },
    }
}

fn low_order_points() -> Vec<[u8; 32]> {
    let mut points = vec![[0u8; 32], {
        let mut p = [0u8; 32];
        p[0] = 1;
        p
    }];
    for hex in [
        "e0eb7a7c3b41b8ae1656e3faf19fc46ada098deb9c32b1fd866205165f49b800",
        "5f9c95bca3508c24b1d0b1559c83ef5b04445cc4581c8e86d8224eddd09f1157",
        "ecffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
        "edffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
        "eeffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
    ] {
        points.push(crate::keys::from_hex(hex).expect("hex"));
    }
    points
}

#[test]
fn a_sealed_request_opens_verifies_and_answers() {
    let (server, player, reply) = (ServerSecret::generate(), PlayerSecret::generate(), ReplySecret::generate());
    let mut req = request(&player, &reply);
    for addr in [SocketAddr::from((Ipv4Addr::new(192, 0, 2, 7), 27450)), SocketAddr::from((Ipv6Addr::LOCALHOST, 1))] {
        req.server_addr = addr;
        let body = req.seal(&server.public(), &player).expect("seal");
        assert!((MIN_JOIN_BODY..=MAX_JOIN_BODY).contains(&body.len()));
        let opened = open_join(&body, &server).expect("open");
        let got = opened.request(&server.public()).expect("request");
        assert_eq!((got.player, got.server_addr, got.name.as_str()), (player.public(), addr, "Mac"));
        assert_eq!(&got.password[..], b"hunter2");
        assert_eq!(got.reply.as_bytes(), reply.public().as_bytes());
    }
    for answer in [
        JoinAnswer::Accepted(Box::new([0xA5; TOKEN_SIZE])),
        JoinAnswer::Refused(Refusal::new(RefusalCode::Flood, 17, "4 joins in 30 s".into())),
        JoinAnswer::Refused(Refusal::banned("cheating".into(), Expiry::from_wire(1_794_076_500))),
        JoinAnswer::Refused(Refusal::banned(String::new(), Expiry::Never)),
    ] {
        let body = answer.seal(&reply.public()).expect("seal");
        assert!(body.len() <= MAX_ANSWER_BODY);
        assert_eq!(JoinAnswer::open(&body, &reply), Ok(answer.clone()));
        assert!(JoinAnswer::open(&body, &ReplySecret::generate()).is_err(), "another reply key cannot read it");
    }
}

#[test]
fn forged_and_replayed_requests_are_refused() {
    let (server, player, reply) = (ServerSecret::generate(), PlayerSecret::generate(), ReplySecret::generate());
    let other_server = ServerSecret::generate();
    let mallory = PlayerSecret::generate();
    let mut req = request(&player, &reply);
    req.player = mallory.public();
    assert!(req.seal(&server.public(), &player).is_err(), "the sealer refuses a foreign key");
    let mut inner = Zeroizing::new(Vec::new());
    inner.extend_from_slice(&PROTOCOL_VERSION.to_be_bytes());
    inner.extend_from_slice(&API_LEVEL.to_be_bytes());
    inner.extend_from_slice(reply.public().as_bytes());
    inner.extend_from_slice(&[7; CHALLENGE_SIZE]);
    inner.extend_from_slice(mallory.public().as_bytes());
    put_addr(&mut inner, SocketAddr::from((Ipv4Addr::LOCALHOST, 27450)));
    for (field, max) in [(&b"Mac"[..], MAX_NAME_BYTES), (b"b", MAX_BUILD_BYTES), (b"", MAX_PASSWORD_BYTES)] {
        put_field(&mut inner, field, 0, max, "f").expect("field");
    }
    inner.push(0);
    let signature = player.sign(&signed_message(&server.public(), &inner));
    inner.extend_from_slice(&signature);
    let body = seal_envelope(JOIN_MAGIC, &server.public().box_key(), &inner).expect("seal");
    let opened = open_join(&body, &server).expect("the seal opens");
    assert_eq!(opened.request(&server.public()).err(), Some(JoinError::Signature));
    let body = request(&player, &reply).seal(&other_server.public(), &player).expect("seal");
    let inner = open_join(&body, &other_server).expect("open").0;
    let body = seal_envelope(JOIN_MAGIC, &server.public().box_key(), &inner).expect("reseal");
    assert_eq!(open_join(&body, &server).expect("open").request(&server.public()).err(), Some(JoinError::Signature));
    let good = request(&player, &reply).seal(&server.public(), &player).expect("seal");
    let inner = open_join(&good, &server).expect("open").0;
    for i in 0..inner.len() {
        let mut bad = inner.to_vec();
        bad[i] ^= 0x01;
        let body = seal_envelope(JOIN_MAGIC, &server.public().box_key(), &bad).expect("seal");
        if let Ok(opened) = open_join(&body, &server) {
            assert!(opened.request(&server.public()).is_err(), "bit 0 of byte {i}");
        }
    }
}

#[test]
fn hostile_bodies_never_panic() {
    let server = ServerSecret::generate();
    let (player, reply) = (PlayerSecret::generate(), ReplySecret::generate());
    let good = request(&player, &reply).seal(&server.public(), &player).expect("seal");
    for len in 0..good.len() {
        assert!(open_join(&good[..len], &server).is_err(), "truncated to {len}");
    }
    for i in 0..good.len() {
        let mut bad = good.clone();
        bad[i] ^= 0x80;
        assert!(open_join(&bad, &server).is_err(), "byte {i} flipped: the seal or the envelope refuses it");
    }
    let mut noise = Noise(0x9E37_79B9_7F4A_7C15);
    for len in 0..=MAX_JOIN_BODY + 64 {
        let mut body = noise.bytes(len);
        if len >= 5 {
            body[..4].copy_from_slice(&JOIN_MAGIC);
            body[4] = ENVELOPE_VERSION;
        }
        assert!(open_join(&body, &server).is_err());
        assert!(JoinAnswer::open(&body, &reply).is_err());
    }
    for len in 0..MAX_INNER + 32 {
        for _ in 0..20 {
            let inner = noise.bytes(len);
            let body = seal_envelope(JOIN_MAGIC, &server.public().box_key(), &inner).expect("seal");
            if let Ok(opened) = open_join(&body, &server) {
                let _ = opened.prefix();
                assert!(opened.request(&server.public()).is_err(), "random bytes never verify");
            }
            let answer = seal_envelope(ANSWER_MAGIC, &reply.public().box_key(), &inner).expect("seal");
            if let Ok(JoinAnswer::Refused(r)) = JoinAnswer::open(&answer, &reply) {
                assert!(!r.text.chars().any(char::is_control));
            }
        }
    }
}

#[test]
fn low_order_and_weak_keys_are_refused() {
    for p in low_order_points() {
        assert!(ServerKey::from_bytes(p).is_err(), "{p:02x?}");
        assert!(ReplyKey::from_bytes(p).is_err(), "{p:02x?}");
    }
    let mut identity = [0u8; 32];
    identity[0] = 1;
    assert!(PlayerKey::from_bytes(identity).is_err());
    assert!(ServerKey::from_bytes(*ServerSecret::generate().public().as_bytes()).is_ok());
}

#[test]
fn refusal_texts_are_bounded_and_clean() {
    let reply = ReplySecret::generate();
    let long = JoinAnswer::Refused(Refusal::new(RefusalCode::Busy, 0, "€\n".repeat(200)));
    match JoinAnswer::open(&long.seal(&reply.public()).expect("seal"), &reply).expect("open") {
        JoinAnswer::Refused(r) => {
            assert!(r.text.len() <= MAX_REFUSAL_TEXT && !r.text.contains('\n'));
        }
        JoinAnswer::Accepted(_) | JoinAnswer::Held(_) => panic!("a refusal"),
    }
    let mut inner = vec![JoinAnswer::REFUSED, RefusalCode::Busy as u8, 0, 0, 2, b'a', 0x07];
    let body = seal_envelope(ANSWER_MAGIC, &reply.public().box_key(), &inner).expect("seal");
    assert!(JoinAnswer::open(&body, &reply).is_err(), "a control character from the server");
    inner[1] = 0;
    let body = seal_envelope(ANSWER_MAGIC, &reply.public().box_key(), &inner).expect("seal");
    assert!(JoinAnswer::open(&body, &reply).is_err(), "an unknown code");
}

#[test]
fn the_content_fields_read_back_and_refuse_what_they_do_not_state() {
    let (server, player, reply) = (ServerSecret::generate(), PlayerSecret::generate(), ReplySecret::generate());
    let cases = [
        JoinContent::default(),
        JoinContent { content_id: Some([1; 32]), manifest: None },
        JoinContent { content_id: None, manifest: Some([2; 32]) },
        JoinContent { content_id: Some([3; 32]), manifest: Some([4; 32]) },
    ];
    for content in cases {
        let req = JoinRequest { content, ..request(&player, &reply) };
        let body = req.seal(&server.public(), &player).expect("seal");
        assert!((MIN_JOIN_BODY..=MAX_JOIN_BODY).contains(&body.len()));
        let got = open_join(&body, &server).expect("open").request(&server.public()).expect("request");
        assert_eq!(got.content, content);
    }
    let widest = JoinRequest {
        name: "n".repeat(MAX_NAME_BYTES),
        build: "b".repeat(MAX_BUILD_BYTES),
        password: Zeroizing::new(vec![b'p'; MAX_PASSWORD_BYTES]),
        ..request(&player, &reply)
    };
    assert_eq!(widest.seal(&server.public(), &player).expect("seal").len(), MAX_JOIN_BODY);
    let unsigned = |content: &[u8]| {
        let mut inner = request(&player, &reply).signed_bytes(&server.public(), &player).expect("bytes").to_vec();
        inner.truncate(inner.len() - 64 - JoinContent::MAX_SIZE);
        inner.extend_from_slice(content);
        let signature = player.sign(&signed_message(&server.public(), &inner));
        inner.extend_from_slice(&signature);
        let body = seal_inner(&inner, &server.public()).expect("seal");
        open_join(&body, &server).and_then(|o| o.request(&server.public()))
    };
    assert!(unsigned(&[0]).is_ok_and(|r| r.content == JoinContent::default()));
    let mut both = vec![3u8];
    both.extend_from_slice(&[9; 64]);
    assert!(unsigned(&both).is_ok());
    for bad in [&[4u8][..], &[0x80], &[1], &[1, 0, 0], &[2; 32], &both[..64], &[0, 0]] {
        assert!(unsigned(bad).is_err(), "{bad:02x?}");
    }
    for code in [RefusalCode::ResourcesChanged, RefusalCode::Content] {
        let answer = JoinAnswer::Refused(Refusal::new(code, 0, "content differs".into()));
        assert_eq!(JoinAnswer::open(&answer.seal(&reply.public()).expect("seal"), &reply), Ok(answer));
    }
}

#[test]
fn a_bans_end_belongs_to_banned_alone() {
    let reply = ReplySecret::generate();
    let mut other = Refusal::new(RefusalCode::Full, 0, "full".into());
    other.ban = Some(Expiry::Never);
    assert!(JoinAnswer::Refused(other).seal(&reply.public()).is_err());
    let mut bare = Refusal::banned("x".into(), Expiry::Never);
    bare.ban = None;
    assert!(JoinAnswer::Refused(bare).seal(&reply.public()).is_err());
    let open = |inner: &[u8]| {
        let body = seal_envelope(ANSWER_MAGIC, &reply.public().box_key(), inner).expect("seal");
        JoinAnswer::open(&body, &reply)
    };
    let banned = RefusalCode::Banned as u8;
    let until = 1_794_076_500u64.to_be_bytes();
    let mut good = vec![JoinAnswer::REFUSED, banned, 0, 0];
    good.extend_from_slice(&until);
    good.extend_from_slice(&[2, b'h', b'i']);
    match open(&good) {
        Ok(JoinAnswer::Refused(r)) => {
            assert_eq!(
                (r.code, r.ban, r.text.as_str()),
                (RefusalCode::Banned, Some(Expiry::from_wire(1_794_076_500)), "hi")
            );
        }
        other => panic!("{other:?}"),
    }
    for cut in 4..good.len() {
        assert!(open(&good[..cut]).is_err(), "cut at {cut}");
    }
    let mut full = vec![JoinAnswer::REFUSED, RefusalCode::Full as u8, 0, 0];
    full.extend_from_slice(&until);
    full.extend_from_slice(&[0]);
    assert!(open(&full).is_err(), "eight more bytes on another code");
}

#[test]
fn a_held_answer_reads_back_and_refuses_what_it_does_not_allow() {
    let reply = ReplySecret::generate();
    let hold = |text: &str, ask_after_ms| Hold { ticket: [0x5A; HOLD_TICKET_SIZE], ask_after_ms, text: text.into() };
    for answer in [hold("", MIN_ASK_AFTER_MS), hold("the board looks you up", 1000), hold("€", MAX_ASK_AFTER_MS)] {
        let answer = JoinAnswer::Held(answer);
        let body = answer.seal(&reply.public()).expect("seal");
        assert_eq!(JoinAnswer::open(&body, &reply), Ok(answer));
    }
    for pause in [0, MIN_ASK_AFTER_MS - 1, MAX_ASK_AFTER_MS + 1, u16::MAX] {
        assert!(JoinAnswer::Held(hold("", pause)).seal(&reply.public()).is_err(), "pause {pause}");
    }
    let hostile = format!("a\u{202E}b\u{7}c\u{200B}{}", "€".repeat(200));
    let body = JoinAnswer::Held(hold(&hostile, 1000)).seal(&reply.public()).expect("seal");
    let Ok(JoinAnswer::Held(read)) = JoinAnswer::open(&body, &reply) else { panic!("a held answer") };
    assert!(read.text.starts_with("abc€") && read.text.len() <= MAX_HOLD_TEXT, "{:?}", read.text);
    assert!(!read.text.chars().any(ssx_core::text::is_hidden));
    assert_eq!(read.text, hold_text(&hostile));
    let inner = |pause: u16, text: &[u8], len: u16| {
        let mut v = vec![JoinAnswer::HELD];
        v.extend_from_slice(&[1; HOLD_TICKET_SIZE]);
        v.extend_from_slice(&pause.to_be_bytes());
        v.extend_from_slice(&len.to_be_bytes());
        v.extend_from_slice(text);
        seal_envelope(ANSWER_MAGIC, &reply.public().box_key(), &v).expect("seal")
    };
    assert!(JoinAnswer::open(&inner(1000, b"ok", 2), &reply).is_ok());
    for (pause, text, len, why) in [
        (1000, &b"a\x07"[..], 2, "a control character"),
        (1000, "a\u{2066}".as_bytes(), 4, "an isolate"),
        (1000, &b"\xff"[..], 1, "not UTF-8"),
        (1000, &b"ok"[..], 3, "a length past the end"),
        (1000, &b"okk"[..], 2, "bytes after the text"),
        (100, &b"ok"[..], 2, "a pause too short"),
        (1000, &[b'x'; MAX_HOLD_TEXT + 1][..], MAX_HOLD_TEXT as u16 + 1, "a text too long"),
    ] {
        assert!(JoinAnswer::open(&inner(pause, text, len), &reply).is_err(), "{why}");
    }
    assert!(!format!("{:?}", hold("t", 1000)).contains("90"), "the ticket is not shown");
}

#[test]
fn a_held_ask_opens_with_the_servers_secret_only() {
    let (server, reply) = (ServerSecret::generate(), ReplySecret::generate());
    let ask = HeldAsk { ticket: [9; HOLD_TICKET_SIZE], reply: reply.public() };
    let body = ask.seal(&server.public()).expect("seal");
    assert_eq!(body.len(), HELD_ASK_BODY);
    assert!(HeldAsk::open(&body, &server) == Ok(ask.clone()));
    assert!(HeldAsk::open(&body, &ServerSecret::generate()).is_err());
    for len in 0..body.len() {
        assert!(HeldAsk::open(&body[..len], &server).is_err(), "truncated to {len}");
    }
    for i in 0..body.len() {
        let mut bad = body.clone();
        bad[i] ^= 0x01;
        assert!(HeldAsk::open(&bad, &server).is_err(), "byte {i} flipped");
    }
    for point in low_order_points() {
        let mut inner = vec![9; HOLD_TICKET_SIZE];
        inner.extend_from_slice(&point);
        let body = seal_envelope(HELD_MAGIC, &server.public().box_key(), &inner).expect("seal");
        assert!(HeldAsk::open(&body, &server).is_err(), "a low-order reply key");
    }
    let mut longer = vec![9; HOLD_TICKET_SIZE + KEY_SIZE + 1];
    longer[HOLD_TICKET_SIZE..HOLD_TICKET_SIZE + KEY_SIZE].copy_from_slice(reply.public().as_bytes());
    let body = seal_envelope(HELD_MAGIC, &server.public().box_key(), &longer).expect("seal");
    assert!(HeldAsk::open(&body, &server).is_err(), "a longer inner");
}
