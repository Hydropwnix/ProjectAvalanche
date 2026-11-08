use super::*;

fn ip(s: &str) -> IpAddr {
    s.parse().expect("an address")
}

#[test]
fn names_coolify_and_docker_write_are_taken_lowercase() {
    for (name, kept) in [
        ("coolify-proxy", "coolify-proxy"),
        ("Coolify-Proxy", "coolify-proxy"),
        ("traefik", "traefik"),
        ("proxy.internal", "proxy.internal"),
        ("my_project_proxy_1", "my_project_proxy_1"),
        ("a", "a"),
        ("x1.y2.example", "x1.y2.example"),
        ("1proxy.lan", "1proxy.lan"),
        ("xn--bcher-kva.example", "xn--bcher-kva.example"),
    ] {
        assert_eq!(HostName::parse(name).map(|n| n.to_string()), Ok(kept.to_string()), "{name}");
    }
    assert_eq!(HostName::parse("Coolify-Proxy").map(|n| n.absolute()), Ok("coolify-proxy.".to_string()));
    let longest = ["a".repeat(63), "b".repeat(63), "c".repeat(63), "d".repeat(61)].join(".");
    assert_eq!(longest.len(), 253);
    assert!(HostName::parse(&longest).is_ok());
}

#[test]
fn hostile_and_malformed_names_are_refused() {
    let refused = |s: &str| HostName::parse(s).expect_err(s);
    for address in ["10.0.0.1", "127.1", "2130706433", "0x7f.1", "0x7f000001", "017700000001", "1234", "10.0.0.1."] {
        assert!(HostName::parse(address).is_err(), "{address}");
    }
    assert!(refused("1234").contains("last label starts with a letter"));
    for bad in ["::1", "[::1]", "fd00::5", "proxy:8080", "coolify-proxy:443", "user@proxy", "proxy/24"] {
        assert!(refused(bad).contains("letters, digits"), "{bad}");
    }
    assert!(refused("coolify-proxy.").contains("trailing dot"));
    for empty in ["", ".", "..", ".proxy", "proxy..lan", "a.b."] {
        assert!(HostName::parse(empty).is_err(), "{empty:?}");
    }
    for edge in ["-proxy", "proxy-", "_proxy", "proxy_", "a.-b", "a.b-"] {
        assert!(refused(edge).contains("starts and ends"), "{edge}");
    }
    for odd in ["pr oxy", "proxy\t", "proxy\0", "proxy\n", "prøxy", "proxy*", "*.example", "%2e", "pro\"xy"] {
        assert!(refused(odd).contains("letters, digits"), "{odd:?}");
    }
    assert!(refused(&"a".repeat(64)).contains("longer than 63"));
    let too_long = ["a".repeat(63), "b".repeat(63), "c".repeat(63), "d".repeat(62)].join(".");
    assert!(refused(&too_long).contains("1 to 253"));
    let message = refused(&format!("\u{1b}[31m{}", "x".repeat(5000)));
    assert!(message.len() < 300 && !message.contains('\u{1b}'), "{message}");
}

#[test]
fn an_answer_is_taken_whole_and_only_without_internet_addresses() {
    let trusted = |list: &[&str]| judge(&list.iter().map(|s| ip(s)).collect::<Vec<_>>());
    assert_eq!(trusted(&["10.0.1.5"]), Answer::Trusted(vec![ip("10.0.1.5")]));
    assert_eq!(
        trusted(&["172.18.0.2", "fd00:1::2", "::ffff:172.18.0.2", "172.18.0.2"]),
        Answer::Trusted(vec![ip("172.18.0.2"), ip("fd00:1::2")]),
        "canonical, each address once, in order"
    );
    for private in ["192.168.5.1", "100.64.0.9", "127.0.0.1", "::1", "169.254.3.3", "fe80::1"] {
        assert!(matches!(trusted(&[private]), Answer::Trusted(_)), "{private}");
    }
    for public in ["8.8.8.8", "2a00:1450:4001::200e", "::ffff:1.1.1.1", "172.32.0.1"] {
        let Answer::Refused(why) = trusted(&["10.0.1.5", public]) else { panic!("{public} accepted") };
        assert!(why.ends_with("is an address the Internet routes to"), "{why}");
    }
    for never in ["0.0.0.0", "::", "224.0.0.1", "ff02::1", "255.255.255.255"] {
        let Answer::Refused(why) = trusted(&[never]) else { panic!("{never} accepted") };
        assert!(why.ends_with("is not a unicast address"), "{why}");
    }
    assert_eq!(trusted(&[]), Answer::Refused("no address".into()));
    let nine: Vec<String> = (1..=9).map(|i| format!("10.0.0.{i}")).collect();
    let nine: Vec<&str> = nine.iter().map(String::as_str).collect();
    assert_eq!(trusted(&nine), Answer::Refused("more than 8 addresses".into()));
    assert!(matches!(trusted(&nine[..8]), Answer::Trusted(v) if v.len() == 8));
    let same = vec![ip("10.0.0.7"); 100_000];
    assert_eq!(judge(&same), Answer::Trusted(vec![ip("10.0.0.7")]));
}
