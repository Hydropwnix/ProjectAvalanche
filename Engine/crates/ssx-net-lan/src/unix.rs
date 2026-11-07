use std::ffi::CStr;
use std::net::Ipv4Addr;

use crate::{Interface, MAX_INTERFACES, prefix_of};

unsafe fn ipv4(sa: *const libc::sockaddr) -> Option<Ipv4Addr> {
    if sa.is_null() {
        return None;
    }
    if i32::from(unsafe { (*sa).sa_family }) != libc::AF_INET {
        return None;
    }
    let sin = unsafe { &*sa.cast::<libc::sockaddr_in>() };
    Some(Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr)))
}

unsafe fn mask(sa: *const libc::sockaddr) -> Option<Ipv4Addr> {
    if sa.is_null() {
        return None;
    }
    let sin = unsafe { &*sa.cast::<libc::sockaddr_in>() };
    Some(Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr)))
}

pub fn interfaces() -> std::io::Result<Vec<Interface>> {
    let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
    if unsafe { libc::getifaddrs(&mut head) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut out = Vec::new();
    let mut cur = head;
    let mut walked = 0;
    while !cur.is_null() && walked < MAX_INTERFACES {
        walked += 1;
        let node = unsafe { &*cur };
        cur = node.ifa_next;
        let (Some(address), Some(mask)) = (unsafe { ipv4(node.ifa_addr) }, unsafe { mask(node.ifa_netmask) }) else {
            continue;
        };
        let Some(prefix) = prefix_of(mask) else { continue };
        let name = if node.ifa_name.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(node.ifa_name) }.to_string_lossy().chars().take(64).collect()
        };
        let flags = node.ifa_flags;
        let has = |f: libc::c_int| flags & f as libc::c_uint != 0;
        out.push(Interface {
            name,
            address,
            prefix,
            up: has(libc::IFF_UP),
            loopback: has(libc::IFF_LOOPBACK),
            point_to_point: has(libc::IFF_POINTOPOINT),
            broadcast: has(libc::IFF_BROADCAST),
        });
    }
    unsafe { libc::freeifaddrs(head) };
    Ok(out)
}
