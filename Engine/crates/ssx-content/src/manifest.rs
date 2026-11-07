use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::ops::Range;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use ssx_core::text::{Text, shown_max};
use toml::Spanned;

use crate::id::{BASE_NAMESPACE, ContentId, Kind, PORT_NAMESPACE, valid_namespace};
use crate::limits::{
    MAX_ENTRIES, MAX_LIST, MAX_LONG_TEXT, MAX_MANIFEST_BYTES, MAX_SHORT_TEXT, MAX_TOML_DEPTH, table_depth,
};
use crate::path::check_package_path;
use crate::version::{Dependency, Version};

pub const MANIFEST_FILE: &str = "resource.toml";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PackageType {
    #[default]
    Mod,
    Map,
    Gamemode,
    Script,
    Library,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Resource {
    pub name: String,
    pub version: Version,
    pub title: String,
    pub authors: Vec<String>,
    pub description: String,
    pub api: u32,
    pub kind: PackageType,
    pub priority: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    File(String),
    Files(BTreeMap<String, String>),
}

impl Entry {
    pub fn files(&self) -> Vec<&str> {
        match self {
            Entry::File(f) => vec![f.as_str()],
            Entry::Files(m) => m.values().map(String::as_str).collect(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scripts {
    #[serde(default)]
    pub client: Vec<String>,
    #[serde(default)]
    pub server: Vec<String>,
    #[serde(default)]
    pub shared: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Files {
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(default)]
    pub lazy: Vec<String>,
    #[serde(default)]
    pub server_only: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shaders {
    #[serde(default)]
    pub files: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionScript {
    #[serde(default)]
    pub files: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dependencies {
    pub require: Vec<Dependency>,
    pub provide: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Permissions {
    #[serde(default)]
    pub request: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    pub resource: Resource,
    pub add: BTreeMap<ContentId, Entry>,
    pub overrides: BTreeMap<ContentId, Entry>,
    pub patches: BTreeMap<ContentId, String>,
    pub scripts: Scripts,
    pub files: Files,
    pub shaders: Shaders,
    pub actionscript: ActionScript,
    pub dependencies: Dependencies,
    pub permissions: Permissions,
    pub settings: toml::Table,
    pub positions: Positions,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Positions {
    pub resource: (usize, usize),
    pub entries: BTreeMap<ContentId, (usize, usize)>,
    pub patches: BTreeMap<ContentId, (usize, usize)>,
    pub requires: BTreeMap<String, (usize, usize)>,
    pub scripts: (usize, usize),
    pub files: (usize, usize),
    pub shaders: (usize, usize),
    pub actionscript: (usize, usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub line: usize,
    pub column: usize,
    pub message: Text,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub struct ManifestError {
    pub file: PathBuf,
    pub diagnostics: Vec<Diagnostic>,
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let path = self.file.to_string_lossy();
        let file = shown_max(&path, ssx_core::text::PATH_CHARS);
        for (i, d) in self.diagnostics.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            f.write_str(&Text::new(format_args!("{file}:{}:{}: {}", d.line, d.column, d.message)))?;
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawResource {
    name: Spanned<String>,
    version: Spanned<String>,
    #[serde(default)]
    title: String,
    #[serde(default)]
    authors: Vec<String>,
    #[serde(default)]
    description: String,
    api: Spanned<u32>,
    #[serde(rename = "type", default)]
    kind: PackageType,
    #[serde(default)]
    priority: i32,
}

type RawEntries = BTreeMap<Spanned<String>, Spanned<toml::Value>>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDependencies {
    #[serde(default)]
    require: Vec<Spanned<String>>,
    #[serde(default)]
    provide: Vec<Spanned<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    resource: Spanned<RawResource>,
    #[serde(default)]
    add: RawEntries,
    #[serde(default, rename = "override")]
    overrides: RawEntries,
    #[serde(default)]
    patch: BTreeMap<Spanned<String>, Spanned<String>>,
    scripts: Option<Spanned<Scripts>>,
    files: Option<Spanned<Files>>,
    shaders: Option<Spanned<Shaders>>,
    actionscript: Option<Spanned<ActionScript>>,
    dependencies: Option<RawDependencies>,
    permissions: Option<Spanned<Permissions>>,
    settings: Option<Spanned<toml::Table>>,
}

pub fn line_col(text: &str, at: usize) -> (usize, usize) {
    let at = at.min(text.len());
    let before = &text[..text.floor_char_boundary(at)];
    let line = before.matches('\n').count() + 1;
    let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (line, col)
}

fn extension(p: &str) -> String {
    Path::new(p).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

struct Checker<'a> {
    text: &'a str,
    diags: Vec<Diagnostic>,
    refs: Vec<(String, Range<usize>)>,
}

impl Checker<'_> {
    fn error(&mut self, span: Range<usize>, message: impl Into<Text>) {
        let (line, column) = line_col(self.text, span.start);
        self.diags.push(Diagnostic { line, column, message: message.into() });
    }

    fn entry(&mut self, id: &ContentId, value: &Spanned<toml::Value>) -> Option<Entry> {
        let span = value.span();
        let entry = match value.get_ref() {
            toml::Value::String(s) => Entry::File(s.clone()),
            toml::Value::Table(t) if !t.is_empty() => {
                let mut files = BTreeMap::new();
                if t.len() > MAX_LIST {
                    self.error(span, format!("{id}: more than {MAX_LIST} files"));
                    return None;
                }
                for (role, v) in t {
                    if role.is_empty() || role.len() > 32 || !role.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
                        self.error(
                            span.clone(),
                            format!("{id}: bad role name {} (lower case `[a-z_]`)", ssx_core::text::quoted(role)),
                        );
                        return None;
                    }
                    let Some(s) = v.as_str() else {
                        self.error(span.clone(), format!("{id}: `{role}` must be a file path"));
                        return None;
                    };
                    files.insert(role.clone(), s.to_string());
                }
                Entry::Files(files)
            }
            _ => {
                self.error(span, format!("{id}: expected a file path or a table of file paths"));
                return None;
            }
        };
        let formats = id.kind().formats();
        if formats.is_empty() {
            self.error(span.clone(), format!("{id}: the kind `{}` is not moddable yet", id.kind()));
            return None;
        }
        for f in entry.files() {
            self.refs.push((f.to_string(), span.clone()));
            if let Err(e) = check_package_path(f, false) {
                self.error(span.clone(), format!("{id}: {e}"));
            } else if !formats.contains(&extension(f).as_str()) {
                let list = formats.iter().map(|e| format!(".{e}")).collect::<Vec<_>>().join(" ");
                self.error(
                    span.clone(),
                    format!("{id}: {} is not a {} file ({list})", ssx_core::text::quoted(f), id.kind()),
                );
            }
        }
        Some(entry)
    }

    fn globs(&mut self, span: Range<usize>, section: &str, list: &[String], ext: Option<&str>) {
        if list.len() > MAX_LIST {
            self.error(span.clone(), format!("[{section}]: more than {MAX_LIST} patterns"));
            return;
        }
        for g in list {
            if let Err(e) = check_package_path(g, true) {
                self.error(span.clone(), format!("[{section}]: {e}"));
            } else if let Some(ext) = ext
                && !g.ends_with(ext)
            {
                self.error(span.clone(), format!("[{section}]: {} must name `{ext}` files", ssx_core::text::quoted(g)));
            }
        }
    }
}

fn one_line(t: &str, max: usize) -> bool {
    t.len() <= max && !t.chars().any(char::is_control)
}

fn text_block(t: &str, max: usize) -> bool {
    t.len() <= max && !t.chars().any(|ch| ch.is_control() && ch != '\n' && ch != '\t')
}

fn rspan_of(a: &[Spanned<String>], b: &[Spanned<String>]) -> Range<usize> {
    a.first().or_else(|| b.first()).map_or(0..0, Spanned::span)
}

fn valid_permission(p: &str) -> bool {
    !p.is_empty() && p.len() <= 128 && p.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

impl Manifest {
    pub fn parse_bytes(bytes: &[u8], file: &Path) -> Result<Self, ManifestError> {
        let fail = |message: String| ManifestError {
            file: file.to_path_buf(),
            diagnostics: vec![Diagnostic { line: 0, column: 0, message: message.into() }],
        };
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(fail(format!("over {MAX_MANIFEST_BYTES} bytes")));
        }
        let text = std::str::from_utf8(bytes).map_err(|e| fail(format!("not UTF-8: {e}")))?;
        Self::parse(text, file)
    }

    pub fn parse(text: &str, file: &Path) -> Result<Self, ManifestError> {
        let fail = |diagnostics| ManifestError { file: file.to_path_buf(), diagnostics };
        if text.len() > MAX_MANIFEST_BYTES {
            let message = format!("over {MAX_MANIFEST_BYTES} bytes");
            return Err(fail(vec![Diagnostic { line: 0, column: 0, message: message.into() }]));
        }
        let raw: RawManifest = toml::from_str(text).map_err(|e| {
            let (line, column) = e.span().map_or((0, 0), |s| line_col(text, s.start));
            fail(vec![Diagnostic { line, column, message: ssx_core::text::message(e.message()) }])
        })?;
        let mut c = Checker { text, diags: Vec::new(), refs: Vec::new() };
        let at = |span: Range<usize>| line_col(text, span.start);
        let mut positions = Positions { resource: at(raw.resource.span()), ..Positions::default() };
        let r = raw.resource.get_ref();
        let name = r.name.get_ref().clone();
        if !valid_namespace(&name) {
            c.error(
                r.name.span(),
                format!("bad package name {} (lower case `[a-z0-9_.-]`)", ssx_core::text::quoted(&name)),
            );
        } else if name == BASE_NAMESPACE || name == PORT_NAMESPACE {
            c.error(r.name.span(), format!("the package name {} is reserved", ssx_core::text::quoted(&name)));
        }
        let version = r.version.get_ref().parse::<Version>().unwrap_or_else(|e| {
            c.error(r.version.span(), e.to_string());
            Version::new(0, 0, 0)
        });
        if *r.api.get_ref() == 0 {
            c.error(r.api.span(), "api levels start at 1");
        }
        let rspan = raw.resource.span();
        if !one_line(&r.title, MAX_SHORT_TEXT) {
            c.error(rspan.clone(), format!("title: at most {MAX_SHORT_TEXT} bytes on one line"));
        }
        if !text_block(&r.description, MAX_LONG_TEXT) {
            c.error(rspan.clone(), format!("description: at most {MAX_LONG_TEXT} bytes, no control characters"));
        }
        if r.authors.len() > MAX_LIST || r.authors.iter().any(|a| a.is_empty() || !one_line(a, MAX_SHORT_TEXT)) {
            c.error(rspan, format!("authors: at most {MAX_LIST} names of 1 to {MAX_SHORT_TEXT} bytes on one line"));
        }
        for (section, n, span) in [
            ("add", raw.add.len(), raw.add.keys().next().map(Spanned::span)),
            ("override", raw.overrides.len(), raw.overrides.keys().next().map(Spanned::span)),
            ("patch", raw.patch.len(), raw.patch.keys().next().map(Spanned::span)),
        ] {
            if n > MAX_ENTRIES {
                c.error(span.unwrap_or(0..0), format!("[{section}]: {n} entries (at most {MAX_ENTRIES})"));
            }
        }
        let mut add = BTreeMap::new();
        for (key, value) in &raw.add {
            match ContentId::parse(&format!("{name}:{}", key.get_ref())) {
                Ok(id) => {
                    if let Some(e) = c.entry(&id, value) {
                        positions.entries.insert(id.clone(), at(key.span()));
                        add.insert(id, e);
                    }
                }
                Err(e) => c.error(
                    key.span(),
                    format!("[add] {}: {e} (keys are `<kind>/<path>`)", ssx_core::text::quoted(key.get_ref())),
                ),
            }
        }
        let mut overrides = BTreeMap::new();
        for (key, value) in &raw.overrides {
            match ContentId::parse(key.get_ref()) {
                Ok(id) if id.namespace() == name => {
                    c.error(key.span(), format!("[override] {id}: the package's own content goes in [add]"))
                }
                Ok(id) => {
                    if let Some(e) = c.entry(&id, value) {
                        positions.entries.insert(id.clone(), at(key.span()));
                        overrides.insert(id, e);
                    }
                }
                Err(e) => c.error(key.span(), format!("[override] {}: {e}", ssx_core::text::quoted(key.get_ref()))),
            }
        }
        let mut patches = BTreeMap::new();
        for (key, file) in &raw.patch {
            let id = match ContentId::parse(key.get_ref()) {
                Ok(id) if !id.kind().patchable() => {
                    c.error(key.span(), format!("[patch] {id}: `{}` records cannot be patched", id.kind()));
                    continue;
                }
                Ok(id) => id,
                Err(e) => {
                    c.error(key.span(), format!("[patch] {}: {e}", ssx_core::text::quoted(key.get_ref())));
                    continue;
                }
            };
            let f = file.get_ref();
            c.refs.push((f.clone(), file.span()));
            if let Err(e) = check_package_path(f, false) {
                c.error(file.span(), format!("[patch] {id}: {e}"));
            } else if extension(f) != "toml" {
                c.error(file.span(), format!("[patch] {id}: a patch is a .toml file"));
            }
            positions.patches.insert(id.clone(), at(key.span()));
            patches.insert(id, f.clone());
        }
        let (scripts, span) = raw.scripts.map_or_else(Default::default, |s| (s.get_ref().clone(), s.span()));
        positions.scripts = if span == (0..0) { (0, 0) } else { at(span.clone()) };
        let s = &scripts;
        for (section, list) in
            [("scripts.client", &s.client), ("scripts.server", &s.server), ("scripts.shared", &s.shared)]
        {
            c.globs(span.clone(), section, list, Some(".luau"));
        }
        let (files, span) = raw.files.map_or_else(Default::default, |f| (f.get_ref().clone(), f.span()));
        positions.files = if span == (0..0) { (0, 0) } else { at(span.clone()) };
        let f = &files;
        for (section, list) in
            [("files.include", &f.include), ("files.lazy", &f.lazy), ("files.server_only", &f.server_only)]
        {
            c.globs(span.clone(), section, list, None);
        }
        let (shaders, span) = raw.shaders.map_or_else(Default::default, |s| (s.get_ref().clone(), s.span()));
        positions.shaders = if span == (0..0) { (0, 0) } else { at(span.clone()) };
        c.globs(span, "shaders.files", &shaders.files, Some(".wgsl"));
        let (actionscript, span) = raw.actionscript.map_or_else(Default::default, |s| (s.get_ref().clone(), s.span()));
        positions.actionscript = if span == (0..0) { (0, 0) } else { at(span.clone()) };
        c.globs(span, "actionscript.files", &actionscript.files, None);
        let mut dependencies = Dependencies::default();
        if let Some(d) = &raw.dependencies {
            for (list, n) in [("require", d.require.len()), ("provide", d.provide.len())] {
                if n > MAX_LIST {
                    c.error(rspan_of(&d.require, &d.provide), format!("[dependencies] {list}: more than {MAX_LIST}"));
                }
            }
            let mut seen = BTreeSet::new();
            for req in &d.require {
                match req.get_ref().parse::<Dependency>() {
                    Ok(dep) if dep.name == name => c.error(req.span(), "a package cannot require itself"),
                    Ok(dep) if !seen.insert(dep.name.clone()) => {
                        c.error(req.span(), format!("{} is required twice", dep.name))
                    }
                    Ok(dep) => {
                        positions.requires.insert(dep.name.clone(), at(req.span()));
                        dependencies.require.push(dep)
                    }
                    Err(e) => c.error(req.span(), e.to_string()),
                }
            }
            for p in &d.provide {
                if valid_namespace(p.get_ref()) {
                    dependencies.provide.push(p.get_ref().clone());
                } else {
                    c.error(p.span(), format!("bad package name {} in provide", ssx_core::text::quoted(p.get_ref())));
                }
            }
        }
        let permissions = raw.permissions.map_or_else(Permissions::default, |p| {
            let list = &p.get_ref().request;
            if list.len() > MAX_LIST || !list.iter().all(|q| valid_permission(q)) {
                let message =
                    format!("[permissions] request: at most {MAX_LIST} names of `[A-Za-z0-9_.-]`, 1 to 128 bytes");
                c.error(p.span(), message);
            }
            p.into_inner()
        });
        let settings = raw.settings.map_or_else(toml::Table::new, |t| {
            if table_depth(t.get_ref()) > MAX_TOML_DEPTH {
                c.error(t.span(), format!("[settings]: nested deeper than {MAX_TOML_DEPTH}"));
            }
            t.into_inner()
        });
        let mut refs = std::mem::take(&mut c.refs);
        refs.sort_by_key(|(_, span)| span.start);
        let mut first: BTreeMap<String, (String, Range<usize>)> = BTreeMap::new();
        for (f, span) in refs {
            let ends = f.match_indices('/').map(|(i, _)| i).chain([f.len()]);
            for end in ends {
                let prefix = &f[..end];
                let (seen, at) =
                    first.entry(prefix.to_ascii_lowercase()).or_insert_with(|| (prefix.to_string(), span.clone()));
                if seen != prefix {
                    let (line, column) = line_col(text, at.start);
                    let what = if end == f.len() { "path" } else { "folder" };
                    let message = format!(
                        "{what} {} differs only in case from {} at line {line}, column {column} (one entry on Windows and macOS)",
                        ssx_core::text::quoted(prefix),
                        ssx_core::text::quoted(seen)
                    );
                    c.error(span.clone(), message);
                    break;
                }
            }
        }
        if !c.diags.is_empty() {
            c.diags.sort_by_key(|d| (d.line, d.column));
            return Err(fail(c.diags));
        }
        Ok(Manifest {
            resource: Resource {
                name,
                version,
                title: r.title.clone(),
                authors: r.authors.clone(),
                description: r.description.clone(),
                api: *r.api.get_ref(),
                kind: r.kind,
                priority: r.priority,
            },
            add,
            overrides,
            patches,
            scripts,
            files,
            shaders,
            actionscript,
            dependencies,
            permissions,
            settings,
            positions,
        })
    }

    pub fn gpu_code(&self) -> bool {
        !self.shaders.files.is_empty()
    }

    pub fn actionscript(&self) -> bool {
        !self.actionscript.files.is_empty()
    }

    pub fn content_ids(&self) -> impl Iterator<Item = &ContentId> {
        self.add.keys().chain(self.overrides.keys())
    }

    pub fn entry(&self, id: &ContentId) -> Option<&Entry> {
        self.add.get(id).or_else(|| self.overrides.get(id))
    }

    pub fn ids_of(&self, kind: Kind) -> impl Iterator<Item = &ContentId> {
        self.content_ids().filter(move |id| id.kind() == kind)
    }
}

#[cfg(test)]
mod tests;
