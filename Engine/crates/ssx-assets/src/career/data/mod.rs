mod readers;
mod records;

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use ssx_formats::attrib::collection::{self, Collection, DecodedCollection, Field, FieldData};
use ssx_formats::attrib::{NameDict, Schema, Value, Vault, fnv1};

pub use readers::*;
pub use records::*;

pub const CLASS_RANGE: u32 = fnv1(b"range");
pub const CLASS_PEAK: u32 = fnv1(b"peak");
pub const CLASS_UI_DROP: u32 = fnv1(b"ui_drop");
pub const CLASS_GE_DROP: u32 = fnv1(b"ge_drop");
pub const CLASS_GE_TRACK: u32 = fnv1(b"ge_track");
pub const CLASS_CHARACTER: u32 = fnv1(b"character");
pub const CLASS_EQUIPMENT: u32 = fnv1(b"equipment");
pub const CLASS_EXPLORE_REWARDS: u32 = fnv1(b"explorerewards");

struct Loaded {
    vlt: Vec<u8>,
    bin: Vec<u8>,
    cols: Vec<Collection>,
    decoded: Vec<DecodedCollection>,
    local: HashMap<(u32, u32), usize>,
}

pub(crate) struct Vaults {
    loaded: Vec<Loaded>,
    pub dict: NameDict,
}

fn read_pair(vlt: &Path) -> Result<(Vec<u8>, Vec<u8>)> {
    let bin = vlt.with_extension("bin");
    Ok((
        std::fs::read(vlt).with_context(|| format!("reading {}", vlt.display()))?,
        std::fs::read(&bin).with_context(|| format!("reading {}", bin.display()))?,
    ))
}

pub(crate) struct Rec<'a> {
    vault: &'a Loaded,
    fields: Vec<&'a Field>,
}

impl Vaults {
    pub fn load(big_root: &Path) -> Result<Self> {
        let dir = big_root.join("bootcache/data/xenon/attribdb");
        let (svlt, sbin) = read_pair(&dir.join("attribdb_schema.vlt"))?;
        let schema = Schema::from_vault(&Vault::parse(&svlt, &sbin)?).context("reading the schema")?;
        let mut loaded = Vec::new();
        let mut dict = NameDict::new();
        for name in ["attribdb.vlt", "ge_attribdb.vlt"] {
            let (vlt, bin) = read_pair(&dir.join(name))?;
            dict.add_strings_from(&vlt);
            dict.add_strings_from(&bin);
            let (cols, decoded) = {
                let v = Vault::parse(&vlt, &bin).with_context(|| format!("parsing {name}"))?;
                let cols = collection::collections(&v)?;
                let decoded = cols.iter().map(|c| c.decode(&v, &schema)).collect::<ssx_formats::Result<Vec<_>>>()?;
                (cols, decoded)
            };
            let local = collection::index(&cols);
            loaded.push(Loaded { vlt, bin, cols, decoded, local });
        }
        Ok(Vaults { loaded, dict })
    }

    pub fn keys_of(&self, class: u32) -> Vec<u32> {
        self.loaded.iter().flat_map(|l| l.cols.iter().filter(|c| c.class == class).map(|c| c.key)).collect()
    }

    pub fn parent(&self, class: u32, key: u32) -> u32 {
        self.loaded.iter().find_map(|l| l.local.get(&(class, key)).map(|&i| l.cols[i].parent)).unwrap_or(0)
    }

    pub fn rec(&self, class: u32, key: u32) -> Result<Rec<'_>> {
        let (l, i) = self
            .loaded
            .iter()
            .find_map(|l| l.local.get(&(class, key)).map(|&i| (l, i)))
            .with_context(|| format!("collection {class:#010x}/{key:#010x} not found"))?;
        let fields =
            collection::with_inherited(&l.cols, &l.decoded, &l.local, i)?.into_iter().map(|(_, f)| f).collect();
        Ok(Rec { vault: l, fields })
    }

    pub fn name(&self, class_name: &str, key: u32) -> String {
        let class = fnv1(class_name.as_bytes());
        let confirmed = self.loaded.iter().find_map(|l| l.local.get(&(class, key)).map(|&i| l.cols[i].export_key));
        match (self.dict.get(key), confirmed) {
            (Some(n), Some(e)) if fnv1(format!("{class_name}/{n}").as_bytes()) == e => n.to_string(),
            _ => format!("{key:08x}"),
        }
    }
}

fn values(d: &FieldData) -> Vec<&Value> {
    match d {
        FieldData::Single(v) => vec![v],
        FieldData::Array(vs) => vs.iter().collect(),
    }
}

fn int_of(v: &Value) -> Option<i64> {
    Some(match v {
        Value::Bool(b) => i64::from(*b),
        Value::Int8(x) => i64::from(*x),
        Value::UInt8(x) => i64::from(*x),
        Value::Int16(x) => i64::from(*x),
        Value::UInt16(x) => i64::from(*x),
        Value::Int32(x) => i64::from(*x),
        Value::UInt32(x) => i64::from(*x),
        Value::Int64(x) => *x,
        Value::UInt64(x) => *x as i64,
        Value::Raw { bytes, .. } if bytes.len() == 4 => {
            i64::from(i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        }
        _ => return None,
    })
}

impl Rec<'_> {
    fn data(&self, key: u32) -> Option<&FieldData> {
        self.fields.iter().find(|f| f.key == key).map(|f| &f.data)
    }

    pub fn keys(&self) -> Vec<u32> {
        self.fields.iter().map(|f| f.key).collect()
    }

    pub fn int(&self, key: u32) -> Option<i64> {
        self.data(key).and_then(|d| values(d).first().and_then(|v| int_of(v)))
    }

    pub fn ints(&self, key: u32) -> Vec<i64> {
        self.data(key).map_or_else(Vec::new, |d| values(d).into_iter().filter_map(int_of).collect())
    }

    pub fn float(&self, key: u32) -> Option<f32> {
        self.data(key).and_then(|d| match values(d).first() {
            Some(Value::Float(x)) => Some(*x),
            _ => None,
        })
    }

    pub fn floats(&self, key: u32) -> Vec<f32> {
        self.data(key).map_or_else(Vec::new, |d| {
            values(d).into_iter().filter_map(|v| if let Value::Float(x) = v { Some(*x) } else { None }).collect()
        })
    }

    pub fn vector(&self, key: u32) -> Option<Vec<f32>> {
        self.data(key).and_then(|d| match values(d).first() {
            Some(Value::Vector2(v)) => Some(v.to_vec()),
            Some(Value::Vector3(v)) => Some(v.to_vec()),
            Some(Value::Vector4(v)) => Some(v.to_vec()),
            _ => None,
        })
    }

    pub fn text(&self, key: u32) -> Option<String> {
        self.texts(key).into_iter().next()
    }

    pub fn texts(&self, key: u32) -> Vec<String> {
        self.data(key).map_or_else(Vec::new, |d| {
            values(d)
                .into_iter()
                .filter_map(|v| if let Value::Text(Some(t)) = v { Some(t.clone()) } else { None })
                .collect()
        })
    }

    pub fn refs(&self, key: u32) -> Vec<u32> {
        self.data(key).map_or_else(Vec::new, |d| {
            values(d)
                .into_iter()
                .filter_map(|v| if let Value::Ref(r) = v { Some(r.collection) } else { None })
                .collect()
        })
    }

    pub fn reference(&self, key: u32) -> u32 {
        self.refs(key).first().copied().unwrap_or(0)
    }

    pub fn raws(&self, key: u32) -> Vec<Vec<u8>> {
        self.data(key).map_or_else(Vec::new, |d| {
            values(d)
                .into_iter()
                .filter_map(|v| if let Value::Raw { bytes, .. } = v { Some(bytes.clone()) } else { None })
                .collect()
        })
    }

    pub fn raw_text(&self, key: u32, offset: u32) -> Option<String> {
        let d = self.data(key)?;
        let Some(Value::Raw { pointers, .. }) = values(d).first().copied() else { return None };
        let p = pointers.iter().find(|p| p.offset == offset)?;
        let file = if p.target.file == 0 { &self.vault.vlt } else { &self.vault.bin };
        let rest = file.get(p.target.offset as usize..)?;
        let end = rest.iter().position(|&b| b == 0)?;
        Some(String::from_utf8_lossy(&rest[..end]).into_owned())
    }
}
