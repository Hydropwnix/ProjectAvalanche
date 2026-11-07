use anyhow::{Result, bail};

use crate::fs::{FileSource, normalise};
use crate::image::ImageData;

pub fn mount(big_root: &std::path::Path, classic: bool) -> Result<Box<dyn FileSource>> {
    let disc = crate::fs::Disc::new(big_root)?;
    let content = disc.content();
    let mut layers: Vec<Box<dyn FileSource>> = Vec::new();
    if let Some(c) = content.filter(|c| c.packages_provide(ssx_content::Kind::Ui)) {
        layers.push(Box::new(crate::fs::PackageMovies::new(c)));
    }
    if !classic {
        layers.push(Box::new(PortFiles));
    }
    if layers.is_empty() {
        return Ok(Box::new(disc));
    }
    layers.push(Box::new(disc));
    Ok(Box::new(crate::fs::Layered::new(layers)?))
}

pub const ROOT: &str = "ava/fe/";

pub struct PortFiles;

impl PortFiles {
    fn file(path: &str) -> Option<&'static str> {
        let path = normalise(path);
        let rel = path.strip_prefix(ROOT)?;
        super::all_files().find(|(p, _)| *p == rel).map(|(_, t)| *t)
    }
}

impl FileSource for PortFiles {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        Self::file(path).map(|t| t.as_bytes().to_vec())
    }

    fn exists(&self, path: &str) -> bool {
        Self::file(path).is_some()
    }

    fn list(&self, dir: &str, extension: &str) -> Vec<String> {
        let mut dir = normalise(dir);
        if !dir.is_empty() && !dir.ends_with('/') {
            dir.push('/');
        }
        let mut out: Vec<String> = super::all_files()
            .map(|(p, _)| format!("{ROOT}{p}"))
            .filter(|p| {
                p.starts_with(&dir)
                    && std::path::Path::new(p).extension().is_some_and(|e| e.eq_ignore_ascii_case(extension))
            })
            .collect();
        out.sort();
        out.dedup();
        out
    }

    fn image(&self, path: &str) -> Result<ImageData> {
        bail!("{path}: the port's front-end files hold no images")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_port_files_are_served_under_their_root() {
        for (p, _) in super::super::all_files() {
            assert_eq!(normalise(p), *p, "{p} is not a normalised path");
            assert!(PortFiles.exists(&format!("{ROOT}{p}")));
            assert!(!PortFiles.exists(p), "{p} is served outside {ROOT}");
        }
        assert!(PortFiles.read("AVA\\FE\\nav\\overlay.json").is_some());
        let vms = PortFiles.list("ava/fe/viewmodels", "vm");
        assert!(!vms.is_empty() && vms.iter().all(|p| p.starts_with("ava/fe/viewmodels/") && p.ends_with(".vm")));
        assert!(PortFiles.list("data/common/fe/viewmodels/", "vm").is_empty());
        assert!(PortFiles.image("ava/fe/x.dds").is_err());
        let browser = PortFiles.exists("ava/fe/views/ava_browser.json");
        assert_eq!(browser, cfg!(feature = "net"));
        assert_eq!(PortFiles.exists("ava/fe/nav/overlay_net.json"), cfg!(feature = "net"));
        assert!(PortFiles.exists("ava/fe/popups/ava_settings_broken.pop"), "single-player, always there");
    }

    #[test]
    fn a_layer_over_the_disc_reads_its_own_paths_first_and_merges_listings() {
        struct Base;
        impl FileSource for Base {
            fn read(&self, path: &str) -> Option<Vec<u8>> {
                (path == "data/common/fe/a.vm" || path == "ava/fe/nav/overlay.json").then(|| b"base".to_vec())
            }
            fn exists(&self, path: &str) -> bool {
                self.read(path).is_some()
            }
            fn list(&self, dir: &str, _: &str) -> Vec<String> {
                if dir.starts_with("data/") { vec!["data/common/fe/a.vm".into()] } else { Vec::new() }
            }
            fn image(&self, path: &str) -> Result<ImageData> {
                bail!("{path}: base")
            }
        }
        let l = crate::fs::Layered::new(vec![Box::new(PortFiles), Box::new(Base)]).unwrap();
        assert_ne!(l.read("ava/fe/nav/overlay.json").unwrap(), b"base");
        assert_eq!(l.read("data/common/fe/a.vm").unwrap(), b"base");
        assert_eq!(l.list("data/common/fe/", "vm"), ["data/common/fe/a.vm"]);
        assert_eq!(l.image("data/common/fe/x.dds").unwrap_err().to_string(), "data/common/fe/x.dds: base");
        assert!(crate::fs::Layered::new(Vec::new()).is_err());
    }
}
