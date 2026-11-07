use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AddressKey {
    V4([u8; 4]),
    V6Prefix([u8; 8]),
}

pub fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(IpAddr::V6(v6), IpAddr::V4),
        v4 => v4,
    }
}

pub fn address_key(ip: IpAddr) -> AddressKey {
    match canonical(ip) {
        IpAddr::V4(v4) => AddressKey::V4(v4.octets()),
        IpAddr::V6(v6) => {
            let mut prefix = [0u8; 8];
            prefix.copy_from_slice(&v6.octets()[..8]);
            AddressKey::V6Prefix(prefix)
        }
    }
}

fn in_v4(ip: Ipv4Addr, net: [u8; 4], prefix: u32) -> bool {
    let mask = u32::MAX.checked_shl(32 - prefix).unwrap_or(0);
    u32::from(ip) & mask == u32::from(Ipv4Addr::from(net)) & mask
}

fn in_v6(ip: Ipv6Addr, net: [u16; 8], prefix: u32) -> bool {
    let mask = u128::MAX.checked_shl(128 - prefix).unwrap_or(0);
    u128::from(ip) & mask == u128::from(Ipv6Addr::from(net)) & mask
}

const NOT_PUBLIC_V4: [([u8; 4], u32, &str); 15] = [
    ([0, 0, 0, 0], 8, "a \"this network\" address"),
    ([10, 0, 0, 0], 8, "a private address"),
    ([100, 64, 0, 0], 10, "a carrier-grade NAT address (shared, RFC 6598)"),
    ([127, 0, 0, 0], 8, "a loopback address"),
    ([169, 254, 0, 0], 16, "a link-local address"),
    ([172, 16, 0, 0], 12, "a private address"),
    ([192, 0, 0, 0], 24, "an IETF protocol address"),
    ([192, 0, 2, 0], 24, "a documentation address"),
    ([192, 88, 99, 0], 24, "a 6to4 relay address"),
    ([192, 168, 0, 0], 16, "a private address"),
    ([198, 18, 0, 0], 15, "a benchmarking address"),
    ([198, 51, 100, 0], 24, "a documentation address"),
    ([203, 0, 113, 0], 24, "a documentation address"),
    ([224, 0, 0, 0], 4, "a multicast address"),
    ([240, 0, 0, 0], 4, "a reserved address"),
];

const NOT_PUBLIC_V6: [([u16; 8], u32, &str); 9] = [
    ([0, 0, 0, 0, 0, 0, 0, 0], 128, "the unspecified address"),
    ([0, 0, 0, 0, 0, 0, 0, 1], 128, "a loopback address"),
    ([0x64, 0xff9b, 0, 0, 0, 0, 0, 0], 96, "a NAT64 address"),
    ([0x100, 0, 0, 0, 0, 0, 0, 0], 64, "a discard address"),
    ([0x2001, 0xdb8, 0, 0, 0, 0, 0, 0], 32, "a documentation address"),
    ([0x2002, 0, 0, 0, 0, 0, 0, 0], 16, "a 6to4 address"),
    ([0xfc00, 0, 0, 0, 0, 0, 0, 0], 7, "a unique local (private) address"),
    ([0xfe80, 0, 0, 0, 0, 0, 0, 0], 10, "a link-local address"),
    ([0xff00, 0, 0, 0, 0, 0, 0, 0], 8, "a multicast address"),
];

pub fn not_public(ip: IpAddr) -> Option<&'static str> {
    match canonical(ip) {
        IpAddr::V4(v4) => {
            NOT_PUBLIC_V4.into_iter().find(|&(net, prefix, _)| in_v4(v4, net, prefix)).map(|(_, _, why)| why)
        }
        IpAddr::V6(v6) => {
            NOT_PUBLIC_V6.into_iter().find(|&(net, prefix, _)| in_v6(v6, net, prefix)).map(|(_, _, why)| why)
        }
    }
}

pub fn wholly_not_public(net: &crate::forwarded::Network) -> bool {
    let prefix = u32::from(net.prefix());
    match net.addr() {
        IpAddr::V4(v4) => NOT_PUBLIC_V4.iter().any(|&(n, p, _)| p <= prefix && in_v4(v4, n, p)),
        IpAddr::V6(v6) => NOT_PUBLIC_V6.iter().any(|&(n, p, _)| p <= prefix && in_v6(v6, n, p)),
    }
}

pub fn never_a_server(ip: IpAddr) -> bool {
    let ip = canonical(ip);
    ip.is_unspecified() || ip.is_multicast() || ip == IpAddr::V4(Ipv4Addr::BROADCAST)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_and_not() {
        let not = |s: &str| not_public(s.parse().expect("an address"));
        for s in [
            "10.1.2.3",
            "172.31.255.1",
            "192.168.0.10",
            "127.0.0.1",
            "127.4.5.6",
            "169.254.1.1",
            "100.100.0.1",
            "224.0.0.1",
            "239.255.255.250",
            "255.255.255.255",
            "0.1.2.3",
            "198.19.0.1",
            "::1",
            "::",
            "fe80::1",
            "fd12:3456::1",
            "ff02::1",
            "2001:db8::5",
            "::ffff:10.0.0.1",
            "::ffff:127.0.0.1",
        ] {
            assert!(not(s).is_some(), "{s} is not public");
        }
        for s in ["8.8.8.8", "1.1.1.1", "172.32.0.1", "100.128.0.1", "2a00:1450:4001::200e", "::ffff:8.8.4.4"] {
            assert_eq!(not(s), None, "{s} is public");
        }
        assert_eq!(not("192.168.1.5"), Some("a private address"));
        assert!(never_a_server("0.0.0.0".parse().unwrap()) && never_a_server("::ffff:224.0.0.5".parse().unwrap()));
        assert!(!never_a_server("127.0.0.1".parse().unwrap()));
        let a: IpAddr = "2001:db8:1:2:aaaa::1".parse().unwrap();
        let b: IpAddr = "2001:db8:1:2:ffff::9".parse().unwrap();
        assert_eq!(address_key(a), address_key(b));
        assert_eq!(address_key("::ffff:192.0.2.1".parse().unwrap()), AddressKey::V4([192, 0, 2, 1]));
    }

    #[test]
    fn trusted_networks_wholly_private_or_not() {
        let net = |s: &str| crate::forwarded::Network::parse(s).expect("a network");
        for private in ["10.0.1.0/24", "172.18.0.0/16", "192.168.0.0/16", "127.0.0.1", "fd00:1::/64", "100.64.0.0/10"] {
            assert!(wholly_not_public(&net(private)), "{private}");
        }
        for public in ["0.0.0.0/1", "8.8.8.0/24", "172.0.0.0/8", "2a00::/16", "fc00::/6", "203.0.112.0/23"] {
            assert!(!wholly_not_public(&net(public)), "{public}");
        }
    }
}
