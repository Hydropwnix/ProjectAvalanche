use super::*;

fn bytes(x: &mut u64, len: usize, modulo: u64) -> Vec<u8> {
    (0..len)
        .map(|_| {
            *x ^= *x << 13;
            *x ^= *x >> 7;
            *x ^= *x << 17;
            (*x % modulo) as u8
        })
        .collect()
}

#[test]
fn the_text_rule_takes_the_fonts_characters_only() {
    assert_eq!(chat_text("hi everyone!"), Ok("hi everyone!"));
    assert_eq!(chat_text(" a "), Ok(" a "));
    let widest = "\u{221E}".repeat(MAX_CHAT_CHARS);
    assert_eq!(widest.len(), MAX_CHAT_BYTES);
    assert!(chat_text(&widest).is_ok());
    for bad in [
        String::new(),
        "   ".into(),
        "x".repeat(MAX_CHAT_CHARS + 1),
        "\u{221E}".repeat(MAX_CHAT_CHARS + 1),
        "line\nbreak".into(),
        "tab\there".into(),
        "bell\u{7}".into(),
        "del\u{7F}".into(),
        "c1\u{9F}".into(),
        "nbsp\u{A0}".into(),
        "ideographic\u{3000}space".into(),
        "emoji \u{1F600}".into(),
        "bidi \u{202E}".into(),
        "zero width \u{200B}".into(),
    ] {
        assert!(chat_text(&bad).is_err(), "{bad:?}");
    }
}

#[test]
fn round_trips_and_the_largest_messages() {
    let widest = "\u{221E}".repeat(MAX_CHAT_CHARS);
    let client = [ClientChat::text("gg").unwrap(), ClientChat::Text(widest.clone()), ClientChat::quick(7).unwrap()];
    for m in &client {
        assert_eq!(ClientChat::decode(&m.encode()).as_ref(), Ok(m));
    }
    assert_eq!(ClientChat::Text(widest.clone()).encode().len(), MAX_CLIENT_CHAT_SIZE);
    let server = [
        ServerChat::Line { rider: 0xBEEF, text: "<b>&amp;</b>".into() },
        ServerChat::Line { rider: 3, text: widest.clone() },
        ServerChat::Quick { rider: 1, index: 0 },
        ServerChat::server("restart in 5 min").unwrap(),
        ServerChat::Refused(ChatRefusal::Flood { wait_ms: 750 }),
        ServerChat::Refused(ChatRefusal::Disabled),
        ServerChat::Refused(ChatRefusal::muted(crate::operator::Expiry::Never, "").unwrap()),
        ServerChat::Refused(ChatRefusal::muted(crate::operator::Expiry::from_wire(9), "spam").unwrap()),
        ServerChat::Refused(ChatRefusal::muted(crate::operator::Expiry::from_wire(u64::MAX), &widest).unwrap()),
    ];
    for m in &server {
        assert_eq!(ServerChat::decode(&m.encode()).as_ref(), Ok(m));
    }
    assert_eq!(server[8].encode().len(), MAX_SERVER_CHAT_SIZE);
    assert!(server[1].encode().len() < MAX_SERVER_CHAT_SIZE);
    assert!(ChatRefusal::muted(crate::operator::Expiry::Never, "a\u{7}").is_err());
    for bad in [
        &[4u8, 0, 3, 3, 0, 0][..],
        &[4, 0, 10, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        &[4, 0, 11, 3, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0],
        &[4, 0, 12, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x0A],
        &[4, 0, 12, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, b' '],
        &[4, 0, 3, 4, 0, 0],
    ] {
        assert!(ServerChat::decode(bad).is_err(), "{bad:?}");
    }
    assert!(ClientChat::quick(QUICK_LINES).is_err());
    assert!(ServerChat::server("").is_err());
}

#[test]
fn hostile_bytes_are_refused_without_a_panic() {
    let mut x = 0x2545_F491_4F6C_DD1Du64;
    for len in 0..(MAX_SERVER_CHAT_SIZE + 40) {
        for modulo in [5, 256] {
            for _ in 0..300 {
                let b = bytes(&mut x, len, modulo);
                if let Ok(m) = ClientChat::decode(&b) {
                    assert_eq!(m.encode(), b);
                }
                if let Ok(m) = ServerChat::decode(&b) {
                    assert_eq!(m.encode(), b);
                }
            }
        }
    }
    let framed = |kind: u8, payload: &[u8]| super::framed(kind, payload);
    for bad in [
        vec![],
        vec![1],
        vec![1, 0],
        framed(1, b""),
        framed(1, &[0xC3, 0x28]),
        framed(1, b"a\nb"),
        framed(1, &[b'x'; MAX_CHAT_CHARS + 1]),
        framed(2, &[QUICK_LINES]),
        framed(2, &[1, 2]),
        framed(3, b"x"),
        vec![1, 0, 3, b'a', b'b'],
        vec![1, 0, 1, b'a', b'b'],
    ] {
        assert!(ClientChat::decode(&bad).is_err(), "{bad:?}");
    }
    let mut oversized = framed(1, &[b'x'; MAX_CHAT_BYTES + 1]);
    assert!(ClientChat::decode(&oversized).is_err());
    oversized.truncate(MAX_CLIENT_CHAT_SIZE + 1);
    assert!(ClientChat::decode(&oversized).is_err());
    assert!(ClientChat::decode(&vec![b'x'; 64 * 1024]).is_err());
    for bad in [
        framed(1, &[0, 1]),
        framed(1, &[0, 1, 0xFF]),
        framed(1, &[0, 1, b'\r']),
        framed(2, &[0, 1, QUICK_LINES]),
        framed(2, &[0, 1]),
        framed(3, b"   "),
        framed(4, &[1, 0, 0]),
        framed(4, &[1, 0xEA, 0x61]),
        framed(4, &[2, 0, 1]),
        framed(4, &[3, 0, 0]),
        framed(4, &[1, 0, 1, 0]),
        framed(5, b""),
    ] {
        assert!(ServerChat::decode(&bad).is_err(), "{bad:?}");
    }
    assert!(ServerChat::decode(&vec![1; 64 * 1024]).is_err());
}
