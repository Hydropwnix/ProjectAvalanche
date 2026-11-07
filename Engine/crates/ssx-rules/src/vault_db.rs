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
    pub key: u32,
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
    Ok(Files { vlt: crate::read::read_file(vlt)?, bin: crate::read::read_file(&bin)? })
}

// TODO(guess): 1 024 files and 64 MiB in all, port limits about ten times the disc's 92 files and 6.2 MB, so that a
// damaged or hostile data folder cannot grow the dictionary without bound; settle: a name dictionary that no longer
// needs the shader files (the rules subset's own list of names).
pub const MAX_SHADER_FILES: usize = 1024;
pub const MAX_SHADER_BYTES: u64 = 64 << 20;

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
    with_vaults(big_root, &pairs, f)
}

pub fn with_level_db<R>(big_root: &Path, track: Option<&str>, f: impl FnOnce(&Db<'_>) -> Result<R>) -> Result<R> {
    match track {
        Some(t) => with_track_db(big_root, t, f),
        None => with_attribdb(big_root, f),
    }
}

pub fn with_attribdb<R>(big_root: &Path, f: impl FnOnce(&Db<'_>) -> Result<R>) -> Result<R> {
    let attribdb = big_root.join("bootcache/data/xenon/attribdb");
    with_vaults(big_root, &[attribdb.join("ge_attribdb.vlt"), attribdb.join("attribdb_schema.vlt")], f)
}

fn with_vaults<R>(big_root: &Path, pairs: &[PathBuf], f: impl FnOnce(&Db<'_>) -> Result<R>) -> Result<R> {
    let files: Vec<Files> = pairs.iter().map(|p| read_pair(p)).collect::<Result<_>>()?;
    let vaults: Vec<Vault> = files.iter().map(|f| Vault::parse(&f.vlt, &f.bin)).collect::<ssx_formats::Result<_>>()?;
    let schema = Schema::from_vault(vaults.last().context("no schema vault")?).context("reading the schema")?;

    let mut dict = NameDict::new();
    for f in &files {
        dict.add_strings_from(&f.vlt);
        dict.add_strings_from(&f.bin);
    }
    let shader_dirs =
        std::fs::read_dir(big_root).into_iter().flatten().flatten().map(|a| a.path().join("data/xenon/shaders"));
    let (mut files_read, mut bytes_read) = (0usize, 0u64);
    for dir in shader_dirs.filter_map(|d| std::fs::read_dir(d).ok()) {
        for e in dir.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "fxo")) {
            if files_read == MAX_SHADER_FILES {
                bail!("{}: more than {MAX_SHADER_FILES} shader files", big_root.display());
            }
            if let Ok(data) = crate::read::read_file(&e.path()) {
                files_read += 1;
                bytes_read += data.len() as u64;
                if bytes_read > MAX_SHADER_BYTES {
                    bail!("{}: shader files of more than {MAX_SHADER_BYTES} bytes", big_root.display());
                }
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
        let mut m = Material { name: self.collection_name(key), key, ..Default::default() };
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
