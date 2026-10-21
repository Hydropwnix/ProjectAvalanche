use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use ssx_formats::attrib::{
    Collection, DecodedCollection, Field, FieldData, NameDict, Schema, Value, Vault, collection,
};

pub const fn fnv1(s: &str) -> u32 {
    ssx_formats::attrib::fnv1(s.as_bytes())
}

pub const CLASS_MATERIAL: u32 = fnv1("ge_material");
pub const CLASS_TEXTURE: u32 = fnv1("ge_texture");
pub const CLASS_EFFECT: u32 = fnv1("ge_effect");
const FIELD_EFFECT: u32 = fnv1("Effect");
const FIELD_TECHNIQUE: u32 = fnv1("Technique");

#[derive(Clone, Debug, Default)]
pub struct Material {
    pub name: String,
    pub effect: Option<String>,
    pub technique: Option<String>,
    pub textures: BTreeMap<String, String>,
    pub floats: BTreeMap<String, f32>,
    pub vectors: BTreeMap<String, Vec<f32>>,
}

impl Material {
    pub fn float(&self, name: &str) -> Option<f32> {
        self.floats.get(name).copied()
    }

    pub fn texture_file(&self, big_root: &Path, param: &str) -> Option<PathBuf> {
        let rel = self.textures.get(param)?.replace('\\', "/");
        Some(big_root.join("cache3/data/xenon/gameexplorer").join(format!("{rel}.xpr")))
    }
}

struct Files {
    vlt: Vec<u8>,
    bin: Vec<u8>,
}

fn read_pair(vlt: &Path) -> Result<Files> {
    let bin = vlt.with_extension("bin");
    Ok(Files {
        vlt: std::fs::read(vlt).with_context(|| format!("reading {}", vlt.display()))?,
        bin: std::fs::read(&bin).with_context(|| format!("reading {}", bin.display()))?,
    })
}

pub struct Loaded<'a> {
    pub cols: Vec<Collection>,
    pub decoded: Vec<DecodedCollection>,
    pub local: HashMap<(u32, u32), usize>,
    _vault: Vault<'a>,
}

pub struct Db<'a> {
    pub dict: NameDict,
    pub vaults: Vec<Loaded<'a>>,
}

pub fn with_track_db<R>(big_root: &Path, track: &str, f: impl FnOnce(&Db<'_>) -> Result<R>) -> Result<R> {
    let attribdb = big_root.join("bootcache/data/xenon/attribdb");
    let pairs = [
        big_root.join(format!("cache1/data/xenon/attribdb/{track}_track.vlt")),
        attribdb.join("ge_attribdb.vlt"),
        attribdb.join("attribdb_schema.vlt"),
    ];
    let files: Vec<Files> = pairs.iter().map(|p| read_pair(p)).collect::<Result<_>>()?;
    let vaults: Vec<Vault> = files.iter().map(|f| Vault::parse(&f.vlt, &f.bin)).collect::<ssx_formats::Result<_>>()?;
    let schema = Schema::from_vault(&vaults[2]).context("reading the schema")?;

    let mut dict = NameDict::new();
    for f in &files {
        dict.add_strings_from(&f.vlt);
        dict.add_strings_from(&f.bin);
    }
    let shader_dirs =
        std::fs::read_dir(big_root).into_iter().flatten().flatten().map(|a| a.path().join("data/xenon/shaders"));
    for dir in shader_dirs.filter_map(|d| std::fs::read_dir(d).ok()) {
        for e in dir.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "fxo")) {
            if let Ok(data) = std::fs::read(e.path()) {
                dict.add_strings_from(&data);
            }
        }
    }
    for t in &schema.types {
        dict.insert(&t.name);
    }

    let mut loaded = Vec::new();
    for v in vaults {
        let cols = collection::collections(&v)?;
        let decoded = cols.iter().map(|c| c.decode(&v, &schema)).collect::<ssx_formats::Result<Vec<_>>>()?;
        let local = collection::index(&cols);
        loaded.push(Loaded { cols, decoded, local, _vault: v });
    }
    f(&Db { dict, vaults: loaded })
}

impl Db<'_> {
    pub fn find(&self, class: u32, key: u32) -> Option<(usize, usize)> {
        self.vaults.iter().enumerate().find_map(|(v, l)| l.local.get(&(class, key)).map(|&i| (v, i)))
    }

    pub fn fields(&self, class: u32, key: u32) -> Result<Vec<&Field>> {
        let (v, i) =
            self.find(class, key).with_context(|| format!("collection {class:#010x}/{key:#010x} not found"))?;
        let l = &self.vaults[v];
        Ok(collection::with_inherited(&l.cols, &l.decoded, &l.local, i)?.into_iter().map(|(_, f)| f).collect())
    }

    pub fn field(&self, class: u32, coll: u32, key: u32) -> Result<Option<&Field>> {
        Ok(self.fields(class, coll)?.into_iter().find(|f| f.key == key))
    }

    pub fn collections_of(&self, vault: usize, class: u32) -> impl Iterator<Item = (u32, usize)> + '_ {
        self.vaults[vault].cols.iter().enumerate().filter(move |(_, c)| c.class == class).map(|(i, c)| (c.key, i))
    }

    pub fn class_name(&self, class: u32) -> String {
        self.dict.get(class).map_or_else(|| format!("{class:#010x}"), str::to_string)
    }

    pub fn field_name(&self, key: u32) -> String {
        self.dict.get(key).map_or_else(|| format!("{key:#010x}"), str::to_string)
    }

    pub fn collection_name(&self, key: u32) -> String {
        self.dict.get(key).map_or_else(|| format!("{key:#010x}"), str::to_string)
    }

    pub fn text_of(&self, class: u32, key: u32) -> Result<Option<String>> {
        Ok(self.fields(class, key)?.iter().find_map(|f| single_text(&f.data)))
    }

    pub fn material(&self, class: u32, key: u32) -> Result<Material> {
        if class != CLASS_MATERIAL {
            bail!("material reference to class {} instead of ge_material", self.class_name(class));
        }
        let mut m = Material { name: self.collection_name(key), ..Default::default() };
        let mut queue = vec![(class, key)];
        let mut seen = HashSet::new();
        while let Some((c, k)) = queue.pop() {
            if !seen.insert((c, k)) || self.find(c, k).is_none() {
                continue;
            }
            let is_params = self.class_name(c).starts_with("ge_materialtype_default");
            for f in self.fields(c, k)? {
                match single(&f.data) {
                    Some(Value::Ref(r)) if r.collection != 0 => match r.class {
                        CLASS_EFFECT if f.key == FIELD_EFFECT => m.effect = self.text_of(r.class, r.collection)?,
                        CLASS_TEXTURE if is_params => {
                            if let Some(path) = self.text_of(r.class, r.collection)? {
                                m.textures.insert(self.field_name(f.key), path);
                            }
                        }
                        CLASS_TEXTURE | CLASS_EFFECT => {}
                        _ => queue.push((r.class, r.collection)),
                    },
                    Some(Value::Text(Some(t))) if f.key == FIELD_TECHNIQUE => m.technique = Some(t.clone()),
                    Some(Value::Float(x)) if is_params => {
                        m.floats.insert(self.field_name(f.key), *x);
                    }
                    Some(Value::Vector2(v)) if is_params => {
                        m.vectors.insert(self.field_name(f.key), v.to_vec());
                    }
                    Some(Value::Vector3(v)) if is_params => {
                        m.vectors.insert(self.field_name(f.key), v.to_vec());
                    }
                    Some(Value::Vector4(v)) if is_params => {
                        m.vectors.insert(self.field_name(f.key), v.to_vec());
                    }
                    _ => {}
                }
            }
        }
        Ok(m)
    }
}

pub fn single(d: &FieldData) -> Option<&Value> {
    match d {
        FieldData::Single(v) => Some(v),
        FieldData::Array(_) => None,
    }
}

pub fn single_text(d: &FieldData) -> Option<String> {
    match single(d) {
        Some(Value::Text(Some(t))) => Some(t.clone()),
        _ => None,
    }
}

pub fn refs(d: &FieldData) -> Vec<ssx_formats::attrib::RefSpec> {
    let vals: Vec<&Value> = match d {
        FieldData::Single(v) => vec![v],
        FieldData::Array(vs) => vs.iter().collect(),
    };
    vals.into_iter().filter_map(|v| if let Value::Ref(r) = v { Some(*r) } else { None }).collect()
}
