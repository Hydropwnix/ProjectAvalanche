use std::collections::BTreeMap;

use super::content::{locate, own_or_reference};
use super::{Checked, Severity};
use crate::id::{ContentId, Kind};
use crate::manifest::{MANIFEST_FILE, Manifest};
use crate::marker::{Look, MarkerDef, parse_toml};
use crate::path::PackageTree;

pub type MarkerFiles = BTreeMap<ContentId, (String, MarkerDef)>;

pub(super) fn check_entry(
    c: &mut Checked,
    tree: &PackageTree,
    m: &Manifest,
    id: &ContentId,
    f: &str,
    at: (usize, usize),
) {
    if !f.to_ascii_lowercase().ends_with(".toml") {
        let message = format!(
            "{id}: a marker is its marker.toml; a glTF model is an [add] \"model/…\" entry the marker names with `model`"
        );
        return c.push(Severity::Error, MANIFEST_FILE, at, message);
    }
    let bytes = match tree.read(f, crate::limits::MAX_RECORD_BYTES) {
        Ok(b) => b,
        Err(e) => return c.push(Severity::Error, f, (0, 0), e.to_string()),
    };
    let def = match parse_toml(&bytes) {
        Ok(d) => d,
        Err(problems) => {
            for (line, col, message) in problems {
                c.push(Severity::Error, f, (line, col), message);
            }
            return;
        }
    };
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let refer = |c: &mut Checked, rid: &ContentId, kind: Kind, what: &'static str| {
        if rid.kind() != kind {
            c.push(Severity::Error, f, locate(&text, rid.as_str()), format!("{what} {rid} is not a {kind} id"));
        } else {
            own_or_reference(c, m, (f, &text), rid, what);
        }
    };
    if let Look::Model(model) = &def.look {
        refer(c, model, Kind::Model, "marker model");
    }
    if let Some(icon) = &def.icon {
        refer(c, icon, Kind::Texture, "marker icon");
    }
    if let Some(sound) = &def.sound {
        refer(c, sound, Kind::Sound, "marker sound");
    }
    c.markers.insert(id.clone(), (f.to_string(), def));
}
