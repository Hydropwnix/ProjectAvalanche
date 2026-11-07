use std::path::{Path, PathBuf};
use std::time::Instant;

use eframe::egui;
use ssx_assets::content::mods::{self, EnabledEntry, EnabledList, ModFolder, ModsReport, ROUTED_KINDS};
use ssx_assets::content::{GameContent, modtool};
use ssx_content::install::InstallError;
use ssx_content::{ContentId, Layer, Registry};

use crate::jobs::Background;

pub struct Snapshot {
    pub list: Result<EnabledList, String>,
    pub rows: Vec<ModFolder>,
    pub report: Option<ModsReport>,
    pub unknown: Vec<(String, ContentId)>,
    pub base: bool,
    pub seconds: f64,
}

pub enum Confirm {
    Remove { folder: String, title: String },
    Delete { folder: String, why: String },
    Replace { package: PathBuf, name: String, installed: String },
}

pub enum InstallEnd {
    Done(Vec<String>),
    Exists { package: PathBuf, name: String, installed: String },
    Failed(String),
}

#[derive(Default)]
pub struct ModsPage {
    pub snapshot: Option<Snapshot>,
    job: Option<Background<Snapshot>>,
    stale: bool,
    pub install: Option<Background<InstallEnd>>,
    pub message: Option<(bool, String)>,
    pub confirm: Option<Confirm>,
    base: Option<Option<PathBuf>>,
}

fn snapshot(app_dir: &Path, big_root: Option<PathBuf>) -> Snapshot {
    let start = Instant::now();
    let (list, rows) = mods::scan(app_dir);
    let empty = rows.is_empty() && list.as_ref().is_ok_and(|l| l.entries.is_empty());
    let (mut report, mut unknown, base) = (None, Vec::new(), big_root.is_some());
    if !empty {
        let inspect = |r: &ModsReport, reg: &Registry| {
            let mut out = Vec::new();
            for row in &rows {
                let Ok(m) = &row.package else { continue };
                if !r.mounted.iter().any(|x| x.name == m.resource.name) {
                    continue;
                }
                for id in m.overrides.keys().filter(|id| ROUTED_KINDS.contains(&id.kind())) {
                    if !reg.providers(id).any(|(at, _)| at.layer < Layer::ClientMods) {
                        out.push((m.resource.name.clone(), id.clone()));
                    }
                }
            }
            out
        };
        let (r, found) = match &big_root {
            Some(big) => GameContent::of(big).preview_client_mods(app_dir, inspect),
            None => {
                let mut registry = Registry::new();
                let r = mods::mount_enabled(&mut registry, app_dir);
                let found = inspect(&r, &registry);
                (r, found)
            }
        };
        unknown = found;
        report = Some(r);
    }
    Snapshot { list, rows, report, unknown, base, seconds: start.elapsed().as_secs_f64() }
}

impl ModsPage {
    pub fn busy(&self) -> bool {
        self.job.is_some() || self.install.is_some()
    }

    pub fn refresh(&mut self, ctx: &egui::Context, app_dir: PathBuf, big_root: Option<PathBuf>) {
        self.base = Some(big_root.clone());
        if self.job.is_some() {
            self.stale = true;
            return;
        }
        self.job = Some(Background::spawn(ctx, move || snapshot(&app_dir, big_root)));
    }

    pub fn base_changed(&self, big_root: &Option<PathBuf>) -> bool {
        self.base.as_ref() != Some(big_root)
    }

    pub fn poll(&mut self) -> bool {
        let mut due = false;
        if let Some(s) = self.job.as_ref().and_then(Background::take) {
            self.snapshot = Some(s);
            self.job = None;
            due = std::mem::take(&mut self.stale);
        }
        if let Some(end) = self.install.as_ref().and_then(Background::take) {
            self.install = None;
            due = true;
            match end {
                InstallEnd::Done(lines) => self.message = Some((true, lines.join("\n"))),
                InstallEnd::Failed(e) => self.message = Some((false, e)),
                InstallEnd::Exists { package, name, installed } => {
                    self.confirm = Some(Confirm::Replace { package, name, installed });
                }
            }
        }
        due
    }

    fn list_of(rows: &[ModFolder], mods: &Path) -> EnabledList {
        let nameable = |f: &str| ssx_content::path::check_package_path(f, false).is_ok() && mods.join(f).is_dir();
        let entries = rows
            .iter()
            .filter(|r| r.listed || nameable(&r.folder))
            .map(|r| EnabledEntry { folder: r.folder.clone(), enabled: r.enabled })
            .collect();
        EnabledList { entries }
    }

    fn write(&mut self, app_dir: &Path) -> bool {
        let Some(s) = &mut self.snapshot else { return false };
        let list = Self::list_of(&s.rows, &app_dir.join(mods::MODS_DIR));
        match list.write(app_dir) {
            Ok(()) => {
                for r in &mut s.rows {
                    r.listed = list.entries.iter().any(|e| e.folder == r.folder);
                }
                s.list = Ok(list);
                true
            }
            Err(e) => {
                self.message = Some((false, format!("mods.toml not written: {e}")));
                false
            }
        }
    }

    pub fn set_enabled(&mut self, app_dir: &Path, i: usize, on: bool) -> bool {
        match self.snapshot.as_mut().and_then(|s| s.rows.get_mut(i)) {
            Some(r) => r.enabled = on,
            None => return false,
        }
        self.write(app_dir)
    }

    pub fn move_row(&mut self, app_dir: &Path, i: usize, step: isize) -> bool {
        let Some(s) = self.snapshot.as_mut() else { return false };
        let j = i as isize + step;
        if j < 0 || j as usize >= s.rows.len() {
            return false;
        }
        s.rows.swap(i, j as usize);
        self.write(app_dir)
    }

    pub fn forget(&mut self, app_dir: &Path, folder: &str) -> bool {
        if let Some(s) = self.snapshot.as_mut() {
            s.rows.retain(|r| r.folder != folder);
        }
        self.write(app_dir)
    }

    pub fn remove(&mut self, app_dir: &Path, folder: &str, permanently: bool) -> bool {
        let path = app_dir.join(mods::MODS_DIR).join(folder);
        let done = if permanently {
            std::fs::remove_dir_all(&path).map_err(|e| e.to_string())
        } else {
            trash::delete(&path).map_err(|e| e.to_string())
        };
        match done {
            Ok(()) => {
                let how = if permanently { "deleted" } else { "moved to the trash" };
                self.message = Some((true, format!("{folder}: {how}")));
                self.forget(app_dir, folder)
            }
            Err(why) if !permanently => {
                self.confirm = Some(Confirm::Delete { folder: folder.to_string(), why });
                false
            }
            Err(e) => {
                self.message = Some((false, format!("{folder}: not deleted: {e}")));
                false
            }
        }
    }

    pub fn start_install(&mut self, ctx: &egui::Context, app_dir: PathBuf, package: PathBuf, replace: bool) {
        self.message = None;
        self.install =
            Some(Background::spawn(ctx, move || match modtool::install(&package, &app_dir, replace, false) {
                Ok(done) => {
                    let r = &done.manifest.resource;
                    let title = if r.title.is_empty() { r.name.clone() } else { r.title.clone() };
                    let index = if done.indexed { "; its content index matched" } else { "" };
                    let mut lines = vec![format!(
                        "Installed {title} {} ({} files{index}) and switched it on.",
                        r.version, done.files
                    )];
                    let notes: Vec<String> = done
                        .findings
                        .iter()
                        .filter(|f| f.severity != ssx_content::check::Severity::Error)
                        .map(ToString::to_string)
                        .collect();
                    lines.extend(notes.iter().take(3).cloned());
                    if notes.len() > 3 {
                        lines.push(format!(
                            "… and {} more remarks (`ssx-assets mod check` lists them)",
                            notes.len() - 3
                        ));
                    }
                    if let Some((_, aside)) = &done.previous {
                        if let Err(e) = trash::delete(aside) {
                            match std::fs::remove_dir_all(aside) {
                                Ok(()) => lines.push(format!("the replaced version was deleted (no trash: {e})")),
                                Err(e2) => {
                                    lines.push(format!("the replaced version stays in {}: {e2}", aside.display()))
                                }
                            }
                        } else {
                            lines.push("the replaced version is in the trash".into());
                        }
                    }
                    InstallEnd::Done(lines)
                }
                Err(e) => match e.downcast_ref::<InstallError>() {
                    Some(InstallError::Exists { name, version, .. }) => InstallEnd::Exists {
                        package,
                        name: name.clone(),
                        installed: version.map_or_else(|| "(unreadable)".to_string(), |v| v.to_string()),
                    },
                    _ => InstallEnd::Failed(format!("{e:#}")),
                },
            }));
    }
}

pub fn open_folder(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(program).arg(dir).spawn().map(drop).map_err(|e| format!("{program}: {e}"))
}
