use std::collections::BTreeSet;

use super::{Checked, Severity};
use crate::id::{ContentId, Kind};
use crate::manifest::{MANIFEST_FILE, Manifest, line_col};
use crate::material::{self, TextureRef};
use crate::model::{self, FileReader};
use crate::object::{self, ObjectCollision, ObjectLods};
use crate::path::PackageTree;
use crate::texture;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelFile {
    pub file: String,
    pub lods: Vec<(usize, usize, usize)>,
    pub collision_triangles: usize,
    pub materials: Vec<Option<String>>,
    pub parts_without_material: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    pub file: String,
    pub at: (usize, usize),
    pub id: ContentId,
    pub what: &'static str,
}

pub(super) fn locate(text: &str, needle: &str) -> (usize, usize) {
    text.find(&format!("\"{needle}\"")).map_or((0, 0), |at| line_col(text, at + 1))
}

pub fn locate_key(text: &str, key: &str) -> (usize, usize) {
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        let t = line.trim_start();
        if t.strip_prefix(key).is_some_and(|r| r.trim_start().starts_with('=')) {
            return line_col(text, at + line.len() - t.len());
        }
        at += line.len();
    }
    (0, 0)
}

pub(super) fn check_model(
    c: &mut Checked,
    tree: &PackageTree,
    id: &ContentId,
    f: &str,
    at: (usize, usize),
    limits: &texture::Limits,
    named: &mut BTreeSet<String>,
) {
    let caps = model::Limits::default();
    let bytes = match tree.read(f, caps.max_file_bytes) {
        Ok(b) => b,
        Err(e) => return c.push(Severity::Error, MANIFEST_FILE, at, format!("{id}: {e}")),
    };
    if let Ok(files) = model::files_of(f, &bytes, &caps) {
        named.extend(files);
    }
    let m = match model::decode_file(f, &bytes, tree as &dyn FileReader, &caps) {
        Ok(m) => m,
        Err(e) => return c.push(Severity::Error, f, (0, 0), e.message),
    };
    let drawn: BTreeSet<usize> =
        m.materials.iter().flat_map(model::ModelMaterial::drawn_images).map(|r| r.image).collect();
    for img in drawn.iter().filter_map(|&i| m.images.get(i)) {
        let settings = texture::TextureSettings::default();
        if let Err(e) = texture::decode_file(&img.file_name, &img.bytes, &settings, limits) {
            c.push(Severity::Error, f, (0, 0), format!("its image {e}"));
        }
    }
    for n in &m.notes {
        c.push(Severity::Note, f, (0, 0), n.clone());
    }
    let lods: Vec<(usize, usize, usize)> = m
        .lods
        .iter()
        .map(|l| {
            let v = l.parts.iter().map(|p| p.mesh.positions.len()).sum();
            (l.parts.len(), v, l.parts.iter().map(|p| p.mesh.triangle_count()).sum())
        })
        .collect();
    let collision_triangles = m.collision.as_ref().map_or(0, |c| c.triangle_count());
    let shown: Vec<String> = lods
        .iter()
        .enumerate()
        .map(|(k, (p, v, t))| format!("LOD {k}: {p} parts, {v} vertices, {t} triangles"))
        .collect();
    let col = match &m.collision {
        Some(c) => format!("collision {} triangles in {} clusters", c.triangle_count(), c.clusters.len()),
        None => "no collision node".into(),
    };
    c.push(Severity::Note, f, (0, 0), format!("{}; {col}", shown.join("; ")));
    let parts_without_material = m.lods.iter().flat_map(|l| &l.parts).filter(|p| p.material.is_none()).count();
    let materials = m.materials.iter().map(|x| x.name.clone()).collect();
    c.models.insert(
        id.clone(),
        ModelFile { file: f.to_string(), lods, collision_triangles, materials, parts_without_material },
    );
}

pub(super) fn check_toml_record(c: &mut Checked, tree: &PackageTree, id: &ContentId, f: &str) {
    let bytes = match tree.read(f, crate::limits::MAX_RECORD_BYTES) {
        Ok(b) => b,
        Err(e) => return c.push(Severity::Error, f, (0, 0), e.to_string()),
    };
    let problems = match id.kind() {
        Kind::Object => object::parse_toml(&bytes, f).map(|o| {
            c.objects.insert(id.clone(), (f.to_string(), o));
        }),
        _ => material::parse_toml(&bytes, f).map(|m| {
            c.materials.insert(id.clone(), (f.to_string(), m));
        }),
    };
    for (line, col, m) in problems.err().unwrap_or_default() {
        c.push(Severity::Error, f, (line, col), m);
    }
}

pub(super) fn own_or_reference(
    c: &mut Checked,
    m: &Manifest,
    (file, text): (&str, &str),
    id: &ContentId,
    what: &'static str,
) -> bool {
    if id.namespace() != m.resource.name {
        let at = locate(text, id.as_str());
        c.references.push(Reference { file: file.to_string(), at, id: id.clone(), what });
        return false;
    }
    if !m.add.contains_key(id) {
        c.push(
            Severity::Error,
            file,
            locate(text, id.as_str()),
            format!("{what} {id}: this package's [add] has no such id"),
        );
        return false;
    }
    true
}

pub(super) fn cross_check(c: &mut Checked, tree: &PackageTree, m: &Manifest) {
    let text_of = |f: &str| tree.read(f, crate::limits::MAX_RECORD_BYTES).ok().and_then(|b| String::from_utf8(b).ok());
    let mut done: BTreeSet<String> = BTreeSet::new();
    for (f, mat) in c.materials.clone().into_values() {
        if !done.insert(f.clone()) {
            continue;
        }
        let text = text_of(&f).unwrap_or_default();
        for r in mat.textures.values() {
            if let TextureRef::Id(t) = r
                && own_or_reference(c, m, (&f, &text), t, "texture")
                && !c.textures.contains_key(t)
            {
                c.push(Severity::Error, &f, locate(&text, t.as_str()), format!("texture {t}: its file did not decode"));
            }
        }
    }
    for (id, (f, o)) in c.objects.clone() {
        if !done.insert(f.clone()) {
            continue;
        }
        let text = text_of(&f).unwrap_or_default();
        let ObjectLods::Model { model, materials, material } = &o.lods else { continue };
        let own_model =
            own_or_reference(c, m, (&f, &text), model, "model").then(|| c.models.get(model).cloned()).flatten();
        if let Some(mf) = &own_model {
            if o.lod_thresholds.len() != mf.lods.len() {
                let message = format!(
                    "lod_thresholds has {} values for the model's {} LODs (the game reads 0 for a missing one; extra ones are not read)",
                    o.lod_thresholds.len(),
                    mf.lods.len()
                );
                c.push(Severity::Warning, &f, locate_key(&text, "lod_thresholds"), message);
            }
            let names: BTreeSet<&str> = mf.materials.iter().flatten().map(String::as_str).collect();
            for name in materials.keys().filter(|n| !names.contains(n.as_str())) {
                c.push(
                    Severity::Warning,
                    &f,
                    locate(&text, name),
                    format!(
                        "[materials] {}: the model has no glTF material of that name",
                        ssx_core::text::quoted(name)
                    ),
                );
            }
            if mf.parts_without_material > 0 && material.is_none() {
                let message = format!(
                    "{} parts name no glTF material and there is no `material`: drawn with glTF's default material",
                    mf.parts_without_material
                );
                c.push(Severity::Note, &f, (0, 0), message);
            }
        }
        for mat in materials.values().chain(material) {
            if own_or_reference(c, m, (&f, &text), mat, "material") && !c.materials.contains_key(mat) {
                c.push(
                    Severity::Error,
                    &f,
                    locate(&text, mat.as_str()),
                    format!("material {mat}: its material.toml did not read"),
                );
            }
        }
        if let ObjectCollision::Model(cm) = &o.collision
            && (cm == model || own_or_reference(c, m, (&f, &text), cm, "collision model"))
            && c.models.get(cm).is_some_and(|x| x.collision_triangles == 0)
        {
            c.push(
                Severity::Note,
                &f,
                (0, 0),
                format!("{id}: {cm} has no `collision` node: the object has no collision"),
            );
        }
        if let Some(s) = &o.controller {
            if s.namespace() == crate::BASE_NAMESPACE {
                c.references.push(Reference {
                    file: f.clone(),
                    at: locate(&text, s.as_str()),
                    id: s.clone(),
                    what: "controller",
                });
            } else {
                let message = format!(
                    "controller {s}: a controller is one of the game's own Lua scripts (ssx:script/…); Luau controllers come with phase M7"
                );
                c.push(Severity::Error, &f, locate(&text, s.as_str()), message);
            }
        }
    }
    let overrides: Vec<ContentId> = m.overrides.keys().filter(|id| id.kind() == Kind::Model).cloned().collect();
    for id in overrides {
        if let Some(mf) = c.models.get(&id).cloned()
            && (mf.lods.len() > 1 || mf.collision_triangles > 0)
        {
            let at = m.positions.entries.get(&id).copied().unwrap_or((0, 0));
            let message = format!(
                "{id}: in place of one disc mesh only LOD 0 is drawn, with the disc's material; an object override gives LODs and collision"
            );
            c.push(Severity::Warning, MANIFEST_FILE, at, message);
        }
    }
}
