use std::net::IpAddr;

use ssx_net_proto::operator::Expiry;

use super::*;

const NOW: u64 = 1_793_987_125;

fn ip(s: &str) -> IpAddr {
    s.parse().unwrap()
}

fn ban(target: &str, until: Expiry) -> Ban {
    Ban { target: Target::parse(target).unwrap(), reason: "cheats".into(), by: "console".into(), at: NOW, until }
}

#[test]
fn bans_hold_by_player_address_and_range() {
    let id = [7u8; 32];
    let mut s = BanStore::default();
    let p = format!("player:{}", "07".repeat(32));
    s.ban(ban(&p, Expiry::Never)).unwrap();
    s.ban(ban("ip:198.51.100.7", Expiry::after(NOW, Some(60)))).unwrap();
    s.ban(ban("range:203.0.113.*", Expiry::after(NOW, Some(3600)))).unwrap();
    s.ban(ban("range:2001:db8:1::/48", Expiry::after(NOW, Some(86_400)))).unwrap();
    assert_eq!(s.banned(Some(&id), None, NOW).map(|b| b.until), Some(Expiry::Never));
    assert!(s.banned(Some(&[8; 32]), Some(ip("198.51.100.8")), NOW).is_none());
    assert!(s.banned(None, Some(ip("::ffff:198.51.100.7")), NOW).is_some(), "an IPv4-mapped address");
    assert!(s.banned(None, Some(ip("203.0.113.99")), NOW).is_some(), "a wildcard");
    assert!(s.banned(None, Some(ip("2001:db8:1:ffff::1")), NOW).is_some(), "an IPv6 prefix");
    assert!(s.banned(None, Some(ip("2001:db8:2::1")), NOW).is_none());
    assert!(s.banned(None, Some(ip("198.51.100.7")), NOW + 59).is_some());
    assert!(s.banned(None, Some(ip("198.51.100.7")), NOW + 60).is_none());
    let both = s.banned(Some(&id), Some(ip("203.0.113.1")), NOW).unwrap();
    assert_eq!(both.until, Expiry::Never);
    assert!(s.ban(ban("ip:198.51.100.7", Expiry::Never)).unwrap().is_some());
    assert_eq!(s.bans().len(), 4);
    assert!(s.unban(&Target::parse("ip:198.51.100.7").unwrap()).is_some());
    assert!(s.unban_at(10).is_none() && s.unban_at(0).is_some());
    let (gone, _) = s.expire(NOW + 3600);
    assert_eq!(gone.len(), 1, "the wildcard's hour passed");
    assert_eq!(s.bans().len(), 1);
}

#[test]
fn mutes_and_their_end() {
    let mut s = BanStore::default();
    let m =
        |until| Mute { player: [1; 32], reason: "spam".into(), by: "operator 0123456789abcdef".into(), at: NOW, until };
    s.mute(m(Expiry::after(NOW, Some(600)))).unwrap();
    assert!(s.muted(&[1; 32], NOW + 599).is_some() && s.muted(&[1; 32], NOW + 600).is_none());
    assert!(s.muted(&[2; 32], NOW).is_none());
    assert!(s.mute(m(Expiry::Never)).unwrap().is_some(), "replaced");
    assert!(s.unmute(&[1; 32]).is_some() && s.unmute(&[1; 32]).is_none());
    let bad = Mute { reason: "a\u{7}".into(), ..m(Expiry::Never) };
    assert!(s.mute(bad).is_err());
    assert!(s.mute(m(Expiry::from_wire(NOW))).is_err(), "an end at its start");
    assert!(s.mute(m(Expiry::from_wire(u64::MAX))).is_err(), "an end the file cannot carry");
}

#[test]
fn the_file_reads_back_exactly() {
    let mut s = BanStore::default();
    s.ban(ban(&format!("player:{}", "ab".repeat(32)), Expiry::Never)).unwrap();
    s.ban(Ban { reason: "Zoë \"quoted\" \\ €".into(), ..ban("ip:2001:db8::7", Expiry::after(NOW, Some(1))) }).unwrap();
    s.ban(Ban { reason: String::new(), ..ban("range:192.0.2.0/24", Expiry::Never) }).unwrap();
    s.ban(ban("range:10.*.*.*", Expiry::Never)).unwrap();
    s.mute(Mute { player: [3; 32], reason: "flood".into(), by: "console".into(), at: NOW, until: Expiry::Never })
        .unwrap();
    let text = s.to_toml();
    assert_eq!(BanStore::parse(&text), Ok(s.clone()), "{text}");
    assert!(text.contains("# never") && text.contains("UTC"));
    assert_eq!(BanStore::parse(""), Ok(BanStore::default()));
}

#[test]
fn hostile_files_are_refused_with_their_place() {
    let p = "ab".repeat(32);
    let good = format!("[[ban]]\nplayer = \"{p}\"\nreason = \"x\"\nby = \"console\"\nat = 10\nuntil = 0\n");
    assert!(BanStore::parse(&good).is_ok());
    let cases = [
        (good.replace("player", "id"), 2),
        (good.replace(&p, &p.to_uppercase()), 2),
        (good.replace(&p, "ab"), 2),
        (good.replace(&format!("player = \"{p}\""), "ip = \"192.0.2.0/24\""), 2),
        (good.replace(&format!("player = \"{p}\""), "ip = \"::ffff:192.0.2.1\""), 2),
        (good.replace(&format!("player = \"{p}\""), "range = \"192.0.2.1\""), 2),
        (good.replace(&format!("player = \"{p}\""), "range = \"192.0.2.1/24\""), 2),
        (good.replace(&format!("player = \"{p}\"\n"), ""), 1),
        (good.replace("player = ", "ip = \"192.0.2.1\"\nplayer = "), 1),
        (good.replace("reason = \"x\"", "reason = \"a\\u0007\""), 3),
        (good.replace("by = \"console\"", "by = \"\""), 4),
        (good.replace("at = 10", "at = -1"), 5),
        (good.replace("until = 0", "until = 5"), 6),
        (good.replace("until = 0", "until = -2"), 6),
        (good.replace("until = 0", "until = \"never\""), 6),
        (format!("{good}extra = 1\n"), 7),
        (format!("{good}{good}"), 7),
        (
            format!(
                "[[mute]]\nplayer = \"{p}\"\nreason = \"\"\nby = \"c\"\nat = 1\nuntil = 0\n[[mute]]\nplayer = \"{p}\"\nreason = \"\"\nby = \"c\"\nat = 1\nuntil = 0\n"
            ),
            7,
        ),
        ("[ban]\nplayer = 1\n".to_string(), 1),
        ("[[ban]\n".to_string(), 1),
    ];
    for (text, line) in cases {
        let e = BanStore::parse(&text).expect_err(&text);
        assert_eq!(e.line, line, "{e} in\n{text}");
    }
}
