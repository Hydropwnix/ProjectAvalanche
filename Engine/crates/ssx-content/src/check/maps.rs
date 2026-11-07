use std::collections::{BTreeMap, BTreeSet};

use super::content::{locate, own_or_reference};
use super::{Checked, Severity};
use crate::id::{ContentId, Kind, PORT_NAMESPACE};
use crate::manifest::{MANIFEST_FILE, Manifest};
use crate::map::{Map, PropLook, parse_toml, read_map, scene_limits};
use crate::model::{self, FileReader};
use crate::path::PackageTree;

pub type MapFiles = BTreeMap<ContentId, (String, Map)>;

pub(super) fn check_entry(
    c: &mut Checked,
    tree: &PackageTree,
    m: &Manifest,
    id: &ContentId,
    f: &str,
    at: (usize, usize),
    named: &mut BTreeSet<String>,
) {
    if !f.to_ascii_lowercase().ends_with(".toml") {
        let message = format!("{id}: a map is its map.toml, which names its scene (`scene = \"….glb\"`)");
        return c.push(Severity::Error, MANIFEST_FILE, at, message);
    }
    let bytes = match tree.read(f, crate::limits::MAX_RECORD_BYTES) {
        Ok(b) => b,
        Err(e) => return c.push(Severity::Error, f, (0, 0), e.to_string()),
    };
    let toml = match parse_toml(&bytes) {
        Ok(t) => t,
        Err(problems) => {
            for (line, col, message) in problems {
                c.push(Severity::Error, f, (line, col), message);
            }
            return;
        }
    };
    named.insert(toml.scene.clone());
    let caps = scene_limits();
    if let Ok(scene) = tree.read(&toml.scene, caps.max_file_bytes)
        && let Ok(files) = model::files_of(&toml.scene, &scene, &caps)
    {
        named.extend(files);
    }
    let map = match read_map(&bytes, f, tree as &dyn FileReader) {
        Ok(map) => map,
        Err(e) => return c.push(Severity::Error, &e.file, (0, 0), format!("{}{}", e.at, e.message)),
    };
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let refer = |c: &mut Checked, rid: &ContentId, what: &'static str| {
        if rid.namespace() != PORT_NAMESPACE {
            own_or_reference(c, m, (f, &text), rid, what);
        }
    };
    for (rid, what) in [
        (&map.toml.base, "map base drop"),
        (&map.toml.lighting, "map lighting"),
        (&map.toml.sky, "map sky"),
        (&map.toml.postfx, "map post-FX"),
        (&map.toml.music, "map music"),
        (&map.toml.midground, "map mid-ground"),
    ] {
        if let Some(rid) = rid {
            refer(c, rid, what);
        }
    }
    let scene_text = String::new();
    let mut seen = BTreeSet::new();
    for rid in map.definition.ids() {
        if rid.namespace() != PORT_NAMESPACE && seen.insert(rid.clone()) {
            let what = match rid.kind() {
                Kind::Object => "map object",
                Kind::Material => "map material",
                _ => "map marker",
            };
            own_or_reference(c, m, (&map.toml.scene, &scene_text), &rid, what);
        }
    }
    for n in &map.definition.notes {
        c.push(Severity::Note, &map.toml.scene, (0, 0), n.clone());
    }
    let d = &map.definition;
    let own = d.props.iter().filter(|p| matches!(p.look, PropLook::Mesh { .. })).count();
    c.push(
        Severity::Note,
        f,
        locate(&text, "scene"),
        format!(
            "{id}: {} — {} terrain meshes, {} collision and {} rail meshes ({} collision triangles), {} props ({own} own \
             meshes), {} spawns, {} checkpoints, {}finish, {} triggers, {} death volumes, {}boundary, {} markers, {} \
             helicopter points, {} cameras",
            if map.toml.base.is_some() { "a layout on a disc drop" } else { "a new track" },
            d.terrain.len(),
            d.collision.len(),
            d.rails.len(),
            d.collision_triangles(),
            d.props.len(),
            d.spawns.len(),
            d.checkpoints.len(),
            if d.finish.is_some() { "a " } else { "no " },
            d.triggers.len(),
            d.deaths.len(),
            if d.boundary.is_some() { "a " } else { "no " },
            d.markers.len(),
            d.heli_path.len(),
            d.cameras.len(),
        ),
    );
    c.maps.insert(id.clone(), (f.to_string(), map));
}
