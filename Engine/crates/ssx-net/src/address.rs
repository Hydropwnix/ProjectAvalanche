use std::net::IpAddr;

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

pub fn same_address(a: IpAddr, b: IpAddr) -> bool {
    canonical(a) == canonical(b)
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, Ipv6Addr};

    use super::*;

    #[test]
    fn mapped_addresses_and_prefixes() {
        let v4 = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));
        let mapped = IpAddr::V6(Ipv4Addr::new(192, 0, 2, 1).to_ipv6_mapped());
        assert!(same_address(v4, mapped));
        assert_eq!(address_key(v4), address_key(mapped));
        let a: IpAddr = "2001:db8:1:2:aaaa::1".parse::<Ipv6Addr>().unwrap().into();
        let b: IpAddr = "2001:db8:1:2:ffff::9".parse::<Ipv6Addr>().unwrap().into();
        let c: IpAddr = "2001:db8:1:3::1".parse::<Ipv6Addr>().unwrap().into();
        assert_eq!(address_key(a), address_key(b));
        assert_ne!(address_key(a), address_key(c));
        assert!(!same_address(a, b));
    }
}
