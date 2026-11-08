use std::collections::BTreeSet;
use std::path::Path;

use ssx_content::check::{Checked, Severity, locate_key};
use ssx_content::manifest::{MANIFEST_FILE, Manifest};
use ssx_content::material::{MaterialDef, VECTOR_PARAMETERS, missing_tiles, unread_effect};
use ssx_content::{BASE_NAMESPACE, ContentId, Kind};
use ssx_formats::fxo::Effect;

use super::check::push;
use crate::content::models::DiscModels;
use crate::content::objects::DiscRecords;

fn effect_parameters(big: &Path, effect: &str) -> Result<BTreeSet<String>, String> {
    let stem = effect.strip_suffix(".fx").unwrap_or(effect);
    let archives = crate::content::vaults::archives(big).map_err(|e| format!("{e:#}"))?;
    let path = archives
        .iter()
        .map(|a| a.join("data/xenon/shaders").join(format!("{stem}.fxo")))
        .find(|p| p.is_file())
        .ok_or_else(|| format!("{stem}.fxo is not in the data folder's shaders"))?;
    let data = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let fx = Effect::parse(&data).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(fx.parameters.into_iter().filter_map(|p| p.name).collect())
}

fn check_parameters(c: &mut Checked, big: &Path, file: &str, text: &str, m: &MaterialDef) {
    match effect_parameters(big, &m.effect) {
        Ok(names) => {
            for key in m.textures.keys() {
                if !names.contains(&format!("{key}Sampler")) {
                    let message = format!("{key}: {} does not sample a texture of that name", m.effect);
                    push(c, Severity::Error, file, locate_key(text, key), message);
                }
            }
            for key in m.floats.keys().chain(m.vectors.keys()) {
                if !names.contains(key) {
                    let message = format!("{key}: {} has no parameter of that name", m.effect);
                    push(c, Severity::Error, file, locate_key(text, key), message);
                }
            }
            for (texture, tile) in missing_tiles(m, &names) {
                let example = if VECTOR_PARAMETERS.contains(&tile) { "[1.0, 1.0]" } else { "1.0" };
                let message = format!(
                    "{texture} without {tile}: {} multiplies the texture's coordinates by {tile}, whose default is 0 \
                     (one texel of the texture everywhere); set it, e.g. {tile} = {example}",
                    m.effect
                );
                push(c, Severity::Error, file, locate_key(text, texture), message);
            }
        }
        Err(e) => push(c, Severity::Error, file, (0, 0), format!("{}: parameters not checked: {e}", m.effect)),
    }
}

pub(super) fn check(c: &mut Checked, m: &Manifest, big: &Path, base: Option<&BTreeSet<ContentId>>) {
    for r in c.references.clone() {
        let known = match r.id.namespace() {
            BASE_NAMESPACE => base.map(|b| b.contains(&r.id)),
            ns if m.dependencies.require.iter().any(|d| d.name == ns) => continue,
            ns => {
                let message =
                    format!("{} {}: names {ns}'s content, but [dependencies] does not require {ns}", r.what, r.id);
                push(c, Severity::Error, &r.file, r.at, message);
                continue;
            }
        };
        if known == Some(false) {
            let message =
                format!("{} {}: the base game has no such id (`ssx-assets content ids` lists them)", r.what, r.id);
            push(c, Severity::Error, &r.file, r.at, message);
        }
    }
    let mut done = BTreeSet::new();
    for (f, def) in c.materials.clone().into_values() {
        if !done.insert(f.clone()) || def.family.is_some_and(|fam| !fam.is_game()) {
            continue;
        }
        let bytes = c.tree.as_ref().and_then(|t| t.read(&f, ssx_content::limits::MAX_RECORD_BYTES).ok());
        let text = bytes.and_then(|b| String::from_utf8(b).ok()).unwrap_or_default();
        check_parameters(c, big, &f, &text, &def);
    }
    let disc_overrides: Vec<ContentId> = m
        .overrides
        .keys()
        .filter(|id| id.namespace() == BASE_NAMESPACE && matches!(id.kind(), Kind::Material | Kind::Object))
        .cloned()
        .collect();
    if disc_overrides.is_empty() {
        return;
    }
    let records = match DiscModels::index(big) {
        Ok(models) => DiscRecords::new(big, std::sync::Arc::new(models)),
        Err(e) => return push(c, Severity::Error, "", (0, 0), format!("the disc's models cannot be indexed: {e:#}")),
    };
    for id in disc_overrides {
        let at = m.positions.entries.get(&id).copied().unwrap_or((0, 0));
        match id.kind() {
            Kind::Material => match ssx_content::Source::load(&records, &id).map(|a| a.into_value::<MaterialDef>()) {
                Ok(Some(d)) if d.family.is_none() => {
                    let message = format!(
                        "{id}: the disc's material is drawn with {}, which packages do not replace ({})",
                        d.effect,
                        unread_effect(&d.effect)
                    );
                    push(c, Severity::Error, MANIFEST_FILE, at, message);
                }
                Ok(_) => {}
                Err(e) => push(c, Severity::Error, MANIFEST_FILE, at, format!("{id}: the disc's record: {e}")),
            },
            _ => {
                let message = format!(
                    "{id}: drawn at the disc's placements; its collision, rigid body and controller are simulation \
                     content (a ride with this package is not the base game's)"
                );
                push(c, Severity::Note, MANIFEST_FILE, at, message);
            }
        }
    }
}
