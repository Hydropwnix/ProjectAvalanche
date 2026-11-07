use std::path::Path;

use ssx_content::check::{Checked, Severity};
use ssx_content::data::{Mode, Problem, RecordSchema, check_table_with, check_text};
use ssx_content::limits::MAX_RECORD_BYTES;
use ssx_content::manifest::{MANIFEST_FILE, Manifest};
use ssx_content::path::PackageTree;
use ssx_content::{ContentId, Kind};

use super::check::push;
use crate::content::data::DATA_KINDS as KINDS;
use crate::content::data::events::DiscEvents;
use crate::content::data::tuning::DiscTuning;

struct Disc {
    tuning: Option<DiscTuning>,
    events: Option<DiscEvents>,
}

impl Disc {
    fn schema(&self, id: &ContentId) -> Option<RecordSchema> {
        match id.kind() {
            Kind::Tuning | Kind::Scoring => self.tuning.as_ref()?.schema_of(id),
            _ => self.events.as_ref()?.schema_of(id),
        }
    }

    fn record(&self, id: &ContentId) -> Option<toml::Table> {
        match id.kind() {
            Kind::Tuning | Kind::Scoring => self.tuning.as_ref()?.record(id).ok(),
            _ => self.events.as_ref()?.record(id).ok(),
        }
    }

    fn check(&self, schema: &RecordSchema, bytes: &[u8], mode: Mode) -> Result<toml::Table, Vec<Problem>> {
        match &self.tuning {
            Some(t) => check_text(schema, bytes, mode, Some(&|g: &[u8; 16]| t.guid_class(g))),
            None => check_text(schema, bytes, mode, None),
        }
    }

    fn check_whole(&self, schema: &RecordSchema, table: &toml::Table) -> Vec<Problem> {
        let spans = ssx_content::data::spans::Spans::default();
        match &self.tuning {
            Some(t) => check_table_with(schema, table, &spans, Mode::Whole, Some(&|g: &[u8; 16]| t.guid_class(g))),
            None => check_table_with(schema, table, &spans, Mode::Whole, None),
        }
    }
}

pub(super) fn check(c: &mut Checked, m: &Manifest, dir: &Path, big: &Path) {
    let ours = |id: &ContentId| KINDS.contains(&id.kind());
    for (id, _) in m.add.iter().filter(|(id, _)| ours(id)) {
        let at = m.positions.entries.get(id).copied().unwrap_or((0, 0));
        let message = format!(
            "{id}: {} records are changed, not added (the game's lists, saves and rules know the disc's ids only); \
             override or patch a disc record",
            id.kind()
        );
        push(c, Severity::Error, MANIFEST_FILE, at, message);
    }
    let overrides: Vec<(&ContentId, &str)> = m
        .overrides
        .iter()
        .filter(|(id, _)| ours(id))
        .filter_map(|(id, e)| e.files().first().map(|f| (id, *f)))
        .collect();
    let patches: Vec<(&ContentId, &str)> =
        m.patches.iter().filter(|(id, _)| ours(id)).map(|(id, f)| (id, f.as_str())).collect();
    if overrides.is_empty() && patches.is_empty() {
        return;
    }
    let tree = match PackageTree::open(dir) {
        Ok(t) => t,
        Err(e) => return push(c, Severity::Error, MANIFEST_FILE, (0, 0), e.to_string()),
    };
    let wants = |kinds: &[Kind]| overrides.iter().chain(&patches).any(|(id, _)| kinds.contains(&id.kind()));
    let load_note = |c: &mut Checked, what: &str, e: anyhow::Error| {
        push(
            c,
            Severity::Error,
            MANIFEST_FILE,
            (0, 0),
            format!("the disc's {what} cannot be read ({e:#}): not checked"),
        )
    };
    let tuning = if wants(&[Kind::Tuning, Kind::Scoring]) {
        DiscTuning::load(big).map_err(|e| load_note(c, "tuning", e)).ok()
    } else {
        None
    };
    let events = if wants(&[Kind::Event, Kind::Drop]) {
        DiscEvents::load(big).map_err(|e| load_note(c, "events", e)).ok()
    } else {
        None
    };
    let disc = Disc { tuning, events };
    let mut results: std::collections::BTreeMap<ContentId, toml::Table> = std::collections::BTreeMap::new();
    for (mode, list) in [(Mode::Whole, &overrides), (Mode::Patch, &patches)] {
        for (id, file) in list {
            let at = match mode {
                Mode::Whole => m.positions.entries.get(*id),
                Mode::Patch => m.positions.patches.get(*id),
            }
            .copied()
            .unwrap_or((0, 0));
            if let Some(t) = entry(c, &tree, &disc, id, file, at, mode) {
                let merged = match (mode, results.get(*id)) {
                    (Mode::Patch, Some(over)) => ssx_content::patch::apply(over, [&t]),
                    (Mode::Patch, None) => match disc.record(id) {
                        Some(base) => ssx_content::patch::apply(&base, [&t]),
                        None => t,
                    },
                    _ => t,
                };
                results.insert((*id).clone(), merged);
            }
        }
    }
    linked(c, m, &disc, &results);
}

fn linked(c: &mut Checked, m: &Manifest, disc: &Disc, results: &std::collections::BTreeMap<ContentId, toml::Table>) {
    for &(a, b, source) in ssx_content::data::ant_schema::LINKED_FLOATS {
        let (Ok(a), Ok(b)) = (a.parse::<ContentId>(), b.parse::<ContentId>()) else { continue };
        if !results.contains_key(&a) && !results.contains_key(&b) {
            continue;
        }
        let table = |id: &ContentId| results.get(id).cloned().or_else(|| disc.record(id));
        let (Some(ta), Some(tb)) = (table(&a), table(&b)) else { continue };
        if let Some(d) = ssx_content::data::ant_schema::floats_differ(&ta, &tb) {
            let id = if results.contains_key(&a) { &a } else { &b };
            let at = m.positions.entries.get(id).or_else(|| m.positions.patches.get(id)).copied().unwrap_or((0, 0));
            push(c, Severity::Error, MANIFEST_FILE, at, format!("{a} and {b} differ ({d}): {source}"));
        }
    }
}

fn entry(
    c: &mut Checked,
    tree: &PackageTree,
    disc: &Disc,
    id: &ContentId,
    file: &str,
    at: (usize, usize),
    mode: Mode,
) -> Option<toml::Table> {
    if id.namespace() != ssx_content::BASE_NAMESPACE {
        let message = format!("{id}: only the disc's {} records are changed (namespace `ssx`)", id.kind());
        push(c, Severity::Error, MANIFEST_FILE, at, message);
        return None;
    }
    let Some(schema) = disc.schema(id) else {
        let loaded = match id.kind() {
            Kind::Tuning | Kind::Scoring => disc.tuning.is_some(),
            _ => disc.events.is_some(),
        };
        if loaded {
            let message =
                format!("{id}: the disc has no such record (`ssx-assets content ids --kind {}` lists them)", id.kind());
            push(c, Severity::Error, MANIFEST_FILE, at, message);
        }
        return None;
    };
    if c.findings.iter().any(|x| x.file == file && x.severity == Severity::Error) {
        return None;
    }
    let bytes = match tree.read(file, MAX_RECORD_BYTES) {
        Ok(b) => b,
        Err(e) => {
            push(c, Severity::Error, file, (0, 0), e.to_string());
            return None;
        }
    };
    let table = match disc.check(&schema, &bytes, mode) {
        Ok(t) => t,
        Err(problems) => {
            for p in problems {
                push(c, Severity::Error, file, (p.line, p.column), described(&p));
            }
            return None;
        }
    };
    if mode == Mode::Patch
        && let Some(base) = disc.record(id)
    {
        let merged = ssx_content::patch::apply(&base, [&table]);
        for p in disc.check_whole(&schema, &merged) {
            let message = format!("merged onto the disc's record: {}", described(&p));
            push(c, Severity::Error, file, (0, 0), message);
        }
    }
    Some(table)
}

fn described(p: &Problem) -> String {
    if p.path.is_empty() {
        p.message.clone()
    } else {
        format!("{}: {}", ssx_core::text::shown_max(&p.path, ssx_core::text::PATH_CHARS), p.message)
    }
}

pub(super) fn check_earlier(c: &mut Checked, m: &Manifest, dir: &Path, big: &Path) {
    const EARLIER: [Kind; 6] = [Kind::Character, Kind::Gear, Kind::Board, Kind::Marker, Kind::Lighting, Kind::Postfx];
    let mut files: Vec<(&ContentId, &str)> = Vec::new();
    for (id, e) in m.add.iter().chain(&m.overrides).filter(|(id, _)| EARLIER.contains(&id.kind())) {
        files.extend(e.files().into_iter().map(|f| (id, f)));
    }
    files.extend(m.patches.iter().filter(|(id, _)| EARLIER.contains(&id.kind())).map(|(id, f)| (id, f.as_str())));
    files.retain(|(_, f)| f.to_ascii_lowercase().ends_with(".toml"));
    files.retain(|(_, f)| !c.findings.iter().any(|x| x.file == *f && x.severity == Severity::Error));
    if files.is_empty() {
        return;
    }
    let Ok(tree) = PackageTree::open(dir) else { return };
    let mut schemas: std::collections::BTreeMap<Kind, Option<RecordSchema>> = std::collections::BTreeMap::new();
    for (id, file) in files {
        let schema = schemas.entry(id.kind()).or_insert_with(|| earlier_schema(c, id.kind(), big));
        let Some(schema) = schema.as_ref() else { continue };
        let Ok(bytes) = tree.read(file, MAX_RECORD_BYTES) else { continue };
        if let Err(problems) = check_text(schema, &bytes, Mode::Patch, None) {
            for p in problems {
                let message = format!("{id}: {} (its kind's schema)", described(&p));
                push(c, Severity::Error, file, (p.line, p.column), message);
            }
        }
    }
}

fn earlier_schema(c: &mut Checked, kind: Kind, big: &Path) -> Option<RecordSchema> {
    let made = match kind {
        Kind::Character => Ok(ssx_content::data::kinds::character()),
        Kind::Gear => Ok(ssx_content::data::kinds::item(false)),
        Kind::Board => Ok(ssx_content::data::kinds::item(true)),
        Kind::Marker => Ok(ssx_content::data::kinds::marker()),
        Kind::Lighting => {
            crate::content::data::kinds::lighting(&crate::content::GameContent::of(big)).map_err(anyhow::Error::msg)
        }
        _ => crate::content::data::kinds::postfx(big),
    };
    made.map_err(|e| {
        let message = format!("the disc's {kind} fields cannot be read ({e:#}): the {kind} schema is not checked");
        push(c, Severity::Warning, MANIFEST_FILE, (0, 0), message);
    })
    .ok()
}
