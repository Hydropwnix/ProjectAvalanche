use super::cache::{self, Cached, MAX_BYTES, MAX_ETAG, MAX_MASTER};
use super::*;
use crate::test_dir::TestDir;

fn h(n: u8) -> ContentHash {
    ContentHash([n; 32])
}

fn kept(master: &str, hashes: Vec<ContentHash>, checked: u64) -> Cached {
    Cached { master: master.into(), etag: Some("\"abc\"".into()), fetched: checked, checked, hashes }
}

fn resealed(mut bytes: Vec<u8>, edit: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
    bytes.truncate(bytes.len() - 32);
    edit(&mut bytes);
    let sum = blake3::hash(&bytes);
    bytes.extend_from_slice(sum.as_bytes());
    bytes
}

#[test]
fn a_kept_list_reads_back_strictly() {
    let c = kept("http://127.0.0.1:27900", vec![h(1), h(2), h(9)], 1_000);
    let bytes = c.to_bytes().unwrap();
    assert_eq!(Cached::from_bytes(&bytes).unwrap(), c);
    for i in 0..bytes.len() {
        let mut b = bytes.clone();
        b[i] ^= 0x20;
        assert!(Cached::from_bytes(&b).is_err(), "byte {i} changed");
    }
    assert!(Cached::from_bytes(&bytes[..bytes.len() - 1]).is_err(), "cut");
    assert!(Cached::from_bytes(&[bytes.as_slice(), &[0]].concat()).is_err(), "a byte more");
    assert!(Cached::from_bytes(&[]).is_err());
    let hashes_at = bytes.len() - 32 - 3 * 32;
    let swapped = resealed(bytes.clone(), |b| {
        let (one, two) = (b[hashes_at..hashes_at + 32].to_vec(), b[hashes_at + 32..hashes_at + 64].to_vec());
        b[hashes_at..hashes_at + 32].copy_from_slice(&two);
        b[hashes_at + 32..hashes_at + 64].copy_from_slice(&one);
    });
    assert!(Cached::from_bytes(&swapped).unwrap_err().contains("sorted"), "out of order");
    let twice = resealed(bytes.clone(), |b| {
        let one = b[hashes_at..hashes_at + 32].to_vec();
        b[hashes_at + 32..hashes_at + 64].copy_from_slice(&one);
    });
    assert!(Cached::from_bytes(&twice).is_err(), "a hash twice");
    let count_at = hashes_at - 4;
    let more = resealed(bytes.clone(), |b| b[count_at..count_at + 4].copy_from_slice(&4u32.to_le_bytes()));
    assert!(Cached::from_bytes(&more).unwrap_err().contains("bytes for 4 hashes"));
    let huge = resealed(bytes.clone(), |b| b[count_at..count_at + 4].copy_from_slice(&u32::MAX.to_le_bytes()));
    assert!(Cached::from_bytes(&huge).unwrap_err().contains("at most"), "a hostile count is refused before use");
    let format = resealed(bytes.clone(), |b| b[8..12].copy_from_slice(&2u32.to_le_bytes()));
    assert!(Cached::from_bytes(&format).unwrap_err().contains("format 2"));
    assert!(kept("", vec![], 0).to_bytes().is_err());
    assert!(kept(&"m".repeat(MAX_MASTER + 1), vec![], 0).to_bytes().is_err());
    assert!(kept("a b", vec![], 0).to_bytes().is_err(), "a space");
    let mut e = kept("m", vec![], 0);
    e.etag = Some(String::new());
    assert!(e.to_bytes().is_err(), "an empty ETag (none is written as none)");
    assert!(Cached::from_bytes(&vec![0; MAX_BYTES + 1]).unwrap_err().contains("at most"));
}

#[test]
fn the_widest_list_fits_the_cap() {
    let hashes: Vec<ContentHash> = (0..MAX_ENTRIES as u32)
        .map(|i| {
            let mut b = [0u8; 32];
            b[..4].copy_from_slice(&i.to_be_bytes());
            ContentHash(b)
        })
        .collect();
    let c = Cached {
        master: "m".repeat(MAX_MASTER),
        etag: Some("e".repeat(MAX_ETAG)),
        fetched: u64::MAX,
        checked: u64::MAX,
        hashes,
    };
    let bytes = c.to_bytes().unwrap();
    assert_eq!(bytes.len(), MAX_BYTES);
    assert_eq!(Cached::from_bytes(&bytes).unwrap().hashes.len(), MAX_ENTRIES);
    let mut one_more = c.clone();
    one_more.hashes.push(ContentHash([0xFF; 32]));
    assert!(one_more.to_bytes().is_err());
}

#[test]
fn a_lists_age() {
    let c = kept("m", vec![], 1_000);
    assert_eq!(c.age(1_000 + 60).unwrap(), Duration::from_secs(60));
    assert_eq!(c.age(900).unwrap(), Duration::ZERO, "a little ahead of the clock");
    assert!(kept("m", vec![], 1_000 + FUTURE_SLACK.as_secs() + 1).age(1_000).is_err());
}

#[test]
fn a_masters_hashes() {
    let (a, b) = ("ab".repeat(32), "01".repeat(32));
    assert_eq!(from_hex_list(&[a.clone(), b.clone()]).unwrap(), vec![ContentHash([1; 32]), ContentHash([0xAB; 32])]);
    assert!(from_hex_list(&[a.clone(), a.clone()]).unwrap_err().contains("twice"));
    assert!(from_hex_list(&["AB".repeat(32)]).is_err());
    assert!(from_hex_list(&["ab".repeat(31)]).is_err());
    assert!(from_hex_list(&vec![a; MAX_ENTRIES + 1]).unwrap_err().contains("at most"));
}

#[test]
fn the_followed_lists_are_united() {
    let dir = TestDir::new("blocklist-load");
    let now = 10_000_000;
    let (m1, m2, m3, m4, m5, other) = ("http://a", "http://b", "http://c", "http://d", "http://e", "http://x");
    cache::write(&dir.0, &kept(m1, vec![h(1), h(2)], now - 10)).unwrap();
    cache::write(&dir.0, &kept(m2, vec![h(2), h(3)], now - 20)).unwrap();
    cache::write(&dir.0, &kept(m3, vec![h(4)], now - MAX_AGE.as_secs() - 1)).unwrap();
    std::fs::write(cache::path_of(&dir.0, m4), b"not a list").unwrap();
    cache::write(&dir.0, &kept(other, vec![h(5)], now)).unwrap();
    std::fs::copy(cache::path_of(&dir.0, other), cache::path_of(&dir.0, m5)).unwrap();
    let f = load(&dir.0, &[m1, m2, m3, m4, m5, "http://f"].map(String::from), now);
    assert_eq!(f.blocked.entries(), 4);
    assert_eq!(f.blocked.source_of(&h(2)), Some(m1), "the first list that holds it");
    assert_eq!(f.blocked.source_of(&h(3)), Some(m2));
    assert_eq!(f.blocked.source_of(&h(4)), None, "too old");
    assert_eq!(f.blocked.source_of(&h(5)), None, "not followed / another master's file");
    let states: Vec<&State> = f.states.iter().map(|(_, s)| s).collect();
    assert!(matches!(states[0], State::Applied { entries: 2, .. }));
    assert!(matches!(states[2], State::Expired { entries: 1, .. }));
    assert!(matches!(states[3], State::Unreadable(_)));
    assert!(matches!(states[4], State::Unreadable(e) if e.contains("another master")));
    assert_eq!(states[5], &State::NotYet);
    assert!(load(&dir.0, &[], now).blocked.is_empty(), "no master followed: no blocklist");
}

#[test]
fn hits_and_their_reason() {
    let b = Blocked::new(vec![List::new("http://m".into(), vec![h(7)]).unwrap()]);
    let files = [("a.png".to_string(), h(1)), ("b\u{1b}[2J.png".to_string(), h(7)), ("c.png".to_string(), h(7))];
    let hits = b.hits(files.iter().map(|(p, h)| (p.as_str(), h)));
    assert_eq!(hits.len(), 2);
    let why = describe("the resource \"skins\"", &hits);
    assert!(why.contains("http://m") && why.contains("and 1 more") && !why.contains('\u{1b}'), "{why}");
    assert!(Blocked::default().hits(files.iter().map(|(p, h)| (p.as_str(), h))).is_empty());
}

#[test]
fn the_masters_a_player_follows() {
    use serde_json::json;
    assert_eq!(followed_masters(None).unwrap(), vec![ssx_core::OFFICIAL_MASTER.to_string()]);
    assert!(followed_masters(Some(&json!([]))).unwrap().is_empty());
    let two = json!(["http://127.0.0.1:1", "https://m.example.org/"]);
    assert_eq!(followed_masters(Some(&two)).unwrap(), ["http://127.0.0.1:1", "https://m.example.org/"]);
    for bad in [
        json!("http://a"),
        json!([1]),
        json!(["http://a", "http://a"]),
        json!([""]),
        json!(["http://a b"]),
        json!([format!("http://{}", "a".repeat(260))]),
        json!(["http://\u{1b}[2J"]),
        json!((0..9).map(|i| format!("http://m{i}")).collect::<Vec<_>>()),
    ] {
        assert!(followed_masters(Some(&bad)).is_err(), "{bad}");
    }
    let dir = TestDir::new("blocklist-settings");
    assert_eq!(settings_masters(&dir.0).unwrap(), vec![ssx_core::OFFICIAL_MASTER.to_string()], "no file");
    dir.write(SETTINGS_FILE, r#"{"vsync": true, "masters": ["http://127.0.0.1:9"]}"#);
    assert_eq!(settings_masters(&dir.0).unwrap(), ["http://127.0.0.1:9"]);
    dir.write(SETTINGS_FILE, r#"{"masters": []}"#);
    assert!(settings_masters(&dir.0).unwrap().is_empty());
    dir.write(SETTINGS_FILE, "[1, 2]");
    assert!(settings_masters(&dir.0).is_err());
    dir.write_bytes(SETTINGS_FILE, &vec![b' '; MAX_SETTINGS_BYTES as usize + 1]);
    assert!(settings_masters(&dir.0).unwrap_err().contains("larger"));
    let f = for_player(&dir.0, 1);
    assert!(f.problem.is_some() && f.blocked.is_empty() && f.lines().iter().any(|l| l.contains("none followed")));
}

#[test]
fn answers_are_kept() {
    let dir = TestDir::new("blocklist-record");
    let m = "http://127.0.0.1:27900";
    let t = 1_000_000;
    assert!(due(&dir.0, m, t), "nothing kept");
    assert!(record(&dir.0, m, Answer::NotModified, t).unwrap_err().contains("304"));
    assert!(cache::read(&dir.0, m).unwrap().is_none(), "nothing kept from a 304");
    let list = Answer::List { hashes: vec![h(1), h(2)], etag: Some("\"e1\"".into()) };
    assert_eq!(record(&dir.0, m, list, t).unwrap(), 2);
    assert_eq!(etag_of(&dir.0, m).as_deref(), Some("\"e1\""));
    assert!(!due(&dir.0, m, t + 60));
    assert!(due(&dir.0, m, t + REFRESH_INTERVAL.as_secs()));
    let later = t + MAX_AGE.as_secs() - 10;
    assert_eq!(record(&dir.0, m, Answer::NotModified, later).unwrap(), 2);
    let kept = cache::read(&dir.0, m).unwrap().unwrap();
    assert_eq!((kept.fetched, kept.checked), (t, later));
    let f = load(&dir.0, &[m.to_string()], later + 3600);
    assert!(f.blocked.source_of(&h(2)).is_some(), "a 304 keeps the list ruling");
    let bad_tag = Answer::List { hashes: vec![h(3)], etag: Some("a b".into()) };
    assert_eq!(record(&dir.0, m, bad_tag, later + 1).unwrap(), 1);
    assert_eq!(etag_of(&dir.0, m), None, "an ETag the cache cannot keep is dropped");
    std::fs::write(cache::path_of(&dir.0, m), b"torn").unwrap();
    assert!(etag_of(&dir.0, m).is_none() && due(&dir.0, m, later + 2));
    assert!(record(&dir.0, m, Answer::NotModified, later + 2).is_err(), "a 304 over a cache that does not read");
}

#[test]
fn an_answer_is_not_before_its_list() {
    let mut c = kept("m", vec![ContentHash([1; 32])], 1_000);
    c.checked = 999;
    assert!(c.to_bytes().is_err());
    let good = kept("m", vec![ContentHash([1; 32])], 1_000);
    let mut bytes = good.to_bytes().unwrap();
    bytes[20..28].copy_from_slice(&999u64.to_le_bytes());
    let body = bytes.len() - 32;
    let sum = *blake3::hash(&bytes[..body]).as_bytes();
    bytes[body..].copy_from_slice(&sum);
    assert!(Cached::from_bytes(&bytes).unwrap_err().contains("before its list"));
    let tmp = TestDir::new("blocklist-clock");
    let dir = tmp.0.join("Blocklists");
    let n = record(&dir, "m", Answer::List { hashes: vec![ContentHash([1; 32])], etag: Some("\"a\"".into()) }, 2_000);
    assert_eq!(n, Ok(1));
    assert_eq!(record(&dir, "m", Answer::NotModified, 1_500), Ok(1));
    let kept = cache::read(&dir, "m").unwrap().unwrap();
    assert_eq!((kept.fetched, kept.checked), (2_000, 2_000));
}
