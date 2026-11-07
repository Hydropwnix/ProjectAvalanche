use std::collections::BTreeMap;

use super::{Checked, Severity};
use crate::id::{ContentId, Kind};
use crate::limits::MAX_RECORD_BYTES;
use crate::manifest::{MANIFEST_FILE, Manifest};
use crate::path::PackageTree;
use crate::trick::{BASES, ROLES, TrickToml, parse_toml};

pub type TrickFiles = BTreeMap<ContentId, (String, TrickToml)>;

pub(super) fn check_entry(c: &mut Checked, tree: &PackageTree, m: &Manifest, id: &ContentId, f: &str) {
    if !f.to_ascii_lowercase().ends_with(".toml") {
        return c.push(Severity::Error, f, (0, 0), format!("{id}: a trick is its trick.toml"));
    }
    let bytes = match tree.read(f, MAX_RECORD_BYTES) {
        Ok(b) => b,
        Err(e) => return c.push(Severity::Error, f, (0, 0), e.to_string()),
    };
    let t = match parse_toml(&bytes) {
        Ok(t) => t,
        Err(problems) => {
            for (line, col, message) in problems {
                c.push(Severity::Error, f, (line, col), message);
            }
            return;
        }
    };
    let ns = m.resource.name.as_str();
    if id.namespace() != ns {
        let message = format!("{id}: an added trick is an id of the package's own namespace ({ns}:trick/…)");
        c.push(Severity::Error, MANIFEST_FILE, m.positions.entries.get(id).copied().unwrap_or((0, 0)), message);
        return;
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut ok = true;
    for (clip, (role, _)) in t.clips.iter().zip(ROLES) {
        let at = super::content::locate_key(&text, role);
        if clip.namespace() != ns {
            ok = false;
            let message = format!(
                "clips.{role}: {clip} is not this package's (a trick plays clips of its own package, `{ns}:anim/…`)"
            );
            c.push(Severity::Error, f, at, message);
        } else if !m.add.get(clip).is_some_and(|_| clip.kind() == Kind::Anim) {
            ok = false;
            let message = format!("clips.{role}: {clip} is no entry of this package (an `[add]` \"anim/…\" entry)");
            c.push(Severity::Error, f, at, message);
        }
    }
    for (name, field) in t.names.iter().zip(["name", "name_uber", "name_super_uber"]) {
        if name.namespace() != ns {
            ok = false;
            let at = super::content::locate_key(&text, field);
            let message = format!("{field}: {name} is not this package's string (`{ns}:string/<key>`)");
            c.push(Severity::Error, f, at, message);
        }
    }
    if ok {
        let message = format!(
            "{id}: GSENUM_TrickType {} on {} (goofy {}), in the places of the disc's {}; {} hand, {} points",
            t.value(),
            t.input.name(),
            t.input.mirror().name(),
            BASES[t.base].0,
            t.hand.name(),
            t.points
        );
        c.push(Severity::Note, f, (0, 0), message);
        c.tricks.insert(id.clone(), (f.to_string(), t));
    }
}

pub(super) fn cross_check(c: &mut Checked, tree: &PackageTree) {
    let mut by_input: BTreeMap<crate::trick::Input, ContentId> = BTreeMap::new();
    let mut found = Vec::new();
    for (id, (f, t)) in &c.tricks {
        let text = tree.read(f, MAX_RECORD_BYTES).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
        if let Some(other) = by_input.get(&t.input) {
            let message = format!("input: {} is {other}'s already (one trick an input)", t.input.name());
            found.push((Severity::Error, f.clone(), super::content::locate_key(&text, "input"), message));
        } else {
            by_input.insert(t.input, id.clone());
        }
        for (name, field) in t.names.iter().zip(["name", "name_uber", "name_super_uber"]) {
            if !c.m6.tables.iter().any(|table| table.entries.contains_key(name)) {
                let message = format!(
                    "{field}: no string table of the package has {name} (strings/en.toml: `{} = \"…\"`; the HUD would \
                     show the id)",
                    name.path()
                );
                found.push((Severity::Error, f.clone(), super::content::locate_key(&text, field), message));
            }
        }
    }
    for (severity, f, at, message) in found {
        c.push(severity, &f, at, message);
    }
}
