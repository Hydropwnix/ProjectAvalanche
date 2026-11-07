use ssx_content::PackageSource;
use ssx_content::blocklist::{Blocked, describe, hash_tree};

pub fn package_blocked(p: &PackageSource, blocked: &Blocked) -> Option<String> {
    if blocked.is_empty() {
        return None;
    }
    let files = match hash_tree(p.tree()) {
        Ok(f) => f,
        Err(e) => return Some(format!("its files could not be checked against the blocklists: {e}")),
    };
    let hits = blocked.hits(files.iter().map(|(path, hash)| (path.as_str(), hash)));
    (!hits.is_empty()).then(|| format!("blocked: {}", describe("it", &hits)))
}
