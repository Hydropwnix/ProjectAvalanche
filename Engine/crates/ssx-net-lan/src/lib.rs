use std::net::{IpAddr, Ipv4Addr};

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

pub const MAX_INTERFACES: usize = 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interface {
    pub name: String,
    pub address: Ipv4Addr,
    pub prefix: u8,
    pub up: bool,
    pub loopback: bool,
    pub point_to_point: bool,
    pub broadcast: bool,
}

impl Interface {
    pub fn mask(&self) -> Ipv4Addr {
        Ipv4Addr::from(mask_bits(self.prefix))
    }

    pub fn broadcast_address(&self) -> Ipv4Addr {
        Ipv4Addr::from(u32::from(self.address) | !mask_bits(self.prefix))
    }
}

fn mask_bits(prefix: u8) -> u32 {
    match prefix {
        0 => 0,
        p => u32::MAX << (32 - u32::from(p.min(32))),
    }
}

pub fn prefix_of(mask: Ipv4Addr) -> Option<u8> {
    let bits = u32::from(mask);
    let ones = bits.leading_ones();
    (bits.checked_shl(ones).unwrap_or(0) == 0).then_some(ones as u8)
}

pub fn interfaces() -> std::io::Result<Vec<Interface>> {
    #[cfg(unix)]
    return unix::interfaces();
    #[cfg(windows)]
    return windows::interfaces();
    #[cfg(not(any(unix, windows)))]
    return Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "no interface list on this system"));
}

pub fn broadcast_targets(interfaces: &[Interface]) -> Vec<Ipv4Addr> {
    let mut out: Vec<Ipv4Addr> = interfaces
        .iter()
        .filter(|i| i.up && i.broadcast && !i.loopback && !i.point_to_point)
        .filter(|i| (1..=30).contains(&i.prefix) && !i.address.is_unspecified() && !i.address.is_multicast())
        .map(Interface::broadcast_address)
        .collect();
    out.sort();
    out.dedup();
    out
}

pub fn on_link(source: IpAddr, interfaces: &[Interface]) -> bool {
    let v4 = match source {
        IpAddr::V4(v4) => v4,
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => v4,
            None => return v6.is_loopback(),
        },
    };
    if v4.is_loopback() {
        return true;
    }
    interfaces.iter().filter(|i| i.up).any(|i| {
        if i.point_to_point || i.prefix == 0 || i.prefix == 32 {
            return i.address == v4;
        }
        let m = mask_bits(i.prefix);
        u32::from(v4) & m == u32::from(i.address) & m
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iface(address: [u8; 4], prefix: u8) -> Interface {
        Interface {
            name: "eth0".into(),
            address: Ipv4Addr::from(address),
            prefix,
            up: true,
            loopback: false,
            point_to_point: false,
            broadcast: true,
        }
    }

    #[test]
    fn masks_and_prefixes() {
        for p in 0..=32u8 {
            let i = iface([10, 1, 2, 3], p);
            assert_eq!(prefix_of(i.mask()), Some(p), "/{p}");
        }
        assert_eq!(prefix_of(Ipv4Addr::new(255, 0, 255, 0)), None);
        assert_eq!(prefix_of(Ipv4Addr::new(0, 0, 0, 255)), None);
        assert_eq!(iface([192, 168, 1, 20], 24).broadcast_address(), Ipv4Addr::new(192, 168, 1, 255));
        assert_eq!(iface([10, 7, 200, 1], 20).broadcast_address(), Ipv4Addr::new(10, 7, 207, 255));
    }

    #[test]
    fn broadcast_targets_per_interface() {
        let mut list = vec![
            iface([192, 168, 1, 20], 24),
            iface([10, 7, 200, 1], 20),
            iface([192, 168, 1, 21], 24),
            iface([169, 254, 10, 2], 16),
            Interface { loopback: true, broadcast: false, ..iface([127, 0, 0, 1], 8) },
            Interface { point_to_point: true, broadcast: false, ..iface([10, 8, 0, 6], 32) },
            Interface { up: false, ..iface([172, 16, 4, 9], 24) },
            iface([100, 64, 0, 1], 31),
            iface([203, 0, 113, 9], 32),
            Interface { broadcast: false, ..iface([172, 20, 0, 2], 24) },
            iface([0, 0, 0, 0], 8),
        ];
        assert_eq!(
            broadcast_targets(&list),
            [Ipv4Addr::new(10, 7, 207, 255), Ipv4Addr::new(169, 254, 255, 255), Ipv4Addr::new(192, 168, 1, 255)]
        );
        list.clear();
        assert!(broadcast_targets(&list).is_empty());
    }

    #[test]
    fn on_link_sources_only() {
        let list = vec![
            iface([192, 168, 1, 20], 24),
            Interface { point_to_point: true, broadcast: false, ..iface([10, 8, 0, 6], 24) },
            Interface { up: false, ..iface([172, 16, 4, 9], 24) },
        ];
        let on = |s: &str| on_link(s.parse().expect("an address"), &list);
        assert!(on("192.168.1.77") && on("192.168.1.20") && on("192.168.1.255"));
        assert!(on("127.0.0.1") && on("127.45.0.9") && on("::1") && on("::ffff:192.168.1.5"));
        assert!(on("10.8.0.6"), "a point-to-point link's own address");
        for off in ["192.168.2.1", "8.8.8.8", "10.8.0.7", "172.16.4.10", "2001:db8::1", "fe80::1", "::ffff:8.8.8.8"] {
            assert!(!on(off), "{off}");
        }
    }

    #[test]
    fn the_systems_list_reads() {
        let list = interfaces().expect("the interface list");
        assert!(list.len() <= MAX_INTERFACES);
        assert!(list.iter().any(|i| i.loopback && i.address.is_loopback()), "{list:?}");
        assert!(!broadcast_targets(&list).iter().any(|b| b.is_loopback()));
    }
}
