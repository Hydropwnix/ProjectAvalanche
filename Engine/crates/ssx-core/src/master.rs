pub const OFFICIAL_MASTER: &str = "https://masterlist.liamrbsn.com";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Master(Inner);

#[derive(Clone, Debug, PartialEq, Eq)]
enum Inner {
    Official,
    #[cfg(feature = "dev-master")]
    Off,
    #[cfg(feature = "dev-master")]
    Local(String),
    #[cfg(feature = "dev-master")]
    Refused(String),
}

impl Master {
    pub fn url(&self) -> Option<&str> {
        match &self.0 {
            Inner::Official => Some(OFFICIAL_MASTER),
            #[cfg(feature = "dev-master")]
            Inner::Local(url) => Some(url),
            #[cfg(feature = "dev-master")]
            Inner::Off | Inner::Refused(_) => None,
        }
    }

    pub fn is_official(&self) -> bool {
        self.0 == Inner::Official
    }

    pub fn describe(&self) -> String {
        match &self.0 {
            Inner::Official => format!("{OFFICIAL_MASTER} (the project's master server)"),
            #[cfg(feature = "dev-master")]
            Inner::Off => format!("none (a development build: {DEV_MASTER_ENV} names no master)"),
            #[cfg(feature = "dev-master")]
            Inner::Local(url) => format!("{url} (a development build: {DEV_MASTER_ENV})"),
            #[cfg(feature = "dev-master")]
            Inner::Refused(why) => format!("none (a development build: {DEV_MASTER_ENV} refused: {why})"),
        }
    }
}

#[cfg(not(feature = "dev-master"))]
pub fn master() -> Master {
    Master(Inner::Official)
}

#[cfg(feature = "dev-master")]
pub const DEV_MASTER_ENV: &str = "SSX_DEV_MASTER";

#[cfg(feature = "dev-master")]
pub fn master() -> Master {
    static CHOSEN: std::sync::OnceLock<Master> = std::sync::OnceLock::new();
    CHOSEN.get_or_init(|| dev_master(std::env::var_os(DEV_MASTER_ENV).as_deref())).clone()
}

#[cfg(feature = "dev-master")]
pub fn dev_master(value: Option<&std::ffi::OsStr>) -> Master {
    let Some(value) = value else { return Master(Inner::Off) };
    let text = value.to_string_lossy();
    match local_master(&text) {
        Ok(url) => Master(Inner::Local(url)),
        Err(why) => Master(Inner::Refused(format!(
            "{}: {why}; it takes http://127.0.0.1:<port> or http://[::1]:<port>",
            crate::text::quoted_max(&text, crate::text::VALUE_CHARS)
        ))),
    }
}

#[cfg(feature = "dev-master")]
fn local_master(text: &str) -> Result<String, &'static str> {
    use std::net::{IpAddr, Ipv6Addr};
    let rest = text.strip_prefix("http://").ok_or("not http://")?;
    let (ip, port) = match rest.strip_prefix('[') {
        Some(v6) => {
            let (ip, after) = v6.split_once(']').ok_or("an IPv6 address without its ]")?;
            let ip: Ipv6Addr = ip.parse().map_err(|_| "not an IPv6 address in [ ]")?;
            (IpAddr::V6(ip), after.strip_prefix(':').ok_or("no :port after the address")?)
        }
        None => {
            let (ip, port) = rest.split_once(':').ok_or("no :port after the address")?;
            (IpAddr::V4(ip.parse().map_err(|_| "not an IP address (no name is looked up)")?), port)
        }
    };
    if !ip.is_loopback() {
        return Err("not a loopback address");
    }
    let digits = !port.is_empty() && port.len() <= 5 && port.bytes().all(|b| b.is_ascii_digit());
    let port: u16 = (digits && !port.starts_with('0'))
        .then(|| port.parse().ok())
        .flatten()
        .ok_or("the port is not 1 to 65535 (digits only, no path)")?;
    Ok(match ip {
        IpAddr::V4(v4) => format!("http://{v4}:{port}"),
        IpAddr::V6(v6) => format!("http://[{v6}]:{port}"),
    })
}

#[cfg(test)]
mod tests;
