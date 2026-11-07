use std::collections::HashMap;

use bytes::Bytes;
use sha2::{Digest, Sha256};
use ssx_net_proto::keys::{KEY_SIZE, to_hex};
use ssx_net_proto::master::{BLOCKLIST_VERSION, Blocklist};

use crate::config::Config;

pub use ssx_net_proto::blocklist_file::{MAX_REASON_CHARS, RESOURCES_HEADER, SERVERS_HEADER, read};

#[derive(Default)]
pub struct Blocklists {
    pub servers: HashMap<[u8; KEY_SIZE], String>,
    pub resources: Vec<[u8; KEY_SIZE]>,
    pub published: Bytes,
    pub etag: String,
}

pub fn etag(body: &[u8]) -> String {
    let digest: [u8; 32] = Sha256::digest(body).into();
    format!("\"{}\"", &to_hex(&digest)[..32])
}

impl Blocklists {
    pub fn load(config: &Config) -> Result<Self, String> {
        let servers = match &config.blocked_servers {
            Some(p) => read(p, SERVERS_HEADER)?.into_iter().collect(),
            None => HashMap::new(),
        };
        let resources: Vec<[u8; KEY_SIZE]> = match &config.blocked_resources {
            Some(p) => read(p, RESOURCES_HEADER)?.into_iter().map(|(k, _)| k).collect(),
            None => Vec::new(),
        };
        let published = Bytes::from(
            Blocklist { blocklist_version: BLOCKLIST_VERSION, resources: resources.iter().map(to_hex).collect() }
                .to_json(),
        );
        let etag = etag(&published);
        Ok(Self { servers, resources, published, etag })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(servers: &str, resources: &str) -> Result<Blocklists, String> {
        let dir = std::env::temp_dir().join(format!("avalanche-master-blocklists-{}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let (s, r) = (dir.join("servers.txt"), dir.join("resources.txt"));
        std::fs::write(&s, servers).map_err(|e| e.to_string())?;
        std::fs::write(&r, resources).map_err(|e| e.to_string())?;
        let config = Config { blocked_servers: Some(s), blocked_resources: Some(r), ..Config::defaults() };
        let out = Blocklists::load(&config);
        let _ = std::fs::remove_dir_all(&dir);
        out
    }

    #[test]
    fn strict_files() {
        let k1 = "ab".repeat(32);
        let k2 = "CD".repeat(32);
        let good =
            with(&format!("{SERVERS_HEADER}\n{k1} spam servers\r\n{k2}\n"), &format!("{RESOURCES_HEADER}\n{k1}\n"))
                .expect("good");
        assert_eq!(good.servers.len(), 2);
        assert_eq!(good.servers.get(&[0xAB; 32]).map(String::as_str), Some("spam servers"));
        let published = Blocklist::from_json(&good.published).expect("reads back");
        assert_eq!(published.resources, vec![k1.clone()]);
        assert_eq!(good.etag, etag(&good.published));
        let r = format!("{RESOURCES_HEADER}\n");
        for bad in [
            String::new(),
            format!("{k1}\n"),
            format!("{SERVERS_HEADER}\n\n"),
            format!("{SERVERS_HEADER}\n{k1}\n{k1} again\n"),
            format!("{SERVERS_HEADER}\n{}\n", &k1[..63]),
            format!("{SERVERS_HEADER}\n{k1}x\n"),
            format!("{SERVERS_HEADER}\n{k1} a\u{7}b\n"),
            format!("{SERVERS_HEADER}\n{k1} {}\n", "r".repeat(201)),
            format!("# comment\n{SERVERS_HEADER}\n"),
        ] {
            assert!(with(&bad, &r).is_err(), "{bad:?}");
        }
        assert!(with(&format!("{SERVERS_HEADER}\n"), "Project Avalanche blocked servers v1\n").is_err(), "header");
    }
}
