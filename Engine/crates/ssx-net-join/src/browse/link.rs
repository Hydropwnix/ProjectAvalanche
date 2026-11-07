use super::address::{MAX_ADDRESS_TEXT, ServerAddress};

pub const SCHEME: &str = "avalanche";

pub const MAX_LINK: usize = "avalanche://connect/".len() + MAX_ADDRESS_TEXT;

pub fn parse_link(link: &str) -> Result<ServerAddress, String> {
    if link.len() > MAX_LINK {
        return Err(format!("the link has more than {MAX_LINK} characters"));
    }
    let (scheme, rest) = link.split_once("://").ok_or("not a link (no ://)")?;
    if !scheme.eq_ignore_ascii_case(SCHEME) {
        return Err(format!("the link's scheme is {scheme:?}, not {SCHEME}"));
    }
    let address = rest.strip_prefix("connect/").ok_or("the link is not avalanche://connect/host:port")?;
    if let Some(c) = address.chars().find(|c| matches!(c, '/' | '?' | '#' | '@' | '%' | '\\') || c.is_whitespace()) {
        return Err(format!("the link has {c:?} after its address (only host:port may follow connect/)"));
    }
    ServerAddress::parse(address, None).map_err(|e| format!("the link's address: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_read_strictly() {
        let ok = |l: &str, shown: &str| assert_eq!(parse_link(l).map(|a| a.to_string()), Ok(shown.to_string()), "{l}");
        ok("avalanche://connect/127.0.0.1:27450", "127.0.0.1:27450");
        ok("AVALANCHE://connect/[::1]:27450", "[::1]:27450");
        ok("avalanche://connect/Alps.example.org:27500", "alps.example.org:27500");
        for bad in [
            "",
            "avalanche:connect/1.2.3.4:1",
            "avalanche:///connect/1.2.3.4:1",
            "fivem://connect/1.2.3.4:1",
            "javascript://connect/1.2.3.4:1",
            "avalanche://join/1.2.3.4:1",
            "avalanche://Connect/1.2.3.4:1",
            "avalanche://connect/1.2.3.4",
            "avalanche://connect/1.2.3.4:1/",
            "avalanche://connect/1.2.3.4:1?password=x",
            "avalanche://connect/1.2.3.4:1#x",
            "avalanche://connect/user@1.2.3.4:1",
            "avalanche://connect/1.2.3.4%3A1",
            "avalanche://connect/ 1.2.3.4:1",
            "avalanche://connect/1.2.3.4:1 --password-file /etc/shadow",
            "avalanche://connect/1.2.3.4:1\n",
            "avalanche://connect/",
            "avalanche://connect",
        ] {
            assert!(parse_link(bad).is_err(), "{bad:?}");
        }
        assert!(parse_link(&format!("avalanche://connect/{}:1", "a.".repeat(200))).is_err());
    }
}
