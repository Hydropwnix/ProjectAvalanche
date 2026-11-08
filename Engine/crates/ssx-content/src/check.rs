use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;
use std::sync::Arc;

use ssx_core::text::{Text, shown_max};

use crate::glob::{Budget, Pattern};
use crate::id::{ContentId, Kind};
use crate::limits::{MAX_FILE_BYTES, MAX_PACKAGE_BYTES, MAX_RECORD_BYTES};
use crate::manifest::{MANIFEST_FILE, Manifest};
use crate::package::parse_record_at;
use crate::path::{EntryKind, PackageTree};
use crate::texture::settings::SETTINGS_FILE;
use crate::texture::{self, FolderSettings, Shape, TextureKind};
use crate::zip::INDEX_FILE;

mod anim;
pub use anim::AnimFiles;
mod trick;
pub use trick::TrickFiles;
mod gamemode;
pub use gamemode::GamemodeFiles;
mod audio;
mod content;
mod m4;
mod m6;
mod maps;
pub mod marker;
mod movie;
pub use maps::MapFiles;
mod rider;
pub use audio::AudioFiles;
pub use content::{ModelFile, Reference, locate_key};
pub use m4::{M4Files, Particles};
pub use m6::M6Files;
pub use movie::MovieFiles;
pub use rider::{RiderEntryFile, RiderFiles};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
    Note,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub severity: Severity,
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub message: Text,
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let what = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Note => "note",
        };
        let file = if self.file.is_empty() { "(package)" } else { &self.file };
        let file = shown_max(file, ssx_core::text::PATH_CHARS);
        let line = match self.line {
            0 => Text::new(format_args!("{file}: {what}: {}", self.message)),
            l => Text::new(format_args!("{file}:{l}:{}: {what}: {}", self.column, self.message)),
        };
        f.write_str(&line)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextureFile {
    pub file: String,
    pub shape: Shape,
    pub kind: Option<TextureKind>,
    pub width: u32,
    pub height: u32,
}

#[derive(Default)]
pub struct Checked {
    pub findings: Vec<Finding>,
    pub manifest: Option<Manifest>,
    pub tree: Option<PackageTree>,
    pub textures: BTreeMap<ContentId, TextureFile>,
    pub models: BTreeMap<ContentId, ModelFile>,
    pub objects: BTreeMap<ContentId, (String, crate::object::ObjectDef)>,
    pub materials: BTreeMap<ContentId, (String, crate::material::MaterialDef)>,
    pub references: Vec<Reference>,
    pub m4: M4Files,
    pub audio: AudioFiles,
    pub riders: RiderFiles,
    pub m6: M6Files,
    pub markers: marker::MarkerFiles,
    pub movies: MovieFiles,
    pub maps: MapFiles,
    pub anims: AnimFiles,
    pub tricks: TrickFiles,
    pub gamemodes: GamemodeFiles,
}

impl Checked {
    pub fn errors(&self) -> usize {
        self.findings.iter().filter(|f| f.severity == Severity::Error).count()
    }

    fn push(&mut self, severity: Severity, file: &str, (line, column): (usize, usize), message: impl Into<Text>) {
        let message = message.into();
        self.findings.push(Finding { severity, file: file.to_string(), line, column, message });
    }
}

const MAX_UNREFERENCED_SHOWN: usize = 100;
const PATTERN_BUDGET: u64 = 50_000_000;

fn locate_key_in(tree: &PackageTree, file: &str, key: &str) -> (usize, usize) {
    tree.read(file, MAX_RECORD_BYTES)
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .map_or((0, 0), |t| locate_key(&t, key))
}

pub fn check_package(root: &Path, limits: &texture::Limits) -> Checked {
    let mut c = Checked::default();
    let tree = match PackageTree::open(root) {
        Ok(t) => t,
        Err(e) => {
            c.push(Severity::Error, "", (0, 0), e.to_string());
            return c;
        }
    };
    check_tree(&mut c, &tree);
    let settings = folder_settings(&mut c, &tree);
    match tree.kind(MANIFEST_FILE) {
        Some(EntryKind::File) => match tree.read(MANIFEST_FILE, crate::limits::MAX_MANIFEST_BYTES as u64) {
            Ok(bytes) => match Manifest::parse_bytes(&bytes, Path::new(MANIFEST_FILE)) {
                Ok(m) => c.manifest = Some(m),
                Err(e) => {
                    for d in e.diagnostics {
                        c.push(Severity::Error, MANIFEST_FILE, (d.line, d.column), d.message);
                    }
                }
            },
            Err(e) => c.push(Severity::Error, MANIFEST_FILE, (0, 0), e.to_string()),
        },
        _ => c.push(Severity::Error, "", (0, 0), format!("no {MANIFEST_FILE} at the package's root")),
    }
    if let Some(m) = c.manifest.clone() {
        check_manifest_files(&mut c, &tree, &m, &settings, limits);
    }
    c.findings.sort_by(|a, b| (&a.file, a.line, a.column, a.severity).cmp(&(&b.file, b.line, b.column, b.severity)));
    c.tree = Some(tree);
    c
}

pub fn tree_problems(tree: &PackageTree) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for name in &tree.unnamed_names {
        out.push((
            String::new(),
            format!("{}: a name packages cannot use (A-Z a-z 0-9 _ . + - only)", ssx_core::text::quoted(name)),
        ));
    }
    if tree.unnamed > tree.unnamed_names.len() {
        out.push((String::new(), format!("{} more such names", tree.unnamed - tree.unnamed_names.len())));
    }
    let mut total = 0u64;
    for (path, kind) in tree.entries() {
        match kind {
            EntryKind::Link => out.push((path.to_string(), "a link (packages hold files and folders only)".into())),
            EntryKind::Other => out.push((path.to_string(), "a device, FIFO or socket".into())),
            EntryKind::Folder => {}
            EntryKind::File => {
                let size = tree.size(path).unwrap_or(0);
                total = total.saturating_add(size);
                if size > MAX_FILE_BYTES {
                    out.push((path.to_string(), format!("{size} bytes (a file is at most {MAX_FILE_BYTES})")));
                }
            }
        }
    }
    if tree.kind(INDEX_FILE).is_some() {
        let message = format!("{INDEX_FILE} is the packed archive's content index: the name is reserved");
        out.push((INDEX_FILE.to_string(), message));
    }
    if total > MAX_PACKAGE_BYTES {
        out.push((String::new(), format!("{total} bytes in all (a package is at most {MAX_PACKAGE_BYTES})")));
    }
    out
}

fn check_tree(c: &mut Checked, tree: &PackageTree) {
    for (path, message) in tree_problems(tree) {
        c.push(Severity::Error, &path, (0, 0), message);
    }
}

fn folder_settings(c: &mut Checked, tree: &PackageTree) -> BTreeMap<String, Arc<FolderSettings>> {
    let mut out = BTreeMap::new();
    let files: Vec<String> = tree
        .entries()
        .filter(|(p, k)| *k == EntryKind::File && (*p == SETTINGS_FILE || p.ends_with(&format!("/{SETTINGS_FILE}"))))
        .map(|(p, _)| p.to_string())
        .collect();
    for rel in files {
        let dir = rel.rsplit_once('/').map_or("", |(d, _)| d).to_string();
        let parsed =
            tree.read(&rel, MAX_RECORD_BYTES).map_err(|e| e.to_string()).and_then(|b| FolderSettings::parse(&b, &rel));
        match parsed {
            Ok(s) => {
                for (name, _) in s.files() {
                    let path = if dir.is_empty() { name.to_string() } else { format!("{dir}/{name}") };
                    if tree.kind(&path) != Some(EntryKind::File) {
                        let message = format!(
                            "[{}]: this folder has no file of that name (case-sensitive)",
                            ssx_core::text::quoted(name)
                        );
                        c.push(Severity::Error, &rel, s.position(name), message);
                    }
                }
                out.insert(dir, Arc::new(s));
            }
            Err(e) => c.push(Severity::Error, "", (0, 0), e),
        }
    }
    out
}

fn check_manifest_files(
    c: &mut Checked,
    tree: &PackageTree,
    m: &Manifest,
    settings: &BTreeMap<String, Arc<FolderSettings>>,
    limits: &texture::Limits,
) {
    let mut named: BTreeSet<String> = BTreeSet::from([MANIFEST_FILE.to_string()]);
    named.extend(
        settings.keys().map(|d| if d.is_empty() { SETTINGS_FILE.to_string() } else { format!("{d}/{SETTINGS_FILE}") }),
    );
    let present = |c: &mut Checked, at: (usize, usize), id: &dyn fmt::Display, f: &str| -> bool {
        match tree.kind(f) {
            Some(EntryKind::File) => true,
            None => {
                let message = format!(
                    "{id}: names {}, which the package does not have (names are case-sensitive)",
                    ssx_core::text::quoted(f)
                );
                c.push(Severity::Error, MANIFEST_FILE, at, message);
                false
            }
            Some(_) => {
                c.push(
                    Severity::Error,
                    MANIFEST_FILE,
                    at,
                    format!("{id}: {} is not a regular file", ssx_core::text::quoted(f)),
                );
                false
            }
        }
    };
    let mut idle: BTreeMap<Kind, usize> = BTreeMap::new();
    for (id, entry) in m.add.iter().chain(&m.overrides) {
        let at = m.positions.entries.get(id).copied().unwrap_or((0, 0));
        if crate::package::is_rider_kind(id.kind()) {
            let mut all = true;
            for f in entry.files() {
                named.insert(f.to_string());
                all &= present(c, at, id, f);
            }
            if all {
                rider::check_entry(c, tree, id, entry, at, m.add.contains_key(id));
            }
            continue;
        }
        for f in entry.files() {
            named.insert(f.to_string());
            if !present(c, at, id, f) {
                continue;
            }
            match id.kind() {
                Kind::Texture => check_texture(c, tree, id, f, at, settings, limits),
                Kind::Model => content::check_model(c, tree, id, f, at, limits, &mut named),
                Kind::Object | Kind::Material => content::check_toml_record(c, tree, id, f),
                Kind::Sky | Kind::Lighting | Kind::Postfx | Kind::Particles => {
                    m4::check_entry(c, tree, id, f, at, false, &mut named)
                }
                Kind::Sound | Kind::Music => {
                    let own = m.resource.name.clone();
                    audio::check_entry(c, tree, id, f, at, &own, &mut named)
                }
                Kind::Ui | Kind::Font => m6::check_entry(c, tree, m, id, f, at),
                Kind::Marker => marker::check_entry(c, tree, m, id, f, at),
                Kind::Movie => movie::check_entry(c, tree, id, f, at),
                Kind::Map => maps::check_entry(c, tree, m, id, f, at, &mut named),
                Kind::Anim => anim::check_entry(c, tree, id, f, &mut named),
                Kind::Trick => trick::check_entry(c, tree, m, id, f),
                Kind::Gamemode => gamemode::check_entry(c, tree, m, id, f),
                _ if f.to_ascii_lowercase().ends_with(".toml") => check_record(c, tree, f),
                _ => {}
            }
        }
        if !id.kind().read_from_packages() {
            *idle.entry(id.kind()).or_default() += 1;
        }
    }
    content::cross_check(c, tree, m);
    m4::check_shaders(c, tree, m, &mut named);
    m6::check_package(c, tree, m, &mut named);
    trick::cross_check(c, tree);
    for (kind, n) in idle {
        let message = format!(
            "{n} {kind} entr{}: the game does not read this kind from packages; only the files' presence and records \
             are checked",
            if n == 1 { "y" } else { "ies" },
        );
        c.push(Severity::Note, MANIFEST_FILE, (0, 0), message);
    }
    for (id, f) in &m.patches {
        named.insert(f.clone());
        let at = m.positions.patches.get(id).copied().unwrap_or((0, 0));
        if !present(c, at, id, f) {
            continue;
        }
        match id.kind() {
            Kind::Lighting | Kind::Postfx | Kind::Particles => m4::check_entry(c, tree, id, f, at, true, &mut named),
            k if crate::package::is_rider_kind(k) => rider::check_patch(c, tree, id, f),
            _ => check_record(c, tree, f),
        }
    }
    let checked_patches = |k: Kind| {
        matches!(
            k,
            Kind::Lighting | Kind::Postfx | Kind::Particles | Kind::Tuning | Kind::Scoring | Kind::Event | Kind::Drop
        ) || crate::package::is_rider_kind(k)
    };
    if m.patches.keys().any(|id| !checked_patches(id.kind())) {
        let message = "patches of kinds other than lighting, postfx, particles, characters, gear, boards, tuning, \
                       scoring, events and drops are checked as TOML only";
        c.push(Severity::Note, MANIFEST_FILE, (0, 0), message);
    }
    let files: Vec<&str> = tree.entries().filter(|(_, k)| *k == EntryKind::File).map(|(p, _)| p).collect();
    let mut budget = Budget(PATTERN_BUDGET);
    let s = &m.scripts;
    let f = &m.files;
    let groups = [
        ("scripts.client", &s.client, m.positions.scripts),
        ("scripts.server", &s.server, m.positions.scripts),
        ("scripts.shared", &s.shared, m.positions.scripts),
        ("files.include", &f.include, m.positions.files),
        ("files.lazy", &f.lazy, m.positions.files),
        ("files.server_only", &f.server_only, m.positions.files),
        ("shaders.files", &m.shaders.files, m.positions.shaders),
        ("actionscript.files", &m.actionscript.files, m.positions.actionscript),
    ];
    for (section, patterns, at) in groups {
        for text in patterns.iter() {
            let pattern = match Pattern::parse(text) {
                Ok(p) => p,
                Err(e) => {
                    c.push(Severity::Error, MANIFEST_FILE, at, format!("[{section}] {e}"));
                    continue;
                }
            };
            let mut hit = false;
            for file in &files {
                match pattern.matches(file, &mut budget) {
                    Ok(true) => {
                        hit = true;
                        named.insert(file.to_string());
                    }
                    Ok(false) => {}
                    Err(e) => {
                        c.push(
                            Severity::Error,
                            MANIFEST_FILE,
                            at,
                            format!("[{section}] {}: {e}", ssx_core::text::quoted(text)),
                        );
                        return;
                    }
                }
            }
            if !hit {
                c.push(
                    Severity::Warning,
                    MANIFEST_FILE,
                    at,
                    format!("[{section}] {} matches no file", ssx_core::text::quoted(text)),
                );
            }
        }
    }
    if !(s.client.is_empty() && s.server.is_empty() && s.shared.is_empty()) {
        let message = "scripts: each side's Luau files are compiled and their `require`s resolved when the scripts are \
                       loaded";
        c.push(Severity::Note, MANIFEST_FILE, m.positions.scripts, message);
    }
    let unnamed: Vec<&str> = files
        .iter()
        .copied()
        .filter(|f| !named.contains(*f) && *f != INDEX_FILE && !root_document(f) && !f.ends_with(".d.luau"))
        .collect();
    for f in unnamed.iter().take(MAX_UNREFERENCED_SHOWN) {
        c.push(Severity::Warning, f, (0, 0), "named by no entry, pattern or textures.toml (packed anyway)");
    }
    if unnamed.len() > MAX_UNREFERENCED_SHOWN {
        let message = format!("{} more files named by nothing", unnamed.len() - MAX_UNREFERENCED_SHOWN);
        c.push(Severity::Warning, "", (0, 0), message);
    }
}

fn root_document(f: &str) -> bool {
    if f.contains('/') {
        return false;
    }
    let (stem, ext) = f.rsplit_once('.').map_or((f, None), |(s, e)| (s, Some(e)));
    let ext_ok = ext.is_none_or(|e| e.eq_ignore_ascii_case("txt") || e.eq_ignore_ascii_case("md"));
    const NAMES: [&str; 7] = ["README", "LICENSE", "LICENCE", "COPYING", "NOTICE", "CREDITS", "CHANGELOG"];
    ext_ok && NAMES.iter().any(|n| stem.eq_ignore_ascii_case(n))
}

fn check_texture(
    c: &mut Checked,
    tree: &PackageTree,
    id: &ContentId,
    f: &str,
    at: (usize, usize),
    settings: &BTreeMap<String, Arc<FolderSettings>>,
    limits: &texture::Limits,
) {
    let (dir, name) = f.rsplit_once('/').unwrap_or(("", f));
    let s = settings.get(dir).map(|s| s.of(name)).unwrap_or_default();
    let decoded = tree
        .read(f, limits.max_file_bytes)
        .map_err(|e| e.to_string())
        .and_then(|bytes| texture::decode_file(f, &bytes, &s, limits).map_err(|e| e.to_string()));
    match decoded {
        Ok(t) => {
            let file =
                TextureFile { file: f.to_string(), shape: t.shape, kind: t.kind, width: t.width(), height: t.height() };
            c.textures.insert(id.clone(), file);
        }
        Err(e) => c.push(Severity::Error, MANIFEST_FILE, at, format!("{id}: {e}")),
    }
}

fn check_record(c: &mut Checked, tree: &PackageTree, f: &str) {
    match tree.read(f, MAX_RECORD_BYTES).map_err(|e| (0, 0, e.to_string())).and_then(|b| parse_record_at(&b)) {
        Ok(_) => {}
        Err((line, column, message)) => c.push(Severity::Error, f, (line, column), message),
    }
}

#[cfg(test)]
mod tests;
