use std::fmt;
use std::net::{IpAddr, SocketAddr};

pub const MAX_HOST_NAME: usize = 253;
pub const MAX_ADDRESS_TEXT: usize = 262;

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Host {
    Ip(IpAddr),
    Name(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ServerAddress {
    pub host: Host,
    pub port: u16,
}

impl fmt::Display for ServerAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.host {
            Host::Ip(IpAddr::V6(ip)) => write!(f, "[{ip}]:{}", self.port),
            Host::Ip(ip) => write!(f, "{ip}:{}", self.port),
            Host::Name(name) => write!(f, "{name}:{}", self.port),
        }
    }
}

fn host_name(s: &str) -> Result<String, String> {
    if s.is_empty() || s.len() > MAX_HOST_NAME {
        return Err(format!("a host name has 1 to {MAX_HOST_NAME} characters"));
    }
    for label in s.split('.') {
        let ok = !label.is_empty()
            && label.len() <= 63
            && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            && !label.starts_with('-')
            && !label.ends_with('-');
        if !ok {
            return Err(format!("{s:?} is not a host name (letters, digits and - between dots)"));
        }
    }
    if s.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return Err(format!("{s:?} is not an IPv4 address"));
    }
    Ok(s.to_ascii_lowercase())
}

fn port(s: &str) -> Result<u16, String> {
    let ok = !s.is_empty() && s.len() <= 5 && s.bytes().all(|b| b.is_ascii_digit()) && !s.starts_with('0');
    match ok.then(|| s.parse::<u16>().ok()).flatten() {
        Some(p) => Ok(p),
        None => Err(format!("the port {s:?} is not 1 to 65535")),
    }
}

impl ServerAddress {
    pub fn parse(text: &str, default_port: Option<u16>) -> Result<Self, String> {
        if text.is_empty() || text.len() > MAX_ADDRESS_TEXT {
            return Err(format!("an address has 1 to {MAX_ADDRESS_TEXT} characters"));
        }
        if let Some(rest) = text.strip_prefix('[') {
            let (ip, after) = rest.split_once(']').ok_or("an IPv6 address in [ ] has no closing ]")?;
            let ip: std::net::Ipv6Addr = ip.parse().map_err(|_| format!("{ip:?} is not an IPv6 address"))?;
            let port = match after.strip_prefix(':') {
                Some(p) => port(p)?,
                None if after.is_empty() => default_port.ok_or("the address has no port")?,
                None => return Err(format!("{after:?} after the IPv6 address")),
            };
            return Ok(Self { host: Host::Ip(IpAddr::V6(ip)), port });
        }
        if text.matches(':').count() > 1 {
            return Err("an IPv6 address is written in [ ] before its port".into());
        }
        let (host, port) = match text.split_once(':') {
            Some((h, p)) => (h, port(p)?),
            None => (text, default_port.ok_or("the address has no port")?),
        };
        let host = match host.parse::<std::net::Ipv4Addr>() {
            Ok(ip) => Host::Ip(IpAddr::V4(ip)),
            Err(_) => Host::Name(host_name(host)?),
        };
        Ok(Self { host, port })
    }

    pub fn resolve(&self) -> Result<SocketAddr, String> {
        self.resolve_until(None)
    }

    pub fn resolve_until(&self, cancel: Option<&std::sync::atomic::AtomicBool>) -> Result<SocketAddr, String> {
        match &self.host {
            Host::Ip(ip) => Ok(SocketAddr::new(*ip, self.port)),
            Host::Name(name) => super::resolve::resolve(
                name,
                self.port,
                super::resolve::RESOLVE_TIMEOUT,
                cancel,
                super::resolve::system,
            ),
        }
    }

    pub fn is_loopback(&self) -> bool {
        matches!(&self.host, Host::Ip(ip) if ip.is_loopback())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_read_strictly_and_print_one_way() {
        let ok = |t: &str, d: Option<u16>, shown: &str| {
            assert_eq!(ServerAddress::parse(t, d).map(|a| a.to_string()), Ok(shown.to_string()), "{t:?}");
        };
        ok("127.0.0.1:27450", None, "127.0.0.1:27450");
        ok("127.0.0.1", Some(27450), "127.0.0.1:27450");
        ok("[::1]:27500", None, "[::1]:27500");
        ok("[::1]", Some(27450), "[::1]:27450");
        ok("Alps.Example.org:1", None, "alps.example.org:1");
        ok("my-server:65535", None, "my-server:65535");
        for bad in [
            "",
            "127.0.0.1",
            "::1:27450",
            "[::1",
            "[::1]x",
            "[::1]:0",
            "host:0",
            "host:65536",
            "host:027450",
            "host:+1",
            "host:",
            "host :1",
            "-host:1",
            "host-:1",
            "a..b:1",
            "1.2.3:1",
            "1.2.3.4.5:1",
            "user@host:1",
            "host/path:1",
            "höst:1",
            "host:1/x",
            "http://host:1",
        ] {
            assert!(ServerAddress::parse(bad, None).is_err(), "{bad:?}");
        }
        assert!(ServerAddress::parse(&format!("{}:1", "a".repeat(64)), None).is_err(), "a label of 64");
        assert!(ServerAddress::parse(&format!("{}.a:1", vec!["b".repeat(63); 4].join(".")), None).is_err());
        assert!(ServerAddress::parse(&"a".repeat(MAX_ADDRESS_TEXT + 1), Some(1)).is_err());
        assert_eq!(ServerAddress::parse("[::1]:9", None).map(|a| a.is_loopback()), Ok(true));
    }
}
