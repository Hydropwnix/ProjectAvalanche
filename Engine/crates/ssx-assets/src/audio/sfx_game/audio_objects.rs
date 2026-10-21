use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use ssx_formats::attrib::{FieldData, Schema, Value, Vault, collection, fnv1};

const CLASS: u32 = fnv1(b"audio_objects");
const FIELD_TYPE: u32 = 0xC410_DAE9;
const FIELD_OBJECT_INFO: u32 = 0x77BB_D5BB;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Materials {
    pub board: i32,
    pub collision: i32,
}

#[derive(Clone, Debug, Default)]
pub struct AudioObjects {
    by_name: HashMap<(String, String), Materials>,
}

impl AudioObjects {
    pub fn load(big_root: &Path) -> Result<Self> {
        let dir = big_root.join("bootcache/data/xenon/attribdb");
        let read = |name: &str| -> Result<(Vec<u8>, Vec<u8>)> {
            let p = dir.join(name);
            Ok((
                std::fs::read(&p).with_context(|| format!("reading {}", p.display()))?,
                std::fs::read(p.with_extension("bin")).with_context(|| format!("reading {}", p.display()))?,
            ))
        };
        let (svlt, sbin) = read("attribdb_schema.vlt")?;
        let schema = Schema::from_vault(&Vault::parse(&svlt, &sbin)?).context("reading the schema")?;
        let (vlt, bin) = read("attribdb.vlt")?;
        let vault = Vault::parse(&vlt, &bin).context("parsing attribdb.vlt")?;
        let cols = collection::collections(&vault)?;
        let decoded = cols.iter().map(|c| c.decode(&vault, &schema)).collect::<ssx_formats::Result<Vec<_>>>()?;
        let index = collection::index(&cols);
        let mut by_name = HashMap::new();
        for (i, _) in cols.iter().enumerate().filter(|(_, c)| c.class == CLASS) {
            let fields = collection::with_inherited(&cols, &decoded, &index, i)?;
            let field = |k: u32| fields.iter().map(|(_, f)| *f).find(|f| f.key == k);
            let Some(FieldData::Single(Value::Text(Some(type_name)))) = field(FIELD_TYPE).map(|f| &f.data) else {
                continue;
            };
            let Some(FieldData::Single(Value::Raw { bytes, pointers })) = field(FIELD_OBJECT_INFO).map(|f| &f.data)
            else {
                continue;
            };
            let Some(p) = pointers.iter().find(|p| p.offset == 0) else { continue };
            if bytes.len() < 0x14 {
                continue;
            }
            let word = |at: usize| i32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
            let m = Materials { board: word(0xC), collision: word(0x10) };
            by_name.insert((type_name.clone(), vault.cstr(p.target)?), m);
        }
        Ok(Self { by_name })
    }

    pub fn get(&self, type_name: &str, sub_type: &str) -> Option<Materials> {
        self.by_name.get(&(type_name.to_string(), sub_type.to_string())).copied()
    }

    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }
}
