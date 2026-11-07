use super::*;
use crate::forwarded::Network;

fn padded(lines: &[&str]) -> String {
    lines.iter().map(|l| format!("{l:<127}\n")).collect()
}

fn docker_v4() -> String {
    padded(&[
        HEADER_V4,
        "eth0\t00000000\t012CE90A\t0003\t0\t0\t0\t00000000\t0\t0\t0",
        "eth0\t002CE90A\t00000000\t0001\t0\t0\t0\t00FFFFFF\t0\t0\t0",
    ])
}

const DOCKER_V6: &str = "\
fd00004c004400000000000000000000 40 00000000000000000000000000000000 00 00000000000000000000000000000000 00000100 00000003 00000000 00000001     eth0
fe800000000000000000000000000000 40 00000000000000000000000000000000 00 00000000000000000000000000000000 00000100 00000001 00000000 00000001     eth0
00000000000000000000000000000000 00 00000000000000000000000000000000 00 fd00004c004400000000000000000001 00000400 00000001 00000000 00000003     eth0
00000000000000000000000000000001 80 00000000000000000000000000000000 00 00000000000000000000000000000000 00000000 00000004 00000000 80200001       lo
fd00004c004400000000000000000002 80 00000000000000000000000000000000 00 00000000000000000000000000000000 00000000 00000003 00000000 80200001     eth0
ff000000000000000000000000000000 08 00000000000000000000000000000000 00 00000000000000000000000000000000 00000100 00000004 00000000 00000001     eth0
00000000000000000000000000000000 00 00000000000000000000000000000000 00 00000000000000000000000000000000 ffffffff 00000001 00000000 00200200       lo
";

const NONE_V6: &str = "\
00000000000000000000000000000000 00 00000000000000000000000000000000 00 00000000000000000000000000000000 ffffffff 00000001 00000000 00200200       lo
00000000000000000000000000000001 80 00000000000000000000000000000000 00 00000000000000000000000000000000 00000000 00000002 00000000 80200001       lo
";

fn ip(s: &str) -> IpAddr {
    s.parse().expect("an address")
}

#[cfg(target_endian = "little")]
#[test]
fn a_docker_networks_gateways() {
    assert_eq!(parse_v4(&docker_v4()), Ok(vec![ip("10.233.44.1")]));
    assert_eq!(parse_v6(DOCKER_V6), Ok(vec![ip("fd00:4c:44::1")]));
    assert_eq!(parse_v4(&padded(&[HEADER_V4])), Ok(vec![]), "--network none: the header alone");
    assert_eq!(parse_v6(NONE_V6), Ok(vec![]));
    assert_eq!(parse_v6(""), Ok(vec![]), "no IPv6 route");
}

#[test]
fn only_a_default_route_through_a_gateway_counts() {
    let v4 = |flags: &str, mask: &str| {
        parse_v4(&padded(&[HEADER_V4, &format!("eth0\t00000000\t012CE90A\t{flags}\t0\t0\t0\t{mask}\t0\t0\t0")]))
            .map(|g| g.len())
    };
    assert_eq!(v4("0003", "00000000"), Ok(1));
    assert_eq!(v4("0001", "00000000"), Ok(0), "no RTF_GATEWAY");
    assert_eq!(v4("0002", "00000000"), Ok(0), "not up");
    assert_eq!(v4("0003", "000000FF"), Ok(0), "a /8, not the default");
    let two = padded(&[
        HEADER_V4,
        "eth0\t00000000\t012CE90A\t0003\t0\t0\t100\t00000000\t0\t0\t0",
        "eth1\t00000000\t012CE90A\t0003\t0\t0\t200\t00000000\t0\t0\t0",
    ]);
    assert_eq!(parse_v4(&two).map(|g| g.len()), Ok(1), "the same gateway twice is named once");
}

#[test]
fn tables_are_read_strictly() {
    let line = |l: &str| parse_v4(&padded(&[HEADER_V4, l]));
    let bad = |what| Err(RouteError::Line { path: ROUTE_V4, line: 2, what });
    let fields = "not 11 tab-separated fields";
    let widths = "an address, a mask or the flags are not hexadecimal of the kernel's width";
    assert_eq!(parse_v4(&padded(&["Iface\tDestination"])), Err(RouteError::Header));
    assert_eq!(parse_v4(""), Err(RouteError::Header));
    assert_eq!(line("eth0\t00000000\t012CE90A\t0003\t0\t0\t0\t00000000\t0\t0"), bad(fields));
    assert_eq!(line("eth0\t00000000\t012CE90A\t0003\t0\t0\t0\t00000000\t0\t0\t0\t0"), bad(fields));
    assert_eq!(line("eth0\t0000000\t012CE90A\t0003\t0\t0\t0\t00000000\t0\t0\t0"), bad(widths));
    assert_eq!(line("eth0\t00000000\t+12CE90A\t0003\t0\t0\t0\t00000000\t0\t0\t0"), bad(widths));
    assert_eq!(line("eth0\t00000000\t012CE90A\t0x03\t0\t0\t0\t00000000\t0\t0\t0"), bad(widths));
    assert_eq!(
        line("eth0\t00000000\t012CE90A\t0003\t+0\t0\t0\t00000000\t0\t0\t0"),
        bad("a counter is not a decimal number")
    );
    assert_eq!(line("eth0\t00000000\t012CE90A\t0003\t0\t0\t-1\t00000000\t0\t0\t0").map(|g| g.len()), Ok(1));
    assert_eq!(line("a/b\t00000000\t012CE90A\t0003\t0\t0\t0\t00000000\t0\t0\t0"), bad("not an interface name"));
    assert_eq!(line("*\t00000000\t012CE90A\t0003\t0\t0\t0\t00000000\t0\t0\t0").map(|g| g.len()), Ok(1));
    let unended = docker_v4().trim_end_matches('\n').to_string();
    assert!(matches!(parse_v4(&unended), Err(RouteError::Line { what: "the last line does not end", .. })));

    let v6 = |l: &str| parse_v6(&format!("{l}\n")).map(|g| g.len());
    let zero = "00000000000000000000000000000000";
    let gw = "fd00004c004400000000000000000001";
    assert_eq!(v6(&format!("{zero} 00 {zero} 00 {gw} 00000400 00000001 00000000 00000003")), Ok(1), "no device");
    assert_eq!(
        v6(&format!("{zero} 00 {zero} 00 {gw} 00000400 00000001 00000000 00000003 eth0 x")),
        Err(RouteError::Line { path: ROUTE_V6, line: 1, what: "not 9 or 10 fields" })
    );
    assert_eq!(
        v6(&format!("{zero} 0 {zero} 00 {gw} 00000400 00000001 00000000 00000003 eth0")),
        Err(RouteError::Line { path: ROUTE_V6, line: 1, what: "a field is not hexadecimal of the kernel's width" })
    );
    assert_eq!(
        v6(&format!("{zero} 00 {zero} 00 {gw} 00000400 00000001 00000000 0000000g eth0")),
        Err(RouteError::Line { path: ROUTE_V6, line: 1, what: "a field is not hexadecimal of the kernel's width" })
    );
}

#[test]
fn checks_lines_name_a_gateway_inside_a_trusted_network() {
    let trusted = Trusted::new(vec![Network::parse("10.233.44.0/24").expect("a network")]);
    let inside = gateway_lines(&Ok(vec![ip("10.233.44.1")]), &trusted);
    assert_eq!(inside.len(), 1);
    assert!(
        inside[0]
            .starts_with("warning: the default gateway 10.233.44.1 lies inside the trusted network 10.233.44.0/24")
    );
    assert_eq!(
        gateway_lines(&Ok(vec![ip("172.17.0.1"), ip("fd00:4c:44::1")]), &trusted),
        vec![
            "default gateway 172.17.0.1: outside the trusted networks",
            "default gateway fd00:4c:44::1: outside the trusted networks"
        ]
    );
    assert_eq!(gateway_lines(&Ok(vec![]), &trusted), vec!["default gateway: none (no default route)"]);
    assert_eq!(
        gateway_lines(&Err(RouteError::Header), &trusted),
        vec![
            "note: the default gateway was not read (/proc/net/route does not start with the kernel's header): whether \
             it lies in a trusted network is not known"
        ]
    );
}

#[test]
fn runs_start_line_warns_only_for_a_gateway_inside() {
    let trusted = Trusted::new(vec![
        Network::parse("10.233.46.0/24").expect("a network"),
        Network::parse("fd00:4c:46::/64").expect("a network"),
    ]);
    assert_eq!(start_line(&Ok(vec![ip("172.17.0.1")]), &trusted), None, "outside: nothing at the start");
    assert_eq!(start_line(&Ok(vec![]), &trusted), None, "no default route: nothing at the start");
    let Some(StartLine::Warning(one)) = start_line(&Ok(vec![ip("172.17.0.1"), ip("10.233.46.1")]), &trusted) else {
        panic!("a warning for the gateway inside");
    };
    assert!(one.starts_with("the default gateway 10.233.46.1 (in 10.233.46.0/24) lies inside a trusted network: "));
    assert!(one.ends_with(INSIDE_HAZARD));
    let Some(StartLine::Warning(two)) = start_line(&Ok(vec![ip("10.233.46.1"), ip("fd00:4c:46::1")]), &trusted) else {
        panic!("one warning for both");
    };
    assert!(two.starts_with(
        "the default gateways 10.233.46.1 (in 10.233.46.0/24), fd00:4c:46::1 (in fd00:4c:46::/64) lie inside trusted \
         networks: "
    ));
    assert_eq!(
        start_line(&Err(RouteError::Header), &trusted),
        Some(StartLine::NotRead {
            text:
                "the default gateway was not read (/proc/net/route does not start with the kernel's header): whether \
                   it lies in a trusted network is not known"
                    .into(),
            unsupported: false
        })
    );
    assert!(matches!(
        start_line(&Err(RouteError::Unsupported), &trusted),
        Some(StartLine::NotRead { unsupported: true, .. })
    ));
}

#[cfg(target_os = "linux")]
#[test]
fn this_systems_tables_read() {
    assert!(default_gateways().is_ok(), "{:?}", default_gateways());
}
