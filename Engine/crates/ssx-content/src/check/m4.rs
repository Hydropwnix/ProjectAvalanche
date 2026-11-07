use std::collections::BTreeSet;

use super::{Checked, Severity};
use crate::id::{ContentId, Kind};
use crate::lighting::LightingRecord;
use crate::manifest::{MANIFEST_FILE, Manifest};
use crate::material::Problem;
use crate::model::FileReader;
use crate::path::{EntryKind, PackageTree};
use crate::postfx::PostFxRecord;
use crate::sky::{self, SkyDef};
use crate::{lighting, particles, postfx, shader};

#[derive(Default)]
pub struct M4Files {
    pub skies: Vec<(ContentId, String, SkyDef)>,
    pub lighting: Vec<(ContentId, String, bool, LightingRecord)>,
    pub postfx: Vec<(ContentId, String, bool, PostFxRecord)>,
    pub particles: Vec<(ContentId, String, bool, Particles)>,
    pub shaders: Vec<(String, String, shader::Accepted)>,
}

#[derive(Clone)]
pub enum Particles {
    System(ssx_formats::lynx::ParticleSystem),
    Spray(Box<particles::SprayTable>),
    SprayPatch(Box<toml::Table>),
}

fn problems(c: &mut Checked, file: &str, ps: Vec<Problem>) {
    for (l, col, m) in ps {
        c.push(Severity::Error, file, (l, col), m);
    }
}

fn read(c: &mut Checked, tree: &PackageTree, f: &str) -> Option<Vec<u8>> {
    match tree.read(f, crate::limits::MAX_RECORD_BYTES) {
        Ok(b) => Some(b),
        Err(e) => {
            c.push(Severity::Error, f, (0, 0), e.to_string());
            None
        }
    }
}

pub(super) fn check_entry(
    c: &mut Checked,
    tree: &PackageTree,
    id: &ContentId,
    f: &str,
    at: (usize, usize),
    patch: bool,
    named: &mut BTreeSet<String>,
) {
    let ext = f.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match (id.kind(), ext.as_str()) {
        (Kind::Sky, _) => check_sky(c, tree, id, f, at, named),
        (Kind::Lighting, "toml") => {
            let Some(b) = read(c, tree, f) else { return };
            match lighting::parse_toml(&b) {
                Ok(r) => c.m4.lighting.push((id.clone(), f.to_string(), patch, r)),
                Err(ps) => problems(c, f, ps),
            }
        }
        (Kind::Postfx, "toml") => {
            let Some(b) = read(c, tree, f) else { return };
            match postfx::parse_toml(&b) {
                Ok(r) => c.m4.postfx.push((id.clone(), f.to_string(), patch, r)),
                Err(ps) => problems(c, f, ps),
            }
        }
        (Kind::Postfx, "png") => match tree.read(f, 64 << 20) {
            Ok(b) => {
                if let Err(e) = postfx::cube_from_strip(f, &b) {
                    c.push(Severity::Error, MANIFEST_FILE, at, format!("{id}: {e}"));
                }
            }
            Err(e) => c.push(Severity::Error, MANIFEST_FILE, at, format!("{id}: {e}")),
        },
        (Kind::Particles, "toml") => {
            let Some(b) = read(c, tree, f) else { return };
            let read = if particles::is_spray(id) && patch {
                particles::parse_spray_patch(&b).map(|t| Particles::SprayPatch(Box::new(t)))
            } else if particles::is_spray(id) {
                particles::parse_spray(&b).map(|t| Particles::Spray(Box::new(t)))
            } else {
                particles::parse_system(&b).map(Particles::System)
            };
            match read {
                Ok(p) => c.m4.particles.push((id.clone(), f.to_string(), patch, p)),
                Err(ps) => problems(c, f, ps),
            }
        }
        (Kind::Particles, _) => {
            let message = format!("{id}: a particle system is a particles.toml (its textures are texture ids)");
            c.push(Severity::Error, MANIFEST_FILE, at, message);
        }
        _ => {}
    }
}

fn check_sky(
    c: &mut Checked,
    tree: &PackageTree,
    id: &ContentId,
    f: &str,
    at: (usize, usize),
    named: &mut BTreeSet<String>,
) {
    let ext = f.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    let def = match ext.as_str() {
        "toml" => {
            let Some(b) = read(c, tree, f) else { return };
            match sky::parse_toml(&b, f) {
                Ok(d) => d,
                Err(ps) => return problems(c, f, ps),
            }
        }
        "hdr" | "exr" | "ktx2" | "dds" => {
            let mut d = SkyDef::scattering();
            d.name = f.to_string();
            let source = if matches!(ext.as_str(), "hdr" | "exr") {
                sky::CubeSource::Equirect(f.to_string())
            } else {
                sky::CubeSource::Cube(f.to_string())
            };
            d.kind = sky::SkyKind::Cubemap { source, color: sky::ColorEncoding::Srgb, face_size: None };
            d.clouds = false;
            d
        }
        _ => {
            let message = format!("{id}: six faces are named by a sky.toml (`faces = [...]`)");
            return c.push(Severity::Error, MANIFEST_FILE, at, message);
        }
    };
    named.extend(def.files().into_iter().map(str::to_string));
    let limits = sky::Limits::default();
    for file in def.files() {
        if tree.kind(file) != Some(EntryKind::File) {
            c.push(
                Severity::Error,
                f,
                (0, 0),
                format!("names {}, which the package does not have", ssx_core::text::quoted(file)),
            );
            return;
        }
    }
    match sky::probe(&def, tree as &dyn FileReader, &limits) {
        Ok(cost) => match sky::decode(&def, tree as &dyn FileReader, &limits) {
            Ok(cube) => {
                let what = match (&def.kind, cube) {
                    (sky::SkyKind::Cubemap { .. }, Some(cube)) => {
                        format!(
                            "a cubemap sky: faces {}², {} levels, {} bytes decoded",
                            cube.size(),
                            cube.levels.len(),
                            cost.decoded_bytes
                        )
                    }
                    (sky::SkyKind::Dome { model, material }, _) => format!("a dome: {model} with {material}"),
                    _ => "a sky".to_string(),
                };
                c.push(Severity::Note, f, (0, 0), what);
                c.m4.skies.push((id.clone(), f.to_string(), def));
            }
            Err(e) => c.push(Severity::Error, &e.file, (0, 0), e.message),
        },
        Err(e) => c.push(Severity::Error, &e.file, (0, 0), e.message),
    }
}

pub(super) fn check_shaders(c: &mut Checked, tree: &PackageTree, m: &Manifest, named: &mut BTreeSet<String>) {
    let mut budget = crate::glob::Budget(super::PATTERN_BUDGET);
    let patterns: Vec<crate::glob::Pattern> =
        m.shaders.files.iter().filter_map(|p| crate::glob::Pattern::parse(p).ok()).collect();
    let mut declared = |file: &str| patterns.iter().any(|p| p.matches(file, &mut budget).unwrap_or(false));
    let gpu: Vec<String> = tree
        .entries()
        .filter(|(p, k)| {
            *k == EntryKind::File
                && p.rsplit_once('.')
                    .is_some_and(|(_, e)| crate::package::SHADER_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        })
        .map(|(p, _)| p.to_string())
        .collect();
    for f in &gpu {
        if !f.to_ascii_lowercase().ends_with(".wgsl") {
            c.push(Severity::Error, f, (0, 0), "GPU code that is not WGSL: shader hooks are WGSL (`.wgsl`) only");
        } else if !declared(f) {
            let message = "GPU code the manifest does not declare: list it in [shaders] files (the Mods page says \
                           \"contains GPU code\", servers refuse such packages)";
            c.push(Severity::Error, f, (0, 0), message);
        } else {
            named.insert(f.clone());
        }
    }
    let mut seen = BTreeSet::new();
    let materials: Vec<(String, crate::material::MaterialDef)> =
        c.materials.values().filter(|(f, _)| seen.insert(f.clone())).cloned().collect();
    let limits = shader::Limits::default();
    for (mat_file, def) in materials {
        let (Some(hook), Some(family)) = (&def.shader, def.family) else { continue };
        let at = super::locate_key_in(tree, &mat_file, "shader");
        let Some(hooks) = family.hooks() else { continue };
        if tree.kind(&hook.file) != Some(EntryKind::File) {
            c.push(
                Severity::Error,
                &mat_file,
                at,
                format!("shader {}: the package has no such file", ssx_core::text::quoted(&hook.file)),
            );
            continue;
        }
        if !declared(&hook.file) {
            let message = format!(
                "shader {}: not declared in resource.toml's [shaders] files",
                ssx_core::text::quoted(&hook.file)
            );
            c.push(Severity::Error, &mat_file, at, message);
            continue;
        }
        let bytes = match tree.read(&hook.file, limits.max_bytes as u64) {
            Ok(b) => b,
            Err(e) => {
                c.push(Severity::Error, &hook.file, (0, 0), e.to_string());
                continue;
            }
        };
        match shader::check_hook(hooks, &bytes, &limits) {
            Ok(a) => {
                c.push(Severity::Note, &hook.file, (0, 0), format!("accepted: {}", a.summary()));
                c.m4.shaders.push((mat_file.clone(), hook.file.clone(), a));
            }
            Err(ps) => problems(c, &hook.file, ps),
        }
    }
    if m.gpu_code() {
        let message = "the package contains GPU code ([shaders]): a client mod only — servers refuse it, the Mods \
                       page marks it";
        c.push(Severity::Note, MANIFEST_FILE, m.positions.shaders, message);
    }
}
