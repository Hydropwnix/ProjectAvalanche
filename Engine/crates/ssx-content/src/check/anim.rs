use std::collections::{BTreeMap, BTreeSet};

use super::{Checked, Severity};
use crate::anim::{AnimToml, Clip, build, gltf_limits, parse_toml};
use crate::id::ContentId;
use crate::limits::MAX_RECORD_BYTES;
use crate::model;
use crate::path::PackageTree;

pub type AnimFiles = BTreeMap<ContentId, (String, Clip)>;

pub(super) fn check_entry(c: &mut Checked, tree: &PackageTree, id: &ContentId, f: &str, named: &mut BTreeSet<String>) {
    let (toml, record) = if f.to_ascii_lowercase().ends_with(".toml") {
        let bytes = match tree.read(f, MAX_RECORD_BYTES) {
            Ok(b) => b,
            Err(e) => return c.push(Severity::Error, f, (0, 0), e.to_string()),
        };
        match parse_toml(&bytes) {
            Ok(t) => (t, true),
            Err(problems) => {
                for (line, col, message) in problems {
                    c.push(Severity::Error, f, (line, col), message);
                }
                return;
            }
        }
    } else {
        (AnimToml { gltf: Some(f.to_string()), animation: None, additive: false, floats: Vec::new() }, false)
    };
    if let Some(g) = &toml.gltf {
        named.insert(g.clone());
        let caps = gltf_limits();
        if let Ok(bytes) = tree.read(g, caps.max_file_bytes)
            && let Ok(files) = model::files_of(g, &bytes, &caps)
        {
            named.extend(files);
        }
    }
    let clip = match build(&toml, f, tree as &dyn model::FileReader) {
        Ok(clip) => clip,
        Err(e) => return c.push(Severity::Error, &e.file, (e.line, e.column), e.message),
    };
    let message = format!(
        "{id}: {} joint tracks, {} float tracks, {:.3} s ({} ticks){}{}",
        clip.joints.len(),
        clip.floats.len(),
        clip.duration,
        clip.ticks(),
        if clip.additive { ", additive" } else { "" },
        if record { "" } else { " (a glTF file alone: its only animation, not additive, no float tracks)" },
    );
    c.push(Severity::Note, f, (0, 0), message);
    c.anims.insert(id.clone(), (f.to_string(), clip));
}
