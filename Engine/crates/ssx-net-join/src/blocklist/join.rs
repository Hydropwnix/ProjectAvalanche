use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ssx_content::blocklist::{Blocked, Follow, Hit, describe, now_secs};
use ssx_content::delivery::{ClientManifest, FileStore};
use ssx_core::text::quoted;

use super::Outcome;

pub fn hits(manifest: &ClientManifest, blocked: &Blocked) -> Vec<(String, Vec<Hit>)> {
    if blocked.is_empty() {
        return Vec::new();
    }
    manifest
        .resources
        .iter()
        .filter_map(|r| {
            let hits = blocked.hits(r.files.iter().map(|f| (f.path.as_str(), &f.hash)));
            (!hits.is_empty()).then(|| (r.name.clone(), hits))
        })
        .collect()
}

pub struct Checked {
    pub asked: Vec<String>,
    pub verdict: Result<(), String>,
}

pub fn check(manifest: &ClientManifest, follow: &Follow, store: &FileStore, cancel: &Arc<AtomicBool>) -> Checked {
    let asked = super::refresh_until(follow, &None, false, now_secs(), Some(Arc::clone(cancel)))
        .iter()
        .filter(|o| o.result != Ok(super::Refreshed::NotDue))
        .map(Outcome::line)
        .collect();
    let verdict =
        if cancel.load(Ordering::Acquire) { Err("cancelled".into()) } else { refuse_blocked(manifest, follow, store) };
    Checked { asked, verdict }
}

pub fn reason(found: &[(String, Vec<Hit>)]) -> Option<String> {
    let (name, first) = found.first()?;
    let others = found.len() - 1;
    Some(format!(
        "{}{}",
        describe(&format!("the resource {}", quoted(name)), first),
        if others > 0 { format!(" (and {others} more resources hold blocked files)") } else { String::new() }
    ))
}

pub fn refuse_blocked(manifest: &ClientManifest, follow: &Follow, store: &FileStore) -> Result<(), String> {
    let followed = follow.load(now_secs());
    let found = hits(manifest, &followed.blocked);
    let Some(why) = reason(&found) else { return Ok(()) };
    for (_, hits) in &found {
        for h in hits {
            store.forget(&h.hash);
        }
    }
    Err(format!("{why}; nothing of it was fetched"))
}
