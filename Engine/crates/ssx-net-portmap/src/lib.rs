mod exchange;
#[cfg(feature = "fake-gateways")]
pub mod fake;
pub mod gateway;
pub mod http;
pub mod igd;
mod mapper;
pub mod natpmp;
pub mod pcp;
mod session;
pub mod ssdp;
mod wire;
pub mod xml;

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, SocketAddrV4};

pub use mapper::{FirstAttempt, Level, Log, Mapper};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Protocol {
    Udp,
    Tcp,
}

impl fmt::Display for Protocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Protocol::Udp => "UDP",
            Protocol::Tcp => "TCP",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Port {
    pub protocol: Protocol,
    pub port: u16,
}

impl fmt::Display for Port {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.protocol, self.port)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Method {
    Upnp,
    Pcp,
    NatPmp,
}

impl Method {
    pub const ALL: [Method; 3] = [Method::Upnp, Method::Pcp, Method::NatPmp];

    pub fn name(self) -> &'static str {
        match self {
            Method::Upnp => "upnp",
            Method::Pcp => "pcp",
            Method::NatPmp => "natpmp",
        }
    }

    pub fn parse(s: &str) -> Option<Method> {
        Method::ALL.into_iter().find(|m| m.name() == s)
    }

    pub fn title(self) -> &'static str {
        match self {
            Method::Upnp => "UPnP IGD",
            Method::Pcp => "PCP",
            Method::NatPmp => "NAT-PMP",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    pub methods: Vec<Method>,
    pub gateway: Option<Ipv4Addr>,
    pub lifetime: u32,
    pub local: Option<Ipv4Addr>,
    pub ports: Vec<Port>,
    pub description: String,
}

#[derive(Clone, Debug)]
pub struct Targets {
    pub ssdp: SsdpTarget,
    pub pmp_port: u16,
}

#[derive(Clone, Debug)]
pub enum SsdpTarget {
    Multicast,
    Unicast(SocketAddrV4),
}

impl Default for Targets {
    fn default() -> Self {
        Targets { ssdp: SsdpTarget::Multicast, pmp_port: 5351 }
    }
}

impl Targets {
    pub fn loopback_only(&self, gateway: Ipv4Addr) -> bool {
        gateway.is_loopback() && matches!(self.ssdp, SsdpTarget::Unicast(a) if a.ip().is_loopback())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub kind: ErrorKind,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    Malformed,
    Io,
    Refused,
}

fn bounded(message: &str) -> String {
    ssx_core::text::shown_max(message, 300).to_string()
}

impl Error {
    pub fn malformed(message: impl Into<String>) -> Self {
        Error { kind: ErrorKind::Malformed, message: bounded(&message.into()) }
    }

    pub fn io(what: &str, e: std::io::Error) -> Self {
        Error { kind: ErrorKind::Io, message: bounded(&format!("{what}: {e}")) }
    }

    pub fn refused(message: impl Into<String>) -> Self {
        Error { kind: ErrorKind::Refused, message: bounded(&message.into()) }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub mapped: bool,
    pub method: Option<Method>,
    pub gateway: Option<Ipv4Addr>,
    pub external: Option<IpAddr>,
    pub lifetime: u32,
    pub maps: u64,
    pub renewals: u64,
    pub failures: u64,
    pub removals: u64,
    pub last_error: Option<String>,
    pub summary: String,
}
