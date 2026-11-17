use super::*;
use crate::test_rng::Rng;

fn header(opcode: u32, length: u32) -> Vec<u8> {
    let mut h = opcode.to_le_bytes().to_vec();
    h.extend_from_slice(&length.to_le_bytes());
    h
}

#[test]
fn the_documented_handshake_is_its_bytes() {
    let payload = br#"{"v":1,"client_id":"123456789012345678"}"#;
    let bytes = encode(Opcode::Handshake, payload).unwrap();
    assert_eq!(&bytes[..8], &[0, 0, 0, 0, payload.len() as u8, 0, 0, 0]);
    assert_eq!(&bytes[8..], payload);
}

#[test]
fn a_frame_split_at_every_byte_decodes_once() {
    let bytes = encode(Opcode::Frame, br#"{"cmd":"DISPATCH","evt":"READY"}"#).unwrap();
    for cut in 0..=bytes.len() {
        let mut d = Decoder::new();
        let mut got = d.feed(&bytes[..cut]).unwrap();
        got.extend(d.feed(&bytes[cut..]).unwrap());
        assert_eq!(got.len(), 1, "cut at {cut}");
        assert_eq!(got[0].payload, &bytes[8..]);
        assert_eq!(d.held(), 0);
    }
}

#[test]
fn empty_payloads_and_several_frames_in_one_read() {
    let mut bytes = encode(Opcode::Ping, b"").unwrap();
    bytes.extend(encode(Opcode::Frame, b"{}").unwrap());
    bytes.extend(encode(Opcode::Close, b"").unwrap());
    let got = Decoder::new().feed(&bytes).unwrap();
    let kinds: Vec<Opcode> = got.iter().map(|f| f.opcode).collect();
    assert_eq!(kinds, [Opcode::Ping, Opcode::Frame, Opcode::Close]);
}

#[test]
fn an_unknown_opcode_or_an_oversized_length_is_refused_at_its_header() {
    assert_eq!(Decoder::new().feed(&header(5, 2)), Err(FrameError::Opcode(5)));
    assert_eq!(Decoder::new().feed(&header(u32::MAX, 0)), Err(FrameError::Opcode(u32::MAX)));
    let over = MAX_PAYLOAD as u32 + 1;
    assert_eq!(Decoder::new().feed(&header(1, over)), Err(FrameError::Length(u64::from(over))));
    assert_eq!(Decoder::new().feed(&header(1, u32::MAX)), Err(FrameError::Length(u64::from(u32::MAX))));
    let mut d = Decoder::new();
    assert!(d.feed(&header(1, MAX_PAYLOAD as u32)).unwrap().is_empty());
    assert_eq!(d.feed(&vec![b' '; MAX_PAYLOAD]).unwrap().len(), 1);
}

#[test]
fn encode_refuses_what_decode_would() {
    assert_eq!(encode(Opcode::Frame, &vec![0; MAX_PAYLOAD + 1]), Err(FrameError::Length(MAX_PAYLOAD as u64 + 1)));
    assert!(encode(Opcode::Frame, &vec![0; MAX_PAYLOAD]).is_ok());
}

#[test]
fn random_streams_never_panic_and_stay_bounded() {
    let mut rng = Rng::new(0x5EED_D15C);
    for _ in 0..20_000 {
        let mut d = Decoder::new();
        let mut stream = Vec::new();
        for _ in 0..rng.below(6) {
            if rng.below(2) == 0 {
                stream.extend(header(rng.below(7) as u32, rng.below(80) as u32));
            }
            let len = rng.below(96);
            stream.extend(rng.bytes(len));
        }
        let mut rest = &stream[..];
        while !rest.is_empty() {
            let take = 1 + rng.below(rest.len());
            match d.feed(&rest[..take]) {
                Ok(frames) => {
                    for f in frames {
                        assert!(f.payload.len() <= MAX_PAYLOAD);
                        assert!(Opcode::from_number(f.opcode.number()).is_some());
                    }
                }
                Err(_) => break,
            }
            assert!(d.held() <= HEADER + MAX_PAYLOAD);
            rest = &rest[take..];
        }
    }
}

#[test]
fn random_valid_sequences_round_trip_in_any_pieces() {
    let mut rng = Rng::new(42);
    let opcodes = [Opcode::Handshake, Opcode::Frame, Opcode::Close, Opcode::Ping, Opcode::Pong];
    for _ in 0..5_000 {
        let sent: Vec<Frame> = (0..1 + rng.below(5))
            .map(|_| {
                let len = rng.below(300);
                Frame { opcode: opcodes[rng.below(5)], payload: rng.bytes(len) }
            })
            .collect();
        let mut stream = Vec::new();
        for f in &sent {
            stream.extend(encode(f.opcode, &f.payload).unwrap());
        }
        let mut d = Decoder::new();
        let mut got = Vec::new();
        let mut rest = &stream[..];
        while !rest.is_empty() {
            let take = 1 + rng.below(rest.len());
            got.extend(d.feed(&rest[..take]).unwrap());
            rest = &rest[take..];
        }
        assert_eq!(got, sent);
        assert_eq!(d.held(), 0);
    }
}
