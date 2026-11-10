use std::path::Path;

use ssx_content::check::{Checked, Severity};
use ssx_content::id::{BASE_NAMESPACE, PORT_NAMESPACE};
use ssx_content::manifest::{MANIFEST_FILE, Manifest};
use ssx_content::strings::{self, Lang};
use ssx_content::{ContentId, Kind};

use super::check::push;
use crate::content::m6::{DiscStrings, PortStrings};

const PORT_VIEWS: [&str; 8] =
    ["ava_hub", "ava_keyboard", "ava_maps", "ava_options", "ava_browser", "ava_players", "ava_chat", "ava_quickchat"];

pub fn is_port_id(id: &ContentId) -> bool {
    match id.kind() {
        Kind::Ui => id.path().strip_prefix("views/").is_some_and(|v| PORT_VIEWS.contains(&v)),
        Kind::String => PortStrings::parse().is_ok_and(|p| p.texts.contains_key(id)),
        _ => false,
    }
}

pub(super) fn check(c: &mut Checked, m: &Manifest, big_root: &Path) {
    let tables = std::mem::take(&mut c.m6.tables);
    let base_keys = tables.iter().any(|t| t.entries.keys().any(|id| id.namespace() != m.resource.name));
    let disc = if base_keys { DiscStrings::index(big_root).map_err(|e| format!("{e:#}")) } else { Err(String::new()) };
    let port = PortStrings::parse().map_err(|e| format!("{e:#}"));
    if let (true, Err(e)) = (base_keys, &disc) {
        push(
            c,
            Severity::Error,
            "",
            (0, 0),
            format!("the disc's strings cannot be read ({e}): base keys are not checked"),
        );
    }
    for t in &tables {
        for (id, e) in &t.entries {
            let base = match id.namespace() {
                BASE_NAMESPACE => match &disc {
                    Ok(d) => match d.texts.get(id) {
                        Some((_, by_lang)) => by_lang.get(&t.lang).cloned().or_else(|| by_lang.get(&Lang::En).cloned()),
                        None => {
                            let message = format!("{:?}: the game has no string of this key", e.key);
                            push(c, Severity::Error, &t.file, e.at, message);
                            continue;
                        }
                    },
                    Err(_) => continue,
                },
                PORT_NAMESPACE => match port.as_ref().ok().and_then(|p| p.texts.get(id)) {
                    Some((_, text)) => Some(text.clone()),
                    None => {
                        let message = format!("{:?}: the port has no string of this key", e.key);
                        push(c, Severity::Error, &t.file, e.at, message);
                        continue;
                    }
                },
                _ => continue,
            };
            if let Err(why) = strings::check_against(&e.text, base.as_deref()) {
                push(c, Severity::Error, &t.file, e.at, format!("{:?}: {why}", e.key));
            }
        }
    }
    c.m6.tables = tables;
    for (id, (file, info)) in &c.m6.movies.clone() {
        let at = m.positions.entries.get(id).copied().unwrap_or((0, 0));
        let calls = match id.namespace() {
            BASE_NAMESPACE => {
                "it keeps the game's actions of the screen it replaces; it raises the nav events the \
                               disc screen's code names"
            }
            _ => "an added movie (or a port view's replacement) makes the read-only calls: data, input, strings",
        };
        let message = format!(
            "{id}: {file}: {} {} of SWF {}, {} action blocks ({} actions); {calls}",
            info.container.map_or("?", |c| c.name()),
            if info.has_actionscript() { "movie with ActionScript" } else { "movie without ActionScript" },
            info.version,
            info.blocks,
            info.actions,
        );
        push(c, Severity::Note, MANIFEST_FILE, at, message);
    }
    for (id, (file, info)) in &c.m6.fonts.clone() {
        let at = m.positions.entries.get(id).copied().unwrap_or((0, 0));
        let message = format!(
            "{id}: {file}: \"{}\", {} outlines, {} characters ({} glyphs), {} kerning pairs ({})",
            info.family,
            info.outlines,
            info.codes,
            info.mapped_glyphs,
            info.kerning_pairs,
            if info.kerning_dropped.is_some() { "left out" } else { info.kerning_source }
        );
        push(c, Severity::Note, MANIFEST_FILE, at, message);
    }
}

#[cfg(test)]
mod tests {
    use super::PORT_VIEWS;

    #[test]
    fn the_port_views_are_the_built_views() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fe/views");
        let mut built: Vec<String> = std::fs::read_dir(&dir)
            .expect("the port's views")
            .map(|e| e.expect("a view").file_name().into_string().expect("a UTF-8 name"))
            .map(|name| name.strip_suffix(".json").map(str::to_string).unwrap_or_else(|| panic!("{name}: not a view")))
            .collect();
        built.sort();
        let mut listed: Vec<String> = PORT_VIEWS.iter().map(|v| v.to_string()).collect();
        listed.sort();
        assert_eq!(listed, built);
    }
}
