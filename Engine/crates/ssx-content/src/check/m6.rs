use std::collections::{BTreeMap, BTreeSet};

use super::{Checked, Severity};
use crate::font;
use crate::id::{ContentId, Kind};
use crate::manifest::{MANIFEST_FILE, Manifest};
use crate::path::{EntryKind, PackageTree};
use crate::strings::{self, Lang, Table};
use crate::ui::{self, MovieInfo};

#[derive(Default)]
pub struct M6Files {
    pub movies: BTreeMap<ContentId, (String, MovieInfo)>,
    pub fonts: BTreeMap<ContentId, (String, font::FontInfo)>,
    pub tables: Vec<Table>,
}

pub(super) fn check_entry(
    c: &mut Checked,
    tree: &PackageTree,
    m: &Manifest,
    id: &ContentId,
    f: &str,
    at: (usize, usize),
) {
    match id.kind() {
        Kind::Ui => check_movie(c, tree, m, id, f, at),
        Kind::Font => check_font(c, tree, id, f, at),
        _ => {}
    }
}

fn check_movie(c: &mut Checked, tree: &PackageTree, m: &Manifest, id: &ContentId, f: &str, at: (usize, usize)) {
    let ext = f.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    if ext == "json" {
        let message = format!(
            "{id}: a layout of a generated view (JSON) is not read from packages: a screen is a movie (.swf / .gfx)"
        );
        c.push(Severity::Note, MANIFEST_FILE, at, message);
        return;
    }
    if !declared(m, f) {
        let message = format!(
            "{id}: {} is a movie the manifest does not declare in [actionscript] files",
            ssx_core::text::quoted(f)
        );
        c.push(Severity::Error, MANIFEST_FILE, at, message);
    }
    let caps = ssx_formats::gfx::Limits::default();
    let checked =
        tree.read(f, caps.max_file_bytes as u64).map_err(|e| e.to_string()).and_then(|b| ui::check_movie(&b, &caps));
    match checked {
        Ok(info) => {
            for (tag, n, why) in &info.unplayed {
                c.push(Severity::Warning, f, (0, 0), format!("{n} × {tag}: {why}"));
            }
            c.m6.movies.insert(id.clone(), (f.to_string(), info));
        }
        Err(e) => c.push(Severity::Error, MANIFEST_FILE, at, format!("{id}: {f}: {e}")),
    }
}

fn check_font(c: &mut Checked, tree: &PackageTree, id: &ContentId, f: &str, at: (usize, usize)) {
    let caps = font::Limits::default();
    let converted = tree
        .read(f, caps.max_file_bytes as u64)
        .map_err(|e| e.to_string())
        .and_then(|b| font::convert(&b, id.as_str(), &caps));
    match converted {
        Ok(cv) => {
            if cv.info.clamped_advances > 0 {
                let message = format!(
                    "{id}: {} glyphs are wider than the player's advance holds (1.6 EM): clamped",
                    cv.info.clamped_advances
                );
                c.push(Severity::Warning, f, (0, 0), message);
            }
            if let Some(why) = &cv.info.kerning_dropped {
                c.push(Severity::Warning, f, (0, 0), format!("{id}: its kerning is left out: {why}"));
            }
            c.m6.fonts.insert(id.clone(), (f.to_string(), cv.info));
        }
        Err(e) => c.push(Severity::Error, MANIFEST_FILE, at, format!("{id}: {f}: {e}")),
    }
}

fn declared(m: &Manifest, f: &str) -> bool {
    let mut budget = crate::glob::Budget(super::PATTERN_BUDGET);
    m.actionscript
        .files
        .iter()
        .filter_map(|p| crate::glob::Pattern::parse(p).ok())
        .any(|p| p.matches(f, &mut budget).unwrap_or(false))
}

pub(super) fn check_package(c: &mut Checked, tree: &PackageTree, m: &Manifest, named: &mut BTreeSet<String>) {
    let movies: Vec<String> = tree
        .entries()
        .filter(|(p, k)| {
            *k == EntryKind::File
                && p.rsplit_once('.')
                    .is_some_and(|(_, e)| crate::package::MOVIE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        })
        .map(|(p, _)| p.to_string())
        .collect();
    for f in movies {
        if declared(m, &f) {
            named.insert(f);
        } else {
            let message = "a movie the manifest does not declare: list it in [actionscript] files (a movie is a program \
                           the player runs; the Mods page says \"contains ActionScript\", servers refuse such packages)";
            c.push(Severity::Error, &f, (0, 0), message);
        }
    }
    for id in m.content_ids().filter(|id| id.kind() == Kind::String) {
        let at = m.positions.entries.get(id).copied().unwrap_or((0, 0));
        let message = format!("{id}: strings come from the tables `strings/<language>.toml`, by key, not from entries");
        c.push(Severity::Error, MANIFEST_FILE, at, message);
    }
    let tables: Vec<String> = tree
        .entries()
        .filter(|(p, k)| *k == EntryKind::File && strings::table_lang(p).is_some())
        .map(|(p, _)| p.to_string())
        .collect();
    let ns = &m.resource.name;
    let mut seen: BTreeMap<Lang, String> = BTreeMap::new();
    for f in tables {
        named.insert(f.clone());
        let lang = match strings::table_lang(&f) {
            Some(Ok(l)) => l,
            Some(Err(e)) => {
                c.push(Severity::Error, &f, (0, 0), e);
                continue;
            }
            None => continue,
        };
        if let Some(other) = seen.insert(lang, f.clone()) {
            c.push(Severity::Error, &f, (0, 0), format!("a second table of {} beside {other}", lang.code()));
            continue;
        }
        let parsed = tree
            .read(&f, crate::limits::MAX_RECORD_BYTES)
            .map_err(|e| vec![(0, 0, e.to_string())])
            .and_then(|b| strings::parse_table(&b, &f, lang, ns));
        match parsed {
            Ok(t) => {
                for (id, e) in &t.entries {
                    if id.namespace() == ns.as_str()
                        && let Err(why) = strings::check_against(&e.text, None)
                    {
                        c.push(Severity::Error, &f, e.at, format!("{}: {why}", ssx_core::text::quoted(&e.key)));
                    }
                }
                c.m6.tables.push(t);
            }
            Err(ps) => {
                for (l, col, message) in ps {
                    c.push(Severity::Error, &f, (l, col), message);
                }
            }
        }
    }
    let has_en = c.m6.tables.iter().any(|t| t.lang == Lang::En);
    let missing_en = c.m6.tables.iter().find(|t| t.lang != Lang::En).and_then(|t| {
        let added = t.entries.keys().find(|id| id.namespace() == ns.as_str())?;
        Some((t.file.clone(), added.clone()))
    });
    if let (false, Some((file, added))) = (has_en, missing_en) {
        let message = format!(
            "strings the package adds have no English table: a language without the key shows {added} as its id"
        );
        c.push(Severity::Warning, &file, (0, 0), message);
    }
}
