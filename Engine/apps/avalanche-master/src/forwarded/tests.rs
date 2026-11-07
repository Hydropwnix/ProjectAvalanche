use super::*;

fn trusted(nets: &[&str]) -> Trusted {
    Trusted::new(nets.iter().map(|n| Network::parse(n).expect("a network")).collect())
}

fn ip(s: &str) -> IpAddr {
    s.parse().expect("an address")
}

fn from_proxy(lines: &[&str]) -> Result<Source, Refusal> {
    source(ip("10.0.1.2"), &trusted(&["10.0.1.0/24"]), lines.iter().map(|l| l.as_bytes()))
}

#[test]
fn networks_are_strict() {
    let n = Network::parse("10.0.1.0/24").expect("valid");
    assert!(n.contains(ip("10.0.1.255")) && n.contains(ip("::ffff:10.0.1.7")) && !n.contains(ip("10.0.2.1")));
    assert_eq!(Network::parse("172.18.0.2").map(|n| n.to_string()), Ok("172.18.0.2/32".into()));
    assert_eq!(Network::parse("fd00:1::/64").map(|n| n.to_string()), Ok("fd00:1::/64".into()));
    assert!(Network::parse("fd00::/8").expect("v6").contains(ip("fdab::1")));
    assert!(!Network::parse("fd00::/8").expect("v6").contains(ip("10.0.0.1")), "families never mix");
    assert!(!Network::parse("0.0.0.0/1").expect("v4").contains(ip("::1")));
    for (bad, why) in [
        ("10.0.1.5/24", "host bits"),
        ("0.0.0.0/0", "/0"),
        ("::/0", "/0"),
        ("10.0.0.0/33", "prefix"),
        ("fd00::/129", "prefix"),
        ("10.0.0.0/024", "prefix"),
        ("10.0.0.0/", "prefix"),
        ("10.0.0.0/+8", "prefix"),
        ("10.0.0.0/8/8", "prefix"),
        ("::ffff:10.0.0.0/104", "IPv4-mapped"),
        ("[fd00::]/8", "address"),
        ("fe80::1%eth0/128", "address"),
        ("010.0.0.0/8", "address"),
        ("proxy.local/32", "address"),
        (" 10.0.0.0/8", "address"),
        ("", "address"),
    ] {
        let e = Network::parse(bad).expect_err(bad);
        assert!(e.contains(why), "{bad}: {e}");
    }
}

#[test]
fn the_right_most_entry_of_a_trusted_proxy_is_the_client() {
    let proxied = |client: &str| Ok(Source::Proxied { proxy: ip("10.0.1.2"), client: ip(client) });
    assert_eq!(from_proxy(&["203.0.113.5"]), proxied("203.0.113.5"));
    assert_eq!(from_proxy(&["1.2.3.4, 198.51.100.7"]), proxied("198.51.100.7"));
    assert_eq!(from_proxy(&["1.2.3.4", "5.6.7.8,\t198.51.100.7 "]), proxied("198.51.100.7"), "every line, in order");
    assert_eq!(from_proxy(&["2001:db8::5"]), proxied("2001:db8::5"));
    assert_eq!(from_proxy(&["::ffff:203.0.113.9"]), proxied("203.0.113.9"), "IPv4-mapped is IPv4");
    assert_eq!(from_proxy(&["10.0.1.3"]), proxied("10.0.1.3"), "one layer: a client in the network is a client");
    let max = vec!["1.1.1.1"; MAX_FORWARDED_ENTRIES].join(",");
    assert_eq!(from_proxy(&[&max]), proxied("1.1.1.1"));
}

#[test]
fn a_trusted_proxys_header_is_refused_when_it_is_not_a_list_of_addresses() {
    let too_many = vec!["1.1.1.1"; MAX_FORWARDED_ENTRIES + 1].join(",");
    let lines: Vec<String> = (0..MAX_FORWARDED_ENTRIES + 1).map(|i| format!("10.0.0.{i}")).collect();
    let line_refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let long = format!("{}1.1.1.1", " ".repeat(MAX_FORWARDED_BYTES));
    let huge = "9".repeat(64 * 1024);
    for (lines, want) in [
        (&[][..], Refusal::Missing),
        (&[""][..], Refusal::Empty),
        (&["  "][..], Refusal::Empty),
        (&["1.2.3.4,"][..], Refusal::Empty),
        (&["1.2.3.4,,5.6.7.8"][..], Refusal::Empty),
        (&["1.2.3.4", ""][..], Refusal::Empty),
        (&["unknown"][..], Refusal::NotAnAddress),
        (&["_hidden"][..], Refusal::NotAnAddress),
        (&["[2001:db8::1]"][..], Refusal::NotAnAddress),
        (&["[2001:db8::1]:443"][..], Refusal::NotAnAddress),
        (&["203.0.113.5:4711"][..], Refusal::NotAnAddress),
        (&["fe80::1%eth0"][..], Refusal::NotAnAddress),
        (&["203.0.113.05"][..], Refusal::NotAnAddress),
        (&["203.0.113"][..], Refusal::NotAnAddress),
        (&["0x7f.0.0.1"][..], Refusal::NotAnAddress),
        (&["203.0.113.5 198.51.100.7"][..], Refusal::NotAnAddress),
        (&["for=203.0.113.5"][..], Refusal::NotAnAddress),
        (&["203.0.113.5\u{b}"][..], Refusal::NotAnAddress),
        (&["203.0.113.5;proto=https"][..], Refusal::NotAnAddress),
        (&["garbage, 203.0.113.5"][..], Refusal::NotAnAddress),
        (&["0.0.0.0"][..], Refusal::NotUnicast),
        (&["::"][..], Refusal::NotUnicast),
        (&["224.0.0.1"][..], Refusal::NotUnicast),
        (&["255.255.255.255"][..], Refusal::NotUnicast),
        (&["ff02::1"][..], Refusal::NotUnicast),
        (&[too_many.as_str()][..], Refusal::TooMany),
        (&line_refs[..], Refusal::TooMany),
        (&[long.as_str()][..], Refusal::TooLong),
        (&[huge.as_str()][..], Refusal::TooLong),
    ] {
        assert_eq!(
            from_proxy(lines),
            Err(want),
            "{:?}",
            lines.iter().map(|l| &l[..l.len().min(40)]).collect::<Vec<_>>()
        );
    }
    let bytes: &[&[u8]] = &[b"203.0.113.5\xff"];
    assert_eq!(source(ip("10.0.1.2"), &trusted(&["10.0.1.0/24"]), bytes.iter().copied()), Err(Refusal::NotAnAddress));
}

#[test]
fn any_other_peer_is_its_own_client_and_its_headers_are_not_read() {
    let lines: &[&[u8]] = &[b"198.51.100.7", b"garbage"];
    let none = Trusted::default();
    assert_eq!(source(ip("203.0.113.5"), &none, lines.iter().copied()), Ok(Source::Direct(ip("203.0.113.5"))));
    let t = trusted(&["10.0.1.0/24", "fd00:1::/64"]);
    assert_eq!(source(ip("10.0.2.1"), &t, lines.iter().copied()), Ok(Source::Direct(ip("10.0.2.1"))));
    assert_eq!(source(ip("::ffff:10.0.2.1"), &t, lines.iter().copied()), Ok(Source::Direct(ip("10.0.2.1"))));
    assert_eq!(
        source(ip("fd00:1::9"), &t, [&b"2001:db8::7"[..]]),
        Ok(Source::Proxied { proxy: ip("fd00:1::9"), client: ip("2001:db8::7") })
    );
    assert_eq!(source(ip("::ffff:10.0.1.9"), &t, [&b""[..]]), Err(Refusal::Empty), "a mapped proxy is the proxy");
}
