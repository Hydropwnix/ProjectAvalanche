use std::collections::BTreeMap;

use super::{Checked, Severity};
use crate::gamemode::{GamemodeToml, parse_toml};
use crate::id::ContentId;
use crate::limits::MAX_RECORD_BYTES;
use crate::manifest::{MANIFEST_FILE, Manifest, PackageType};
use crate::path::PackageTree;

pub type GamemodeFiles = BTreeMap<ContentId, (String, GamemodeToml)>;

pub(super) fn check_entry(c: &mut Checked, tree: &PackageTree, m: &Manifest, id: &ContentId, f: &str) {
    let at = m.positions.entries.get(id).copied().unwrap_or((0, 0));
    if !f.to_ascii_lowercase().ends_with(".toml") {
        return c.push(Severity::Error, MANIFEST_FILE, at, format!("{id}: a gamemode is its gamemode.toml"));
    }
    let ns = m.resource.name.as_str();
    if id.namespace() != ns {
        let message = format!("{id}: a gamemode is an id of the package's own namespace ({ns}:gamemode/…)");
        return c.push(Severity::Error, MANIFEST_FILE, at, message);
    }
    if m.resource.kind != PackageType::Gamemode {
        let message = format!(
            "{id}: a gamemode comes in a package of type \"gamemode\" (resource.toml `type = \"gamemode\"`): its \
             server scripts are the gamemode's"
        );
        c.push(Severity::Error, MANIFEST_FILE, (0, 0), message);
    }
    let bytes = match tree.read(f, MAX_RECORD_BYTES) {
        Ok(b) => b,
        Err(e) => return c.push(Severity::Error, f, (0, 0), e.to_string()),
    };
    let g = match parse_toml(&bytes) {
        Ok(g) => g,
        Err(problems) => {
            for (line, col, message) in problems {
                c.push(Severity::Error, f, (line, col), message);
            }
            return;
        }
    };
    let text = String::from_utf8_lossy(&bytes);
    let mut ok = true;
    for e in &g.maps {
        ok &= super::content::own_or_reference(c, m, (f, &text), &e.map, "map.id") || e.map.namespace() != ns;
    }
    if ok {
        c.gamemodes.insert(id.clone(), (f.to_string(), g));
    }
}
