use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use ssx_content::limits::MAX_LIST;
use ssx_content::manifest::line_col;
use ssx_content::path::check_package_path;
use ssx_content::{DependencyError, Kind, Layer, Manifest, PackageSource, Registry, Report, start_order};

pub const MODS_DIR: &str = "Mods";
pub const ENABLED_FILE: &str = "mods.toml";
pub const ENABLED_FORMAT: u32 = 1;
pub const MAX_ENABLED_BYTES: u64 = 256 << 10;
pub const ROUTED_KINDS: [Kind; 26] = [
    Kind::Texture,
    Kind::Model,
    Kind::Object,
    Kind::Material,
    Kind::Character,
    Kind::Gear,
    Kind::Board,
    Kind::Sky,
    Kind::Lighting,
    Kind::Postfx,
    Kind::Particles,
    Kind::Sound,
    Kind::Music,
    Kind::Ui,
    Kind::Font,
    Kind::String,
    Kind::Marker,
    Kind::Movie,
    Kind::Tuning,
    Kind::Scoring,
    Kind::Event,
    Kind::Drop,
    Kind::Anim,
    Kind::Map,
    Kind::Trick,
    Kind::Gamemode,
];

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EnabledEntry {
    pub folder: String,
    pub enabled: bool,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct EnabledFile {
    format: u32,
    #[serde(default)]
    mods: Vec<EnabledEntry>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EnabledList {
    pub entries: Vec<EnabledEntry>,
}

impl EnabledList {
    pub fn parse(bytes: &[u8], file: &str) -> Result<Self, String> {
        if bytes.len() as u64 > MAX_ENABLED_BYTES {
            return Err(format!("{file}: over {MAX_ENABLED_BYTES} bytes"));
        }
        let text = std::str::from_utf8(bytes).map_err(|e| format!("{file}: not UTF-8: {e}"))?;
        let parsed: EnabledFile = toml::from_str(text).map_err(|e| {
            let (line, col) = e.span().map_or((0, 0), |s| line_col(text, s.start));
            format!("{file}:{line}:{col}: {}", ssx_core::text::message(e.message()))
        })?;
        if parsed.format != ENABLED_FORMAT {
            let f = parsed.format;
            return Err(format!(
                "{file}: format {f}; this game reads format {ENABLED_FORMAT} (a newer launcher wrote it?)"
            ));
        }
        if parsed.mods.len() > MAX_LIST {
            return Err(format!("{file}: {} entries (at most {MAX_LIST})", parsed.mods.len()));
        }
        let mut seen = BTreeSet::new();
        for e in &parsed.mods {
            if e.folder.contains('/') {
                return Err(format!("{file}: {:?} is not a folder name (one name in {MODS_DIR}/)", e.folder));
            }
            check_package_path(&e.folder, false).map_err(|m| format!("{file}: {m}"))?;
            if !seen.insert(e.folder.to_ascii_lowercase()) {
                return Err(format!("{file}: {:?} is listed twice", e.folder));
            }
        }
        Ok(EnabledList { entries: parsed.mods })
    }

    pub fn to_toml(&self) -> String {
        let file = EnabledFile { format: ENABLED_FORMAT, mods: self.entries.clone() };
        toml::to_string(&file).unwrap_or_default()
    }

    pub fn write(&self, app_dir: &Path) -> Result<(), String> {
        let text = self.to_toml();
        Self::parse(text.as_bytes(), ENABLED_FILE)?;
        ssx_files::create_dir_all(app_dir).map_err(|e| e.to_string())?;
        ssx_files::replace(&app_dir.join(ENABLED_FILE), text.as_bytes(), &ssx_files::Options::new())
            .map_err(|e| e.to_string())
    }

    pub fn read(app_dir: &Path) -> Result<Option<Self>, String> {
        let path = app_dir.join(ENABLED_FILE);
        let name = path.display().to_string();
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("{name}: {e}")),
        };
        if !meta.is_file() || meta.len() > MAX_ENABLED_BYTES {
            return Err(format!("{name}: not a regular file of at most {MAX_ENABLED_BYTES} bytes"));
        }
        let bytes = std::fs::read(&path).map_err(|e| format!("{name}: {e}"))?;
        Self::parse(&bytes, &name).map(Some)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skipped {
    pub folder: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mounted {
    pub folder: String,
    pub name: String,
    pub version: ssx_content::Version,
    pub priority: i32,
    pub kinds: BTreeMap<Kind, usize>,
    pub scripts: usize,
    pub server_scripts: usize,
}

#[derive(Clone, Debug, Default)]
pub struct ModsReport {
    pub list_error: Option<String>,
    pub mounted: Vec<Mounted>,
    pub skipped: Vec<Skipped>,
    pub registry: Report,
    pub blocklists: Vec<String>,
}

fn culprits(e: &DependencyError, packages: &[Manifest]) -> Vec<String> {
    match e {
        DependencyError::DuplicateName(n) => vec![n.clone()],
        DependencyError::ApiLevel { package, .. }
        | DependencyError::Missing { package, .. }
        | DependencyError::Version { package, .. }
        | DependencyError::ProvidedOnly { package, .. } => vec![package.clone()],
        DependencyError::AmbiguousProvide { name, .. } => packages
            .iter()
            .filter(|m| m.dependencies.require.iter().any(|d| d.name == *name))
            .map(|m| m.resource.name.clone())
            .collect(),
        DependencyError::Cycle(path) => path.split(" -> ").map(str::to_string).collect(),
    }
}

pub fn mount_enabled(registry: &mut Registry, app_dir: &Path) -> ModsReport {
    mount_enabled_except(registry, app_dir, None)
}

pub fn package_rules(p: &PackageSource) -> Option<String> {
    let problems = ssx_content::check::tree_problems(p.tree());
    let shown: Vec<String> = problems
        .iter()
        .take(3)
        .map(|(path, problem)| if path.is_empty() { problem.clone() } else { format!("{path}: {problem}") })
        .collect();
    match problems.len() {
        0 => None,
        n if n > 3 => Some(format!("{} (and {} more)", shown.join("; "), n - 3)),
        _ => Some(shown.join("; ")),
    }
}

pub fn mount_enabled_except(registry: &mut Registry, app_dir: &Path, except: Option<&str>) -> ModsReport {
    mount_opened(registry, open_enabled_except(app_dir, except))
}

pub struct Opened {
    report: ModsReport,
    open: Vec<(String, PackageSource)>,
}

impl Opened {
    pub fn sources(&self) -> Vec<&dyn ssx_content::Source> {
        self.open.iter().map(|(_, p)| p as &dyn ssx_content::Source).collect()
    }
}

pub fn open_enabled_except(app_dir: &Path, except: Option<&str>) -> Opened {
    let mut report = ModsReport::default();
    let list = match EnabledList::read(app_dir) {
        Ok(Some(l)) => l,
        Ok(None) => return Opened { report, open: Vec::new() },
        Err(e) => {
            report.list_error = Some(e);
            return Opened { report, open: Vec::new() };
        }
    };
    let mods = app_dir.join(MODS_DIR);
    let followed = ssx_content::blocklist::for_player(app_dir, ssx_content::blocklist::now_secs());
    report.blocklists = followed.lines();
    let mut open: Vec<(String, PackageSource)> = Vec::new();
    for e in list.entries.iter().filter(|e| e.enabled) {
        let root = mods.join(&e.folder);
        let skip = |reason: String| Skipped { folder: e.folder.clone(), reason };
        match std::fs::symlink_metadata(&root) {
            Ok(m) if m.is_dir() => {}
            Ok(m) if m.is_file() && e.folder.to_ascii_lowercase().ends_with(".zip") => {
                report.skipped.push(skip("a .zip package: the game reads package folders (install it first)".into()));
                continue;
            }
            Ok(_) => {
                report.skipped.push(skip(format!("{} is not a folder", root.display())));
                continue;
            }
            Err(err) => {
                report.skipped.push(skip(format!("{}: {err}", root.display())));
                continue;
            }
        }
        match PackageSource::open(&root) {
            Ok(p) if Some(p.manifest.resource.name.as_str()) == except => {
                report.skipped.push(skip("the package --mod-dev loads from its own folder".into()));
            }
            Ok(p) => match package_rules(&p) {
                Some(reason) => report.skipped.push(skip(reason)),
                None => match super::blocklist::package_blocked(&p, &followed.blocked) {
                    Some(reason) => report.skipped.push(skip(reason)),
                    None => open.push((e.folder.clone(), p)),
                },
            },
            Err(err) => report.skipped.push(skip(err.to_string())),
        }
    }
    loop {
        let manifests: Vec<Manifest> = open.iter().map(|(_, p)| p.manifest.clone()).collect();
        let Err(errors) = start_order(&manifests) else { break };
        let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for e in &errors {
            for name in culprits(e, &manifests) {
                out.entry(name).or_default().push(e.to_string());
            }
        }
        let before = open.len();
        open.retain(|(folder, p)| match out.get(&p.manifest.resource.name) {
            Some(reasons) => {
                report.skipped.push(Skipped { folder: folder.clone(), reason: reasons.join("; ") });
                false
            }
            None => true,
        });
        if open.len() == before {
            let all: Vec<String> = errors.iter().map(ToString::to_string).collect();
            for (folder, _) in open.drain(..) {
                report.skipped.push(Skipped { folder, reason: all.join("; ") });
            }
            break;
        }
    }
    Opened { report, open }
}

pub fn mount_opened(registry: &mut Registry, opened: Opened) -> ModsReport {
    let Opened { mut report, open } = opened;
    for (folder, p) in open {
        let m = &p.manifest;
        let mut kinds = BTreeMap::new();
        for id in m.content_ids() {
            *kinds.entry(id.kind()).or_default() += 1;
        }
        let scripts = m.scripts.client.len() + m.scripts.shared.len();
        let server_scripts = m.scripts.server.len() + m.scripts.shared.len();
        let (name, priority, version) = (m.resource.name.clone(), m.resource.priority, m.resource.version);
        report.mounted.push(Mounted { folder, name, version, priority, kinds, scripts, server_scripts });
        registry.mount(Layer::ClientMods, priority, Box::new(p));
    }
    report.registry = registry.report();
    report
}

impl ModsReport {
    pub fn summary(&self) -> String {
        if let Some(e) = &self.list_error {
            return format!("mods: none mounted — the enabled list cannot be read: {e}");
        }
        let names: Vec<&str> = self.mounted.iter().map(|m| m.name.as_str()).collect();
        let mut s = format!("mods: {} mounted ({})", names.len(), names.join(", "));
        if !self.skipped.is_empty() {
            let folders: Vec<&str> = self.skipped.iter().map(|k| k.folder.as_str()).collect();
            s += &format!(", {} skipped ({}) — see the log", folders.len(), folders.join(", "));
        }
        let c = self.registry.conflicts.len();
        if c > 0 {
            s += &format!(", {c} conflicts");
        }
        s
    }

    pub fn lines(&self) -> Vec<ssx_core::text::Text> {
        let mut out = vec![self.summary()];
        for k in &self.skipped {
            out.push(format!("mods: skipped {}: {}", k.folder, k.reason));
        }
        out.extend(self.blocklists.iter().map(|l| format!("mods: {l}")));
        for m in &self.mounted {
            let kinds: Vec<String> = m.kinds.iter().map(|(k, n)| format!("{k} ×{n}")).collect();
            out.push(format!("mods: {} ({}/, priority {}): {}", m.name, m.folder, m.priority, kinds.join(", ")));
            let idle: Vec<String> =
                m.kinds.iter().filter(|(k, _)| !ROUTED_KINDS.contains(k)).map(|(k, n)| format!("{k} ×{n}")).collect();
            if !idle.is_empty() {
                out.push(format!("mods: {}: {} not read from mods yet (no effect)", m.name, idle.join(", ")));
            }
            if m.scripts > 0 {
                out.push(format!(
                    "mods: {}: client scripts ({} [scripts] patterns), run by the game's script host",
                    m.name, m.scripts
                ));
            }
        }
        for c in &self.registry.conflicts {
            out.push(format!("mods: conflict on {}: {} wins over {}", c.id, c.winner, c.losers.join(", ")));
        }
        let mut overrides: BTreeMap<&str, usize> = BTreeMap::new();
        for o in self.registry.overrides.iter().filter(|o| o.winner.0 == Layer::ClientMods) {
            *overrides.entry(o.winner.1.as_str()).or_default() += 1;
        }
        for (name, n) in overrides {
            out.push(format!("mods: {name} overrides {n} ids of lower layers"));
        }
        for d in &self.registry.dangling_patches {
            out.push(format!("mods: {} patches {}, which no source provides", d.source, d.id));
        }
        out.into_iter().map(ssx_core::text::Text::from).collect()
    }
}

#[derive(Debug)]
pub struct ModFolder {
    pub folder: String,
    pub listed: bool,
    pub enabled: bool,
    pub present: bool,
    pub package: Result<Manifest, String>,
}

pub fn scan(app_dir: &Path) -> (Result<EnabledList, String>, Vec<ModFolder>) {
    let list = EnabledList::read(app_dir).map(Option::unwrap_or_default);
    let mods = app_dir.join(MODS_DIR);
    let open = |folder: &str| -> Result<Manifest, String> {
        let root = mods.join(folder);
        match std::fs::symlink_metadata(&root) {
            Ok(m) if m.is_dir() => PackageSource::open(&root).map(|p| p.manifest).map_err(|e| e.to_string()),
            Ok(_) if folder.to_ascii_lowercase().ends_with(".zip") => {
                Err("a .zip: install it (the game reads package folders)".into())
            }
            Ok(_) => Err(format!("{} is not a folder", root.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(format!("no folder {folder} in {MODS_DIR}/")),
            Err(e) => Err(format!("{}: {e}", root.display())),
        }
    };
    let mut rows = Vec::new();
    let mut named = BTreeSet::new();
    if let Ok(l) = &list {
        for e in &l.entries {
            named.insert(e.folder.to_ascii_lowercase());
            let present = std::fs::symlink_metadata(mods.join(&e.folder)).is_ok();
            let (folder, enabled, package) = (e.folder.clone(), e.enabled, open(&e.folder));
            rows.push(ModFolder { folder, listed: true, enabled, present, package });
        }
    }
    let mut others: Vec<String> = std::fs::read_dir(&mods)
        .map(|rd| rd.flatten().filter_map(|e| e.file_name().to_str().map(str::to_string)).collect())
        .unwrap_or_default();
    others.retain(|n| !n.starts_with('.') && !named.contains(&n.to_ascii_lowercase()));
    others.retain(|n| mods.join(n).is_dir() || n.to_ascii_lowercase().ends_with(".zip"));
    others.sort();
    for folder in others {
        let package = open(&folder);
        rows.push(ModFolder { folder, listed: false, enabled: false, present: true, package });
    }
    (list, rows)
}

#[cfg(test)]
mod tests;
