use std::net::Ipv4Addr;

pub fn default_gateway() -> Result<Ipv4Addr, String> {
    #[cfg(target_os = "linux")]
    return linux();
    #[cfg(windows)]
    return windows();
    #[cfg(not(any(target_os = "linux", windows)))]
    return Err("no default gateway lookup on this system: set [port_mapping] gateway".into());
}

#[cfg(target_os = "linux")]
fn linux() -> Result<Ipv4Addr, String> {
    use std::io::Read;
    let mut text = String::new();
    std::fs::File::open("/proc/net/route")
        .and_then(|f| f.take(256 * 1024).read_to_string(&mut text))
        .map_err(|e| format!("/proc/net/route: {e}"))?;
    parse_route(&text)
        .ok_or_else(|| "no default route with a gateway (/proc/net/route): set [port_mapping] gateway".into())
}

pub fn parse_route(text: &str) -> Option<Ipv4Addr> {
    let mut best: Option<(u32, Ipv4Addr)> = None;
    for line in text.lines().skip(1).take(4096) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        let hex = |i: usize| cols.get(i).and_then(|c| u32::from_str_radix(c, 16).ok());
        let (Some(dest), Some(gw), Some(flags), Some(mask)) = (hex(1), hex(2), hex(3), hex(7)) else { continue };
        let metric = cols.get(6).and_then(|c| c.parse::<u32>().ok()).unwrap_or(u32::MAX);
        if dest != 0 || mask != 0 || flags & 0x3 != 0x3 || gw == 0 {
            continue;
        }
        let ip = Ipv4Addr::from(gw.to_ne_bytes());
        if best.is_none_or(|(m, _)| metric < m) {
            best = Some((metric, ip));
        }
    }
    best.map(|(_, ip)| ip)
}

#[cfg(windows)]
fn windows() -> Result<Ipv4Addr, String> {
    use windows_sys::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, ERROR_NO_DATA, NO_ERROR};
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GAA_FLAG_INCLUDE_GATEWAYS, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST,
        GetAdaptersAddresses, IP_ADAPTER_ADDRESSES_LH,
    };
    use windows_sys::Win32::NetworkManagement::Ndis::IfOperStatusUp;
    use windows_sys::Win32::Networking::WinSock::{AF_INET, SOCKADDR_IN};
    let mut size: u32 = 15 * 1024;
    for _ in 0..4 {
        let mut buf = vec![0u64; (size as usize).div_ceil(8)];
        let first = buf.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        let flags =
            GAA_FLAG_INCLUDE_GATEWAYS | GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
        let r = unsafe { GetAdaptersAddresses(u32::from(AF_INET), flags, std::ptr::null(), first, &mut size) };
        match r {
            NO_ERROR => {}
            ERROR_NO_DATA => return Err("no network adapter: set [port_mapping] gateway".into()),
            ERROR_BUFFER_OVERFLOW if size <= 1024 * 1024 => continue,
            other => return Err(format!("GetAdaptersAddresses: error {other}")),
        }
        let mut best: Option<(u32, Ipv4Addr)> = None;
        let (mut adapter, mut walked) = (first.cast_const(), 0);
        while !adapter.is_null() && walked < 1024 {
            walked += 1;
            let a = unsafe { &*adapter };
            adapter = a.Next;
            if a.OperStatus != IfOperStatusUp {
                continue;
            }
            let g = a.FirstGatewayAddress;
            if g.is_null() {
                continue;
            }
            let g = unsafe { &*g };
            let sa = g.Address.lpSockaddr;
            if sa.is_null() || usize::try_from(g.Address.iSockaddrLength).unwrap_or(0) < size_of::<SOCKADDR_IN>() {
                continue;
            }
            let sin = unsafe { &*sa.cast::<SOCKADDR_IN>() };
            if sin.sin_family != AF_INET {
                continue;
            }
            let ip = Ipv4Addr::from(u32::from_be(unsafe { sin.sin_addr.S_un.S_addr }));
            if !ip.is_unspecified() && best.is_none_or(|(m, _)| a.Ipv4Metric < m) {
                best = Some((a.Ipv4Metric, ip));
            }
        }
        return best.map(|(_, ip)| ip).ok_or_else(|| "no adapter with a gateway: set [port_mapping] gateway".into());
    }
    Err("the adapter list kept growing".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_route_is_read() {
        let table = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n\
                     wlan0\t00000000\t0101A8C0\t0003\t0\t0\t600\t00000000\t0\t0\t0\n\
                     eth0\t00000000\t0100000A\t0003\t0\t0\t100\t00000000\t0\t0\t0\n\
                     eth0\t0000000A\t00000000\t0001\t0\t0\t100\t00FFFFFF\t0\t0\t0\n\
                     tun0\t00000000\t00000000\t0001\t0\t0\t50\t00000000\t0\t0\t0\n";
        let want = if cfg!(target_endian = "little") { Ipv4Addr::new(10, 0, 0, 1) } else { Ipv4Addr::new(1, 0, 0, 10) };
        assert_eq!(parse_route(table), Some(want), "the lowest metric with a gateway");
        assert_eq!(parse_route("Iface\n"), None);
        assert_eq!(parse_route("Iface\nx\tzz\tzz\n"), None);
    }
}
