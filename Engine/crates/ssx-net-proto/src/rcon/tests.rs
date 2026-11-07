use super::*;
use crate::keys::{ReplySecret, ServerSecret};

fn request(operator: &OperatorSecret, reply: &ReplySecret, command: &str) -> RconRequest {
    RconRequest {
        rcon_version: RCON_VERSION,
        reply: reply.public(),
        challenge: [9; CHALLENGE_SIZE],
        operator: operator.public(),
        command: command.into(),
    }
}

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

#[test]
fn a_sealed_request_opens_verifies_and_answers() {
    let (server, operator, reply) = (ServerSecret::generate(), OperatorSecret::generate(), ReplySecret::generate());
    for command in ["status", "ban key:00 1d \"Zoë\" cheats", &"x".repeat(MAX_COMMAND_BYTES)] {
        let body = request(&operator, &reply, command).seal(&server.public(), &operator).expect("seal");
        assert!((MIN_REQUEST_BODY..=MAX_REQUEST_BODY).contains(&body.len()));
        let got = open_rcon(&body, &server).expect("open").request(&server.public()).expect("request");
        assert_eq!((got.operator, got.command.as_str(), got.challenge), (operator.public(), command, [9; 40]));
        assert_eq!(got.reply.as_bytes(), reply.public().as_bytes());
        assert!(open_rcon(&body, &ServerSecret::generate()).is_err(), "another server cannot open it");
    }
    let answers = [
        RconAnswer::ran(true, ["3 riders".to_string(), "  1  Mac".into()]),
        RconAnswer::ran(false, ["no rider matches \"x\"".to_string()]),
        RconAnswer::ran(true, Vec::new()),
        RconAnswer::refused(RconRefusalCode::Denied, 0, "the operator's groups lack command.ban"),
        RconAnswer::refused(RconRefusalCode::Limited, 30, "5 failures in 60 s"),
    ];
    for answer in answers {
        let body = answer.seal(&reply.public()).expect("seal");
        assert!(body.len() <= MAX_ANSWER_BODY);
        assert_eq!(RconAnswer::open(&body, &reply), Ok(answer.clone()));
        assert!(RconAnswer::open(&body, &ReplySecret::generate()).is_err(), "another reply key cannot read it");
    }
}

#[test]
fn forged_altered_and_foreign_requests_are_refused() {
    let (server, operator, reply) = (ServerSecret::generate(), OperatorSecret::generate(), ReplySecret::generate());
    let mallory = OperatorSecret::generate();
    let mut r = request(&operator, &reply, "stop");
    r.operator = mallory.public();
    assert!(r.seal(&server.public(), &operator).is_err(), "the sealer refuses a foreign key");
    let inner = request(&operator, &reply, "kick 3").signed_bytes(&server.public(), &operator).expect("signed");
    for i in 0..inner.len() {
        let mut bad = inner.to_vec();
        bad[i] ^= 0x01;
        let body = seal_rcon_inner(&bad, &server.public()).expect("seal");
        assert!(open_rcon(&body, &server).expect("opens").request(&server.public()).is_err(), "byte {i}");
    }
    let elsewhere = ServerSecret::generate();
    let inner = request(&operator, &reply, "stop").signed_bytes(&elsewhere.public(), &operator).expect("signed");
    let body = seal_rcon_inner(&inner, &server.public()).expect("seal");
    assert_eq!(open_rcon(&body, &server).expect("opens").request(&server.public()).err(), Some(JoinError::Signature));
    let mut body = request(&operator, &reply, "stop").seal(&server.public(), &operator).expect("seal");
    let last = body.len() - 1;
    body[last] ^= 1;
    assert!(open_rcon(&body, &server).is_err());
    body[..4].copy_from_slice(b"SSXJ");
    assert!(matches!(open_rcon(&body, &server), Err(JoinError::Envelope(_))));
    for bad in ["", "   ", "status\nstop", "say \u{7}", &"x".repeat(MAX_COMMAND_BYTES + 1)] {
        assert!(request(&operator, &reply, bad).seal(&server.public(), &operator).is_err(), "{bad:?}");
    }
}

#[test]
fn hostile_bodies_never_panic() {
    let (server, reply) = (ServerSecret::generate(), ReplySecret::generate());
    let mut noise = Noise(0x9E37_79B9_7F4A_7C15);
    for len in [0, 1, 4, 5, 52, 53, MIN_REQUEST_BODY - 1, MIN_REQUEST_BODY, 400, MAX_REQUEST_BODY, MAX_REQUEST_BODY + 1]
    {
        for _ in 0..20 {
            let mut body = noise.bytes(len);
            if body.len() >= 5 {
                body[..4].copy_from_slice(b"SSXC");
                body[4] = 1;
            }
            assert!(open_rcon(&body, &server).is_err());
            assert!(RconAnswer::open(&body, &reply).is_err());
        }
    }
    for len in 0..(MAX_INNER + 8) {
        let body = seal_rcon_inner(&noise.bytes(len), &server.public()).expect("seal");
        if let Ok(opened) = open_rcon(&body, &server) {
            assert!(opened.request(&server.public()).is_err(), "{len}");
        }
    }
    let open = |inner: &[u8]| {
        RconAnswer::open(&seal_envelope(ANSWER_MAGIC, &reply.public().box_key(), inner).unwrap(), &reply)
    };
    for bad in [
        &[1u8, 1, 0xFF, 0xFF][..],
        &[1, 2, 0, 0],
        &[1, 1, 0, 1, 0, 1, 0x0A],
        &[1, 1, 0, 1, 0, 2, b'a'],
        &[1, 1, 0, 0, 0],
        &[2, 0, 0, 0, 0],
        &[2, 9, 0, 0, 0],
        &[2, 1, 0, 0, 1, 0x07],
        &[3, 0, 0, 0],
    ] {
        assert!(open(bad).is_err(), "{bad:?}");
    }
    assert_eq!(open(&[1, 1, 0, 1, 0, 2, b'o', b'k']), Ok(RconAnswer::Ran { ok: true, lines: vec!["ok".into()] }));
    for len in 0..300 {
        let _ = open(&noise.bytes(len));
    }
}

#[test]
fn answers_are_bounded_where_they_are_built() {
    let many = (0..5000).map(|i| format!("{i}\u{1b}[31m {}", "€".repeat(400)));
    let RconAnswer::Ran { lines, .. } = RconAnswer::ran(true, many) else { panic!("ran") };
    assert!(lines.len() <= MAX_ANSWER_LINES);
    assert!(lines.iter().all(|l| l.len() <= MAX_ANSWER_LINE && !l.contains('\u{1b}')));
    assert!(lines.iter().map(String::len).sum::<usize>() <= MAX_ANSWER_TEXT);
    assert!(lines.last().is_some_and(|l| l.contains("more lines are not shown")), "{:?}", lines.last());
    let reply = ReplySecret::generate();
    let body = RconAnswer::ran(true, lines.clone()).seal(&reply.public()).expect("the bounded answer seals");
    assert!(body.len() <= MAX_ANSWER_BODY);
    assert!(RconAnswer::Ran { ok: true, lines: vec!["a\nb".into()] }.seal(&reply.public()).is_err());
    let r = RconAnswer::refused(RconRefusalCode::Busy, 0, &"\u{7}€".repeat(300));
    let RconAnswer::Refused(r) = r else { panic!("refused") };
    assert!(r.text.len() <= MAX_REFUSAL_TEXT && !r.text.contains('\u{7}'));
}

#[test]
fn weak_operator_keys_are_refused() {
    let mut identity = [0u8; 32];
    identity[0] = 1;
    assert!(OperatorKey::from_bytes(identity).is_err());
    let k = OperatorSecret::generate();
    assert_eq!(OperatorKey::from_hex(&k.public().to_hex()), Ok(k.public()));
    assert_eq!(OperatorSecret::from_bytes(*k.to_bytes()).map(|s| s.public()).ok(), Some(k.public()));
    assert_eq!(k.public().id().short().len(), 16);
}
