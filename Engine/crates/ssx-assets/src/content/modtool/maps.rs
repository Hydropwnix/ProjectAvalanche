use std::collections::BTreeSet;
use std::path::Path;

use ssx_content::check::{Checked, Severity};
use ssx_content::map::{MAX_COLLISION_TRIANGLES, PropLook};
use ssx_content::object::{ObjectCollision, ObjectDef};
use ssx_content::{BASE_NAMESPACE, ContentId, PORT_NAMESPACE, Source};

use super::check::push;
use crate::content::markers::PortMarkers;
use crate::content::models::DiscModels;
use crate::content::objects::DiscRecords;

pub(super) fn check(c: &mut Checked, big: &Path, base: Option<&BTreeSet<ContentId>>) {
    let maps: Vec<_> = c.maps.iter().map(|(id, (f, m))| (id.clone(), f.clone(), m.clone())).collect();
    if maps.is_empty() {
        return;
    }
    let mut records: Option<Result<DiscRecords, String>> = None;
    for (id, file, map) in maps {
        let scene = map.toml.scene.clone();
        for (rid, what) in [(&map.toml.sky, "sky"), (&map.toml.postfx, "postfx"), (&map.toml.music, "music")] {
            if let Some(rid) = rid.as_ref().filter(|r| r.namespace() == PORT_NAMESPACE)
                && *rid != ssx_content::sky::scattering_id()
            {
                push(
                    c,
                    Severity::Error,
                    &file,
                    (0, 0),
                    format!("{what} {rid}: the port's own sky is ava:sky/scattering"),
                );
            }
        }
        for mk in map.definition.markers.iter().filter(|mk| mk.marker.namespace() == PORT_NAMESPACE) {
            if !PortMarkers.has(&mk.marker) {
                let message = format!(
                    "node \"{}\" ava_marker {}: the port's markers are ava:marker/{{ring, cylinder, arrow, chevron, beam, \
                     gate}}",
                    mk.node, mk.marker
                );
                push(c, Severity::Error, &scene, (0, 0), message);
            }
        }
        if let Some(b) = map.toml.base.as_ref().filter(|b| base.is_some_and(|ids| ids.contains(b))) {
            match crate::content::GameContent::of(big).layout_base(b) {
                Ok(l) => {
                    let message =
                        format!("{id}: a layout on {b}: {} drop {} (ge_drop 0x{:08x})", l.track, l.index, l.ge_drop);
                    push(c, Severity::Note, &file, (0, 0), message);
                }
                Err(e) => push(c, Severity::Error, &file, (0, 0), format!("base {b}: {e:#}")),
            }
        }
        let content = crate::content::GameContent::of(big);
        let d = &map.definition;
        let named = d.terrain.iter().filter_map(|t| t.material.as_ref().map(|m| (&t.node, m)));
        for (node, mid) in named.chain(d.rails.iter().filter_map(|r| r.material.as_ref().map(|m| (&r.node, m)))) {
            if base.is_some_and(|ids| ids.contains(mid))
                && let Err(e) = content.map_terrain_material(mid)
            {
                push(c, Severity::Error, &scene, (0, 0), format!("node \"{node}\" ava_material: {e:#}"));
            }
        }
        if let Some(t) = map.toml.midground.as_ref().filter(|t| base.is_some_and(|ids| ids.contains(t)))
            && let Err(e) = content.map_midground(t)
        {
            push(c, Severity::Error, &file, (0, 0), format!("midground {t}: {e:#}"));
        }
        let mut triangles = map.definition.collision_triangles();
        let mut placed = BTreeSet::new();
        for p in &map.definition.props {
            let PropLook::Object(oid) = &p.look else { continue };
            if oid.namespace() != BASE_NAMESPACE || !base.is_some_and(|ids| ids.contains(oid)) {
                continue;
            }
            let r = records.get_or_insert_with(|| {
                DiscModels::index(big)
                    .map(|m| DiscRecords::new(big, std::sync::Arc::new(m)))
                    .map_err(|e| format!("{e:#}"))
            });
            let def = match r {
                Ok(r) => r
                    .load(oid)
                    .map_err(|e| e.to_string())
                    .and_then(|a| a.into_value::<ObjectDef>().ok_or_else(|| "not an object record".to_string())),
                Err(e) => Err(format!("the disc's models cannot be indexed: {e}")),
            };
            let def = match def {
                Ok(d) => d,
                Err(e) => {
                    push(c, Severity::Error, &scene, (0, 0), format!("node \"{}\" ava_object {oid}: {e}", p.node));
                    continue;
                }
            };
            if def.controller.is_some() || def.rigid_body.is_some() {
                let message = format!(
                    "node \"{}\" ava_object {oid}: an entity with a controller or a rigid body, which a map does not \
                     place",
                    p.node
                );
                push(c, Severity::Error, &scene, (0, 0), message);
            }
            if let ObjectCollision::Decoded { collision, .. } = &def.collision {
                triangles += collision.triangle_count();
            }
            placed.insert(oid.clone());
        }
        if triangles > MAX_COLLISION_TRIANGLES {
            let message = format!(
                "{id}: {triangles} collision triangles with the disc objects it places; at most \
                 {MAX_COLLISION_TRIANGLES}, the disc's largest track (a package object's collision counts too, at load)"
            );
            push(c, Severity::Error, &file, (0, 0), message);
        } else if !placed.is_empty() {
            let message = format!(
                "{id}: {triangles} collision triangles with the {} disc objects it places (at most {MAX_COLLISION_TRIANGLES})",
                placed.len()
            );
            push(c, Severity::Note, &file, (0, 0), message);
        }
        if map.toml.base.is_none() && map.definition.deaths.is_empty() {
            let message = format!(
                "{id}: a new track without a death volume: a rider who leaves the ground falls until the ride ends \
                 (the game has no other floor: `content.md` §22.1 (c))"
            );
            push(c, Severity::Warning, &file, (0, 0), message);
        }
    }
}
