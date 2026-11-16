use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub const SHADER_FOLDER: &str = "data/xenon/shaders";

pub fn effect_file(big_root: &Path, name: &str) -> Option<PathBuf> {
    let stem = effect_stem(name);
    if stem.is_empty() || stem.contains(['/', '\\']) {
        return None;
    }
    crate::data_roots::find(big_root, &format!("{SHADER_FOLDER}/{stem}.fxo"))
}

pub fn effect_stem(name: &str) -> &str {
    name.trim_end_matches(".fxo").trim_end_matches(".fx")
}

pub fn read_effect(big_root: &Path, name: &str) -> Result<(PathBuf, Vec<u8>)> {
    let path = effect_file(big_root, name).with_context(|| {
        format!("{SHADER_FOLDER}/{}.fxo: in no mounted package, title update or disc archive", effect_stem(name))
    })?;
    let data = ssx_rules::read::read_file(&path)?;
    Ok((path, data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effects_resolve_through_the_packages_then_the_disc() {
        let _guard = crate::OVERLAY_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = std::env::temp_dir().join(format!("ssx-effect-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (disc, pkg) = (dir.join("big"), dir.join("dlc_paid_mnt_eddie"));
        let write = |path: PathBuf| {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"x").unwrap();
            path
        };
        let disc_default = write(disc.join("cache1").join(SHADER_FOLDER).join("object_default.fxo"));
        let disc_sky = write(disc.join("bootcache").join(SHADER_FOLDER).join("sky.fxo"));
        let pkg_default = write(pkg.join(SHADER_FOLDER).join("object_default.fxo"));
        assert_eq!(effect_file(&disc, "object_default.fx"), Some(disc_default.clone()));
        crate::data_roots::set_overlays(vec![pkg.clone()]);
        let found = (
            effect_file(&disc, "object_default.fx"),
            effect_file(&disc, "object_default"),
            effect_file(&disc, "sky.fxo"),
            ["../object_default", "x/object_default", ".fx", ""].map(|f| effect_file(&disc, f)),
        );
        crate::data_roots::set_overlays(Vec::new());
        assert_eq!(found.0, Some(pkg_default.clone()));
        assert_eq!(found.1, Some(pkg_default));
        assert_eq!(found.2, Some(disc_sky));
        assert_eq!(found.3, [None, None, None, None]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
