use super::*;
use crate::activity::{ActivityType, Text};
use crate::test_rng::Rng;

const READY: &str = r#"{"cmd":"DISPATCH","data":{"v":1,"config":{"cdn_host":"cdn.discordapp.com",
    "api_endpoint":"//discord.com/api","environment":"production"},"user":{"id":"53908232506183680",
    "username":"Mason","discriminator":"1337","avatar":null}},"evt":"READY"}"#;

#[test]
fn ready_is_read_without_its_user() {
    assert_eq!(inbound(READY.as_bytes()), Ok(Inbound::Ready));
}

#[test]
fn replies_are_matched_by_the_games_nonce() {
    let ok = r#"{"cmd":"SET_ACTIVITY","data":{"type":0},"evt":null,"nonce":"7"}"#;
    assert_eq!(inbound(ok.as_bytes()), Ok(Inbound::Reply { nonce: 7, error: None }));
    let err =
        r#"{"cmd":"SET_ACTIVITY","data":{"code":4000,"message":"child \"activity\" fails"},"evt":"ERROR","nonce":"8"}"#;
    assert_eq!(
        inbound(err.as_bytes()),
        Ok(Inbound::Reply {
            nonce: 8,
            error: Some(RpcError { code: Some(4000), message: r#"child "activity" fails"#.into() })
        })
    );
    for other in [r#"{"cmd":"SET_ACTIVITY","nonce":"f48f6176-4afb"}"#, r#"{"cmd":"DISPATCH","evt":"GUILD_STATUS"}"#] {
        assert_eq!(inbound(other.as_bytes()), Ok(Inbound::Other));
    }
}

#[test]
fn hostile_payloads_are_refused_or_ignored_never_trusted() {
    assert!(matches!(inbound(b"\xff\xfe"), Err(PayloadError::Json(_))));
    assert_eq!(inbound(b"[1,2]"), Err(PayloadError::NotObject));
    assert_eq!(inbound(br#"{"cmd":5}"#), Err(PayloadError::Member("cmd")));
    let long = format!(r#"{{"cmd":"{}"}}"#, "A".repeat(65));
    assert_eq!(inbound(long.as_bytes()), Err(PayloadError::Member("cmd")));
    assert_eq!(inbound(br#"{"nonce":{"x":1}}"#), Err(PayloadError::Member("nonce")));
    let deep = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
    assert!(matches!(inbound(deep.as_bytes()), Err(PayloadError::Json(_))));
}

#[test]
fn an_error_message_is_escaped_and_bounded() {
    let msg = format!("\u{1b}[31m{}", "x".repeat(1000));
    let err = serde_json::json!({ "cmd": "SET_ACTIVITY", "evt": "ERROR", "nonce": "1", "data": { "message": msg } });
    let Ok(Inbound::Reply { error: Some(e), .. }) = inbound(err.to_string().as_bytes()) else { panic!("not a reply") };
    assert!(!e.message.contains('\u{1b}'));
    assert!(e.message.chars().count() <= MESSAGE_CHARS + 64, "{}", e.message.len());
}

#[test]
fn close_reasons_read_leniently() {
    assert_eq!(
        close_reason(br#"{"code":4000,"message":"Invalid Client ID"}"#),
        RpcError { code: Some(4000), message: "Invalid Client ID".into() }
    );
    assert_eq!(close_reason(b"not json"), RpcError { code: None, message: String::new() });
    assert_eq!(close_reason(br#"{"code":"x"}"#), RpcError { code: None, message: String::new() });
}

#[test]
fn the_games_payloads() {
    let h: Value = serde_json::from_slice(&handshake("1571572936433995906")).unwrap();
    assert_eq!(h, serde_json::json!({ "v": 1, "client_id": "1571572936433995906" }));
    let mut a = Activity::new(ActivityType::Playing);
    a.details = Text::new("In the menus");
    let v: Value = serde_json::from_slice(&set_activity(4242, &a, 3)).unwrap();
    assert_eq!(
        v,
        serde_json::json!({
            "cmd": "SET_ACTIVITY",
            "args": { "pid": 4242, "activity": { "type": 0, "details": "In the menus" } },
            "nonce": "3",
        })
    );
    assert!(valid_client_id("1571572936433995906"));
    assert!(!valid_client_id(""));
    assert!(!valid_client_id("99999999999999999999"));
    assert!(!valid_client_id("+1571572936433995906"));
    assert!(!valid_client_id("01571572936433995906"));
    assert!(!valid_client_id("12a"));
}

#[test]
fn mutated_payloads_never_panic() {
    let seeds = [
        READY.to_string(),
        r#"{"cmd":"SET_ACTIVITY","data":{"code":4000,"message":"bad"},"evt":"ERROR","nonce":"8"}"#.to_string(),
        r#"{"code":1000,"message":"closed"}"#.to_string(),
    ];
    let mut rng = Rng::new(99);
    for i in 0..30_000 {
        let mut bytes = seeds[i % seeds.len()].clone().into_bytes();
        for _ in 0..1 + rng.below(4) {
            let at = rng.below(bytes.len().max(1));
            match rng.below(3) {
                0 if !bytes.is_empty() => bytes[at] = rng.next() as u8,
                1 if !bytes.is_empty() => drop(bytes.remove(at)),
                _ => bytes.insert(at.min(bytes.len()), b"{}[]\":,0\\"[rng.below(9)]),
            }
        }
        if let Ok(Inbound::Reply { error: Some(e), .. }) = inbound(&bytes) {
            assert!(e.message.len() <= 4 * (MESSAGE_CHARS + 64));
        }
        let _ = close_reason(&bytes);
    }
}
