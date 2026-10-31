mod archive;
mod download;
pub mod github;
pub mod install;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui;
use semver::Version;

use crate::jobs::Background;

pub const REPO: &str = "MnkyArts/ProjectAvalanche";
pub const LATEST_URL: &str = "https://api.github.com/repos/MnkyArts/ProjectAvalanche/releases/latest";
pub const WINDOWS_ASSET: &str = "ProjectAvalanche-windows-x64.tar.gz";
pub const LINUX_ASSET: &str = "ProjectAvalanche-linux-x64.tar.gz";

pub fn asset_name() -> &'static str {
    if cfg!(windows) { WINDOWS_ASSET } else { LINUX_ASSET }
}

pub fn current_version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).expect("the package version is a semantic version")
}

pub fn auto_check_enabled() -> bool {
    std::env::var_os("SSX_NO_UPDATE_CHECK").is_none()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    pub tag: String,
    pub title: String,
    pub notes: String,
    pub page: String,
    pub asset: Asset,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
    pub sha256: [u8; 32],
}

#[derive(Clone, Debug)]
pub struct Staged {
    pub release: Release,
    pub dir: PathBuf,
    pub install: PathBuf,
    pub files: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Progress {
    pub fraction: f32,
    pub detail: String,
}

pub enum Update {
    Idle,
    Checking(Background<Result<Option<Release>, String>>),
    UpToDate,
    Available(Release),
    Downloading { release: Release, progress: Arc<Mutex<Progress>>, job: Background<Result<Staged, String>> },
    Ready(Staged),
    Failed(String),
}

impl Update {
    pub fn start_check(ctx: &egui::Context) -> Self {
        let current = current_version();
        Update::Checking(Background::spawn(ctx, move || github::check(&current)))
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        let next = match self {
            Update::Checking(job) => job.take().map(|result| match result {
                Ok(Some(release)) => Update::Available(release),
                Ok(None) => Update::UpToDate,
                Err(e) => Update::Failed(e),
            }),
            Update::Downloading { job, .. } => job.take().map(|result| match result {
                Ok(staged) => Update::Ready(staged),
                Err(e) => Update::Failed(e),
            }),
            _ => None,
        };
        if let Some(next) = next {
            *self = next;
        }
        if matches!(self, Update::Checking(_) | Update::Downloading { .. }) {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    pub fn download(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let Update::Available(release) = self else {
            return Err("no update is ready to download".into());
        };
        let install = install::install_dir()?;
        let release = release.clone();
        let progress = Arc::new(Mutex::new(Progress { fraction: 0.0, detail: "Starting the download…".into() }));
        let (p, c) = (progress.clone(), ctx.clone());
        let wanted = release.clone();
        let job = Background::spawn(ctx, move || stage(&wanted, &install, &p, &c));
        *self = Update::Downloading { release, progress, job };
        Ok(())
    }

    pub fn apply(&mut self) -> Result<(), String> {
        let Update::Ready(staged) = self else {
            return Err("no update is ready to install".into());
        };
        install::apply(staged)
    }

    pub fn discard(&mut self) {
        if let Update::Ready(staged) = self {
            let _ = std::fs::remove_dir_all(&staged.dir);
        }
        *self = Update::Idle;
    }

    pub fn available(&self) -> Option<&Release> {
        match self {
            Update::Available(release) => Some(release),
            _ => None,
        }
    }

    pub fn progress(&self) -> Option<Progress> {
        match self {
            Update::Downloading { progress, .. } => progress.lock().ok().map(|p| p.clone()),
            _ => None,
        }
    }

    pub fn ready(&self) -> Option<&Release> {
        match self {
            Update::Ready(staged) => Some(&staged.release),
            _ => None,
        }
    }
}

fn stage(
    release: &Release,
    install: &Path,
    progress: &Arc<Mutex<Progress>>,
    ctx: &egui::Context,
) -> Result<Staged, String> {
    let dir = fresh_dir(install, &release.version.to_string())?;
    let result = (|| {
        let archive = download::fetch(release, &dir, progress, ctx)?;
        if let Ok(mut p) = progress.lock() {
            p.fraction = 1.0;
            p.detail = "Unpacking the update…".into();
        }
        ctx.request_repaint();
        let files = archive::extract(&archive, &dir)?;
        let _ = std::fs::remove_file(&archive);
        Ok::<_, String>(files)
    })();
    match result {
        Ok(files) => Ok(Staged { release: release.clone(), dir, install: install.to_path_buf(), files }),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&dir);
            Err(e)
        }
    }
}

fn fresh_dir(base: &Path, version: &str) -> Result<PathBuf, String> {
    for n in 0..1000u32 {
        let dir = base.join(format!(".update-{version}-{n}"));
        match std::fs::create_dir(&dir) {
            Ok(()) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
                }
                return Ok(dir);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(format!(
                    "cannot make an update folder in {}: {e}. The launcher's folder must be writable",
                    base.display()
                ));
            }
        }
    }
    Err(format!("cannot make an update folder in {}: too many leftovers", base.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_asset_is_the_one_for_this_system() {
        assert_eq!(asset_name(), if cfg!(windows) { WINDOWS_ASSET } else { LINUX_ASSET });
    }

    #[test]
    fn the_current_version_is_the_package_version() {
        assert_eq!(current_version(), Version::parse(env!("CARGO_PKG_VERSION")).unwrap());
    }
}
