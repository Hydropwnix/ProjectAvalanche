use serde::{Deserialize, Serialize};

pub const MAX_RESOURCES: u32 = 256;
pub const MAX_DOWNLOAD: u64 = 8 << 30;
pub const MAX_MANIFEST_BYTES: u64 = 8 << 20;
pub const CLIENT_MODS: [&str; 3] = ["presentation", "any", "none"];

pub fn hex32(bytes: &[u8; 32]) -> String {
    crate::keys::to_hex(bytes)
}

pub fn from_hex32(text: &str, what: &str) -> Result<[u8; 32], String> {
    if text.len() != 64 || !text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return Err(format!("{what}: not 64 lower-case hex digits"));
    }
    crate::keys::from_hex(text).map_err(|e| format!("{what}: {e}"))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceSummary {
    pub content_id: String,
    pub client_mods: String,
    pub client_scripts: bool,
    pub count: u32,
    pub download: u64,
    pub manifest: String,
    pub manifest_bytes: u64,
}

impl ResourceSummary {
    pub fn validate(&self) -> Result<(), String> {
        if !self.content_id.is_empty() {
            from_hex32(&self.content_id, "the content id")?;
        }
        if !CLIENT_MODS.contains(&self.client_mods.as_str()) {
            return Err(format!("the client-mod policy {:?} is not one of {CLIENT_MODS:?}", self.client_mods));
        }
        if self.count > MAX_RESOURCES {
            return Err(format!("{} resources, at most {MAX_RESOURCES}", self.count));
        }
        if self.download > MAX_DOWNLOAD {
            return Err(format!("a download of {} bytes, at most {MAX_DOWNLOAD}", self.download));
        }
        if self.count == 0 && self.download != 0 {
            return Err("a download without resources".into());
        }
        from_hex32(&self.manifest, "the manifest's hash")?;
        if self.manifest_bytes == 0 || self.manifest_bytes > MAX_MANIFEST_BYTES {
            return Err(format!("a manifest of {} bytes, 1 to {MAX_MANIFEST_BYTES}", self.manifest_bytes));
        }
        Ok(())
    }

    pub fn content_id(&self) -> Option<[u8; 32]> {
        from_hex32(&self.content_id, "the content id").ok()
    }

    pub fn manifest_hash(&self) -> Result<[u8; 32], String> {
        from_hex32(&self.manifest, "the manifest's hash")
    }
}

#[cfg(test)]
pub(crate) fn summary() -> ResourceSummary {
    ResourceSummary {
        content_id: "4e".repeat(32),
        client_mods: "presentation".into(),
        client_scripts: false,
        count: 2,
        download: 20_635,
        manifest: "ab".repeat(32),
        manifest_bytes: 2_431,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_summary_keeps_its_rules() {
        let good = summary();
        assert_eq!(good.validate(), Ok(()));
        assert_eq!(good.content_id(), Some([0x4e; 32]));
        let none = ResourceSummary { content_id: String::new(), count: 0, download: 0, ..good.clone() };
        assert_eq!(none.validate(), Ok(()), "a server without resources or ids");
        assert_eq!(none.content_id(), None);
        let bad = [
            ResourceSummary { content_id: "4E".repeat(32), ..good.clone() },
            ResourceSummary { content_id: "4e".repeat(31), ..good.clone() },
            ResourceSummary { content_id: format!("{}g", "4".repeat(63)), ..good.clone() },
            ResourceSummary { client_mods: "Presentation".into(), ..good.clone() },
            ResourceSummary { client_mods: String::new(), ..good.clone() },
            ResourceSummary { count: MAX_RESOURCES + 1, ..good.clone() },
            ResourceSummary { download: MAX_DOWNLOAD + 1, ..good.clone() },
            ResourceSummary { count: 0, ..good.clone() },
            ResourceSummary { manifest: String::new(), ..good.clone() },
            ResourceSummary { manifest: format!(" {}", "a".repeat(63)), ..good.clone() },
            ResourceSummary { manifest_bytes: 0, ..good.clone() },
            ResourceSummary { manifest_bytes: MAX_MANIFEST_BYTES + 1, ..good.clone() },
        ];
        for b in bad {
            assert!(b.validate().is_err(), "{b:?}");
        }
        let widest = ResourceSummary {
            client_mods: "presentation".into(),
            count: MAX_RESOURCES,
            download: MAX_DOWNLOAD,
            manifest_bytes: MAX_MANIFEST_BYTES,
            ..good
        };
        assert_eq!(widest.validate(), Ok(()));
        assert!(serde_json::to_string(&widest).unwrap().len() <= 280);
    }
}
