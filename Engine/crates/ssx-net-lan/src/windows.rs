use std::net::Ipv4Addr;

use windows_sys::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, ERROR_NO_DATA, NO_ERROR};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST, GetAdaptersAddresses, IF_TYPE_PPP,
    IF_TYPE_SOFTWARE_LOOPBACK, IP_ADAPTER_ADDRESSES_LH,
};
use windows_sys::Win32::NetworkManagement::Ndis::IfOperStatusUp;
use windows_sys::Win32::Networking::WinSock::{AF_INET, SOCKADDR_IN};

use crate::{Interface, MAX_INTERFACES};

const IF_TYPE_TUNNEL: u32 = 131;
const FIRST_BUFFER: u32 = 15 * 1024;
const MAX_BUFFER: u32 = 1024 * 1024;

pub fn interfaces() -> std::io::Result<Vec<Interface>> {
    let mut size = FIRST_BUFFER;
    for _ in 0..4 {
        let mut buf = vec![0u64; (size as usize).div_ceil(8)];
        let first = buf.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
        let r = unsafe { GetAdaptersAddresses(u32::from(AF_INET), flags, std::ptr::null(), first, &mut size) };
        match r {
            NO_ERROR => return Ok(walk(first)),
            ERROR_NO_DATA => return Ok(Vec::new()),
            ERROR_BUFFER_OVERFLOW if size <= MAX_BUFFER => continue,
            other => return Err(std::io::Error::from_raw_os_error(other as i32)),
        }
    }
    Err(std::io::Error::other("the adapter list kept growing"))
}

fn walk(first: *const IP_ADAPTER_ADDRESSES_LH) -> Vec<Interface> {
    let mut out = Vec::new();
    let mut adapter = first;
    let mut walked = 0;
    while !adapter.is_null() && walked < MAX_INTERFACES {
        walked += 1;
        let a = unsafe { &*adapter };
        adapter = a.Next;
        let loopback = a.IfType == IF_TYPE_SOFTWARE_LOOPBACK;
        let point_to_point = a.IfType == IF_TYPE_PPP || a.IfType == IF_TYPE_TUNNEL;
        let name = if a.AdapterName.is_null() {
            String::new()
        } else {
            unsafe { std::ffi::CStr::from_ptr(a.AdapterName.cast()) }.to_string_lossy().chars().take(64).collect()
        };
        let mut unicast = a.FirstUnicastAddress;
        while !unicast.is_null() && walked < MAX_INTERFACES {
            walked += 1;
            let u = unsafe { &*unicast };
            unicast = u.Next;
            let sa = u.Address.lpSockaddr;
            if sa.is_null() || usize::try_from(u.Address.iSockaddrLength).unwrap_or(0) < size_of::<SOCKADDR_IN>() {
                continue;
            }
            let sin = unsafe { &*sa.cast::<SOCKADDR_IN>() };
            if sin.sin_family != AF_INET {
                continue;
            }
            let address = Ipv4Addr::from(u32::from_be(unsafe { sin.sin_addr.S_un.S_addr }));
            out.push(Interface {
                name: name.clone(),
                address,
                prefix: u.OnLinkPrefixLength.min(32),
                up: a.OperStatus == IfOperStatusUp,
                loopback,
                point_to_point,
                broadcast: !loopback && !point_to_point,
            });
        }
    }
    out
}
