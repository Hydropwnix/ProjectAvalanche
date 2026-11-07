use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Result, bail};
use ssx_content::data::{FieldSchema, RecordSchema};
use ssx_content::{ContentId, Kind};

use super::events::DiscEvents;
use super::tuning::DiscTuning;

const MAX_COMMENT: usize = 160;

pub fn files(big_root: &Path, id: &ContentId) -> Result<Vec<(String, Vec<u8>)>> {
    let (schema, table) = match id.kind() {
        Kind::Tuning | Kind::Scoring => {
            let disc = DiscTuning::load(big_root)?;
            let Some(schema) = disc.schema_of(id) else { bail!("{id}: the disc has no such record") };
            (schema, disc.record(id).map_err(anyhow::Error::msg)?)
        }
        Kind::Event | Kind::Drop => {
            let disc = DiscEvents::load(big_root)?;
            let Some(schema) = disc.schema_of(id) else { bail!("{id}: the disc has no such record") };
            (schema, disc.record(id).map_err(anyhow::Error::msg)?)
        }
        k => bail!("{id}: {} is not a data record", k.as_str()),
    };
    Ok(vec![(format!("{}/{}.toml", id.kind().as_str(), id.path()), text(id, &schema, &table)?.into_bytes())])
}

pub fn text(id: &ContentId, schema: &RecordSchema, table: &toml::Table) -> Result<String> {
    let mut out = String::new();
    writeln!(out, "# {id}: the disc's record, exported for your own use (Docs/architecture/content.md §21).")?;
    writeln!(out, "# Override it whole under [override], or name only the fields you change in a file under [patch].")?;
    writeln!(out, "# A field read by the simulation changes the ride (and the session's sim content id).")?;
    let (plain, tables): (Vec<_>, Vec<_>) = table.iter().partition(|(_, v)| !v.is_table());
    for (key, value) in plain {
        out.push('\n');
        if let Some(f) = schema.fields.get(key) {
            comment(&mut out, "", f);
        }
        let mut one = toml::Table::new();
        one.insert(key.clone(), value.clone());
        out.push_str(&toml::to_string(&one)?);
    }
    for (key, value) in tables {
        out.push('\n');
        if let Some(f) = schema.fields.get(key) {
            comment(&mut out, "", f);
            if let ssx_content::data::FieldType::Table(t) = &f.ty {
                for (path, g) in t.flat() {
                    comment(&mut out, &format!("{path}: "), g);
                }
            }
        }
        let mut one = toml::Table::new();
        one.insert(key.clone(), value.clone());
        out.push_str(&toml::to_string(&one)?);
    }
    Ok(out)
}

fn comment(out: &mut String, prefix: &str, f: &FieldSchema) {
    let mut line = format!("{prefix}{} · {}", f.ty, f.class.as_str());
    if f.optional {
        line.push_str(" · optional");
    }
    if !f.note.is_empty() {
        line.push_str(" · ");
        line.push_str(&f.note);
    }
    if !f.source.is_empty() {
        line.push_str(" · range: ");
        line.push_str(&f.source);
    }
    let line: String = line.chars().filter(|c| !c.is_control()).collect();
    if line.chars().count() > MAX_COMMENT {
        let cut: String = line.chars().take(MAX_COMMENT).collect();
        let _ = writeln!(out, "# {cut} …");
    } else {
        let _ = writeln!(out, "# {line}");
    }
}
