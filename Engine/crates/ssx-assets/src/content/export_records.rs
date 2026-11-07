use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use ssx_content::material::{MaterialDef, TextureRef, effect_phase};
use ssx_content::model::collision::SURFACES;
use ssx_content::model::write::{WriteModel, glb};
use ssx_content::object::{ObjectCollision, ObjectDef, ObjectLods};
use ssx_content::{ContentHash, ContentId, Kind, Source};

use super::export::{ExportRow, record_exports};
use super::models::DiscModels;
use super::objects::DiscRecords;

fn leaf(id: &ContentId) -> &str {
    id.path().rsplit('/').next().unwrap_or(id.path())
}

pub fn material_toml(def: &MaterialDef) -> Result<String, String> {
    let family = def.family.ok_or_else(|| {
        format!("its effect {} is replaced by packages from phase {}", def.effect, effect_phase(&def.effect))
    })?;
    let stem = def.effect.strip_suffix(".fx").unwrap_or(&def.effect);
    let mut out = format!(
        "# {} as the disc defines it (an export for your own use: Docs/architecture/content.md §13.6).\nfamily = \"{}\"\neffect = \"{stem}\"\n",
        def.name,
        family.as_str()
    );
    if let Some(t) = &def.technique {
        out += &format!("technique = {t:?}\n");
    }
    let mut rest = Vec::new();
    out += "\n[textures]\n";
    for (k, r) in &def.textures {
        match r {
            TextureRef::Id(id) if family.textures().contains(&k.as_str()) => out += &format!("{k} = \"{id}\"\n"),
            TextureRef::Id(id) => rest.push(format!("{k} = \"{id}\"")),
            TextureRef::Path(p) => rest.push(format!("{k} = {p:?} (not a texture id)")),
            TextureRef::ModelImage { .. } => {}
        }
    }
    out += "\n[parameters]\n";
    let number = |v: f32| if v.fract() == 0.0 && v.abs() < 1e15 { format!("{v:.1}") } else { format!("{v}") };
    let read = |k: &str| family.parameters().contains(&k);
    let vector = |k: &str| ssx_content::material::VECTOR_PARAMETERS.contains(&k);
    for (k, v) in &def.floats {
        let line = format!("{k} = {}", number(*v));
        if read(k) && !vector(k) && v.is_finite() { out += &format!("{line}\n") } else { rest.push(line) }
    }
    for (k, v) in &def.vectors {
        let line = format!("{k} = [{}]", v.iter().map(|x| number(*x)).collect::<Vec<_>>().join(", "));
        let fits = read(k) && vector(k) && (2..=4).contains(&v.len()) && v.iter().all(|x| x.is_finite());
        if fits { out += &format!("{line}\n") } else { rest.push(line) }
    }
    if !rest.is_empty() {
        out += "\n# the record's other values, which the port's family does not read:\n";
        rest.iter().for_each(|l| out += &format!("# {l}\n"));
    }
    Ok(out)
}

fn surface_name(id: u32) -> Option<&'static str> {
    SURFACES.iter().find(|(_, s)| *s == id).map(|(n, _)| *n)
}

fn object_toml(o: &ObjectDef, package: &str, name: &str, materials: &BTreeMap<String, String>) -> String {
    let mut out = format!(
        "# {} as the disc defines it (an export for your own use: Docs/architecture/content.md §13.7).\nmodel = \"{package}:model/{name}\"\n",
        o.name
    );
    let t: Vec<String> = o.lod_thresholds.iter().map(|x| format!("{x}")).collect();
    out += &format!("lod_thresholds = [{}]\n", t.join(", "));
    if matches!(o.collision, ObjectCollision::None) {
        out += "collision = \"none\"\n";
    }
    if let Some(c) = &o.controller {
        out += &format!("controller = \"{c}\"\n");
    }
    out += "\n[materials]\n";
    for (gltf, id) in materials {
        out += &format!("{gltf:?} = \"{id}\"\n");
    }
    if let Some(a) = &o.audio {
        out += &format!("\n[audio]\ntype = {:?}\nsub_type = {:?}\n", a.type_name, a.sub_type);
    }
    if let Some(b) = &o.rigid_body {
        let v3 = |v: [f32; 3]| format!("[{}, {}, {}]", v[0], v[1], v[2]);
        out += &format!(
            "\n[rigid_body]\noffset = {}\naxis = {}\nhalf_height = {}\nradius = {}\nfat = {}\ninv_mass = {}\ninv_inertia = {}\nmax_speed = {}\nmax_angular_speed = {}\nlinear_damping = {}\nangular_damping = {}\nmaterial = {}\n",
            v3(b.offset),
            v3(b.axis),
            b.half_height,
            b.radius,
            b.fat,
            b.inv_mass,
            v3(b.inv_inertia),
            b.max_speed,
            b.max_angular_speed,
            b.linear_damping,
            b.angular_damping,
            v3(b.material)
        );
    }
    out
}

pub fn write_files(
    id: &ContentId,
    out_dir: &Path,
    app_dir: &Path,
    files: Vec<(String, Vec<u8>)>,
) -> Result<Vec<PathBuf>> {
    write_all(id, out_dir, app_dir, files)
}

fn write_all(id: &ContentId, out_dir: &Path, app_dir: &Path, files: Vec<(String, Vec<u8>)>) -> Result<Vec<PathBuf>> {
    let mut rows = Vec::new();
    let mut written = Vec::new();
    for (rel, bytes) in files {
        let path = out_dir.join(&rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(&path, &bytes).with_context(|| format!("writing {}", path.display()))?;
        let file = path.display().to_string();
        rows.push(ExportRow { hash: ContentHash::of(&bytes), size: bytes.len() as u64, id: id.to_string(), file });
        written.push(path);
    }
    record_exports(app_dir, &rows)?;
    Ok(written)
}

pub fn export_record(
    big_root: &Path,
    id: &ContentId,
    out_dir: &Path,
    app_dir: &Path,
    package: &str,
) -> Result<Vec<PathBuf>> {
    if matches!(id.kind(), Kind::Sound | Kind::Music) {
        return super::export_audio::export_to(big_root, id, out_dir, app_dir);
    }
    if id.kind() == Kind::Movie {
        return super::fmv::export::export_to(big_root, id, out_dir, app_dir);
    }
    if matches!(id.kind(), Kind::Ui | Kind::String | Kind::Font) {
        return write_all(id, out_dir, app_dir, super::m6::export::files(big_root, id)?);
    }
    if super::data::DATA_KINDS.contains(&id.kind()) {
        return write_all(id, out_dir, app_dir, super::data::export::files(big_root, id)?);
    }
    let models = Arc::new(DiscModels::index(big_root)?);
    let name = leaf(id).to_string();
    let files = match id.kind() {
        Kind::Model => {
            let m = models.decode(id).map_err(|e| anyhow::anyhow!("{e}"))?;
            let parts = m.lods[0].parts.iter().map(|p| (&*p.mesh, None)).collect();
            let w = WriteModel { name: &name, lods: vec![parts], materials: Vec::new(), collision: None };
            vec![(format!("{name}.glb"), glb(&w).map_err(anyhow::Error::msg)?)]
        }
        Kind::Material => {
            let records = DiscRecords::new(big_root, models);
            let def = records.load(id)?.into_value::<MaterialDef>().context("not a material")?;
            let text = material_toml(&def).map_err(|e| anyhow::anyhow!("{id}: {e}"))?;
            vec![(format!("{name}.toml"), text.into_bytes())]
        }
        Kind::Object => export_object(big_root, id, &models, package, &name)?,
        Kind::Lighting | Kind::Postfx | Kind::Particles => {
            super::export_m4::export(big_root, id, &models, package, &name)?
        }
        k => bail!("{id}: {} is not exported by this command's records part", k.as_str()),
    };
    write_all(id, out_dir, app_dir, files)
}

fn export_object(
    big_root: &Path,
    id: &ContentId,
    models: &Arc<DiscModels>,
    package: &str,
    name: &str,
) -> Result<Vec<(String, Vec<u8>)>> {
    let records = DiscRecords::new(big_root, models.clone());
    let o = records.load(id)?.into_value::<ObjectDef>().context("not an object")?;
    let ObjectLods::Records(lods) = &o.lods else { bail!("{id}: not a disc record") };
    let mut files = Vec::new();
    let mut slots: Vec<ContentId> = Vec::new();
    let mut meshes = Vec::new();
    for lod in lods {
        let mut parts = Vec::new();
        for r in lod {
            let Some(model) = &r.model else { continue };
            let m = models.decode(model).map_err(|e| anyhow::anyhow!("{e}"))?;
            let slot = r.material.as_ref().map(|mat| match slots.iter().position(|s| s == mat) {
                Some(i) => i,
                None => {
                    slots.push(mat.clone());
                    slots.len() - 1
                }
            });
            parts.push((m.lods[0].parts[0].mesh.clone(), slot));
        }
        meshes.push(parts);
    }
    let lods: Vec<Vec<(&ssx_content::model::Mesh, Option<usize>)>> =
        meshes.iter().map(|l| l.iter().map(|(m, s)| (&**m, *s)).collect()).collect();
    let collision = match &o.collision {
        ObjectCollision::Decoded { collision, .. } => {
            if let Some(c) = collision.clusters.iter().find(|c| surface_name(c.surface_id).is_none()) {
                bail!("{id}: a collision surface id {} that no ava_surface names", c.surface_id);
            }
            Some(collision.clone())
        }
        _ => None,
    };
    let materials: Vec<String> = slots.iter().map(|s| leaf(s).to_string()).collect();
    let w = WriteModel { name, lods, materials: materials.clone(), collision: collision.as_deref() };
    files.push((format!("models/{name}.glb"), glb(&w).map_err(anyhow::Error::msg)?));
    let mut bound = BTreeMap::new();
    for mat in &slots {
        let def = records.load(mat)?.into_value::<MaterialDef>().context("not a material")?;
        match material_toml(&def) {
            Ok(text) => {
                files.push((format!("materials/{}.toml", leaf(mat)), text.into_bytes()));
                bound.insert(leaf(mat).to_string(), format!("{package}:material/{}", leaf(mat)));
            }
            Err(_) => {
                bound.insert(leaf(mat).to_string(), mat.to_string());
            }
        }
    }
    files.push((format!("objects/{name}.toml"), object_toml(&o, package, name, &bound).into_bytes()));
    Ok(files)
}
