pub mod guard;
pub mod index;
pub mod manifest;
pub mod md5check;
pub mod setup;

use std::collections::BTreeSet;
use std::path::Path;

pub use manifest::{Class, KNOWN_PRESSING, manifest};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub place: String,
    pub what: String,
}

pub struct Subset {
    pub classes: BTreeSet<Class>,
    pub members: usize,
    pub bytes: u64,
    pub known_pressing: bool,
}

fn marker_classes(m: &serde_json::Value) -> Option<BTreeSet<Class>> {
    let names = m["selection"]["classes"].as_array()?;
    let mut out = BTreeSet::new();
    for n in names {
        let n = n.as_str()?;
        let c = [Class::Rules, Class::Collision, Class::Ghosts, Class::Verify].into_iter().find(|c| c.name() == n)?;
        out.insert(c);
    }
    out.contains(&Class::Rules).then_some(out)
}

pub fn check_subset_configured(
    data: &Path,
    hash: bool,
    configured: &guard::Configured<'_>,
) -> Result<(Subset, Vec<Finding>), Vec<Finding>> {
    let f = |place: &Path, what: String| Finding { place: place.display().to_string(), what };
    let marker_path = data.join(ssx_assets::subset::MARKER);
    let Some(marker) = ssx_assets::subset::read_marker(data) else {
        let what = if std::fs::symlink_metadata(&marker_path).is_ok() {
            "does not read (or is a link, or larger than 64 KiB): run `avalanche-server setup` again".to_string()
        } else {
            "no rules data yet: run `avalanche-server setup <your SSX (2012) disc image>`".to_string()
        };
        return Err(vec![f(&marker_path, what)]);
    };
    if !ssx_assets::subset::is_complete(data) {
        return Err(vec![f(&marker_path, "the setup did not finish: run it again (finished members are kept)".into())]);
    }
    let Some(classes) = marker_classes(&marker) else {
        return Err(vec![f(&marker_path, "its classes do not read".into())]);
    };
    let known_pressing = marker["selection"]["known_pressing"] == serde_json::json!(true);
    let list = manifest().map_err(|e| vec![f(Path::new("avalanche-server"), e)])?;
    let index = index::read(data).map_err(|e| vec![f(&data.join(index::INDEX_FILE), e.to_string())])?;
    let entries = list.of(&classes);
    let mut findings = Vec::new();
    let mut bytes = 0u64;
    for e in &entries {
        let rel = e.member.data_path();
        let path = data.join(&rel);
        let Some(&(size, h)) = index.rows.get(&rel) else {
            findings.push(f(&data.join(index::INDEX_FILE), format!("{rel} is not in it: run setup again")));
            continue;
        };
        match std::fs::symlink_metadata(&path) {
            Err(_) => findings.push(f(&path, "missing: run setup again".into())),
            Ok(m) if !m.is_file() => findings.push(f(&path, "a link or a folder, not the member: refused".into())),
            Ok(m) if m.len() != size => {
                findings.push(f(&path, format!("{} bytes, the index says {size}: changed since the setup", m.len())))
            }
            Ok(_) => {
                bytes += size;
                if known_pressing && (size != e.size || h != e.blake3) {
                    findings.push(f(&path, "the index's hash is not the known disc's".into()));
                }
                if hash {
                    match index::hash_file(&path) {
                        Ok((s, d)) if (s, d) == (size, h) => {}
                        Ok(_) => {
                            findings.push(f(&path, "its BLAKE3 is not the index's: changed since the setup".into()))
                        }
                        Err(err) => findings.push(f(&path, err.to_string())),
                    }
                }
            }
        }
    }
    let wanted: BTreeSet<String> = entries.iter().map(|e| e.member.data_path()).collect();
    if index.rows.len() != wanted.len() {
        for extra in index.rows.keys().filter(|k| !wanted.contains(*k)).take(8) {
            findings.push(f(&data.join(index::INDEX_FILE), format!("{extra}: not a member of the marker's classes")));
        }
    }
    if let Err(why) = guard::prepare(data, &wanted, configured) {
        findings.push(f(data, why));
    }
    Ok((Subset { classes, members: entries.len(), bytes, known_pressing }, findings))
}

pub fn render(findings: &[Finding]) -> String {
    findings.iter().map(|f| format!("{}: {}", f.place, f.what)).collect::<Vec<_>>().join("\n")
}
