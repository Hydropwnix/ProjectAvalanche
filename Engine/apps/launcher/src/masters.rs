use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

pub const MAX_MASTERS: usize = 8;
pub const MAX_TEXT: usize = 256;

fn dns_name(s: &str) -> Result<String, String> {
    if s.is_empty() || s.len() > 253 {
        return Err("a host name has 1 to 253 characters".into());
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
    ok.then(|| s.parse::<u16>().ok()).flatten().ok_or_else(|| format!("the port {s:?} is not 1 to 65535"))
}

pub fn canonical(text: &str) -> Result<String, String> {
    let text = text.trim();
    if text.is_empty() || text.len() > MAX_TEXT || !text.is_ascii() {
        return Err(format!("an address has 1 to {MAX_TEXT} ASCII characters"));
    }
    let (scheme, rest) = text.split_once("://").ok_or("an address starts with https:// or http://")?;
    let tls = match scheme.to_ascii_lowercase().as_str() {
        "https" => true,
        "http" => false,
        _ => return Err(format!("{scheme:?} is not https or http")),
    };
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    if let Some(c) = rest.chars().find(|c| matches!(c, '/' | '?' | '#' | '@' | '%' | '\\') || !c.is_ascii_graphic()) {
        return Err(format!("{c:?} in a master's address (only https://host[:port])"));
    }
    let default = if tls { 443 } else { 80 };
    let (host, port) = if let Some(v6) = rest.strip_prefix('[') {
        let (ip, after) = v6.split_once(']').ok_or("an IPv6 address in [ ] has no closing ]")?;
        let ip: Ipv6Addr = ip.parse().map_err(|_| format!("{ip:?} is not an IPv6 address"))?;
        if ip.is_unspecified() || ip.is_multicast() {
            return Err(format!("{ip} is not a master's address"));
        }
        let port = match after.strip_prefix(':') {
            Some(p) => port(p)?,
            None if after.is_empty() => default,
            None => return Err(format!("{after:?} after the IPv6 address")),
        };
        (format!("[{ip}]"), port)
    } else {
        if rest.matches(':').count() > 1 {
            return Err("an IPv6 address is written in [ ]".into());
        }
        let (h, p) = match rest.split_once(':') {
            Some((h, p)) => (h, port(p)?),
            None => (rest, default),
        };
        let host = match h.parse::<Ipv4Addr>() {
            Ok(ip) if ip.is_unspecified() || ip.is_multicast() => {
                return Err(format!("{ip} is not a master's address"));
            }
            Ok(ip) => IpAddr::V4(ip).to_string(),
            Err(_) => dns_name(h)?,
        };
        (host, p)
    };
    let scheme = if tls { "https" } else { "http" };
    Ok(if port == default { format!("{scheme}://{host}") } else { format!("{scheme}://{host}:{port}") })
}

pub fn add(list: &[String], text: &str) -> Result<Vec<String>, String> {
    let url = canonical(text)?;
    if list.contains(&url) {
        return Err(format!("{url} is on the list already"));
    }
    if list.len() >= MAX_MASTERS {
        return Err(format!("at most {MAX_MASTERS} masters"));
    }
    let mut out = list.to_vec();
    out.push(url);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_games_rule() {
        let ok = |t: &str| canonical(t);
        assert_eq!(ok("https://Master.Example.org"), Ok("https://master.example.org".into()));
        assert_eq!(ok("HTTPS://master.example.org:443/"), Ok("https://master.example.org".into()));
        assert_eq!(ok("https://master.example.org:8443"), Ok("https://master.example.org:8443".into()));
        assert_eq!(ok("http://127.0.0.1:27900"), Ok("http://127.0.0.1:27900".into()));
        assert_eq!(ok("http://[::1]:27900"), Ok("http://[::1]:27900".into()));
        assert_eq!(ok("http://192.0.2.1"), Ok("http://192.0.2.1".into()));
        for bad in [
            "",
            "master.example.org",
            "ftp://master.example.org",
            "https://",
            "https://user@master.example.org",
            "https://master.example.org/v1/servers",
            "https://master.example.org?x=1",
            "https://master.example.org#a",
            "https://master%2eexample.org",
            "https://master example.org",
            "https://master.example.org:0",
            "https://master.example.org:65536",
            "https://master.example.org:080",
            "https://-master.example.org",
            "https://master..example.org",
            "https://1.2.3",
            "https://::1",
            "https://[::1",
            "https://0.0.0.0",
            "https://[ff02::1]",
            "https://mäster.example.org",
            "https://master.example.org\\x",
            "javascript://alert(1)",
        ] {
            assert!(canonical(bad).is_err(), "{bad:?}");
        }
        let list = add(&[], "https://a.example.org").expect("added");
        assert!(add(&list, "HTTPS://A.example.org/").is_err(), "twice");
        let full: Vec<String> = (0..8).map(|i| format!("https://m{i}.example.org")).collect();
        assert!(add(&full, "https://m9.example.org").is_err());
    }
}
