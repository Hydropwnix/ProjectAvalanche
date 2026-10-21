use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use ssx_formats::ant::{Guid, Object, Schema, Stream, Value};

pub struct AssetDb {
    pub streams: Vec<(String, Stream)>,
    index: HashMap<Guid, (usize, usize)>,
}

#[derive(Clone, Copy)]
pub struct Asset<'a> {
    pub schema: &'a Schema,
    pub obj: &'a Object,
    pub guid: Guid,
}

impl AssetDb {
    pub fn load(anim_dir: &Path, bundles: &[&str]) -> Result<Self> {
        let mut streams = Vec::new();
        for b in bundles {
            let p = anim_dir.join(b);
            let data = std::fs::read(&p).with_context(|| format!("reading {}", p.display()))?;
            streams.push((b.to_string(), Stream::parse(&data).with_context(|| format!("parsing {}", p.display()))?));
        }
        let mut index = HashMap::new();
        for (si, (_, s)) in streams.iter().enumerate() {
            for (ai, a) in s.assets.iter().enumerate() {
                if let Some(g) = a.object.guid(&s.schema).filter(|g| !g.is_null()) {
                    index.entry(g).or_insert((si, ai));
                }
            }
        }
        Ok(Self { streams, index })
    }

    pub fn get(&self, g: Guid) -> Option<Asset<'_>> {
        let &(s, a) = self.index.get(&g)?;
        let st = &self.streams[s].1;
        Some(Asset { schema: &st.schema, obj: &st.assets[a].object, guid: g })
    }

    pub fn by_name(&self, name: &str) -> Option<Asset<'_>> {
        self.streams.iter().find_map(|(_, s)| {
            let a = s.assets.iter().find(|a| a.object.name(&s.schema) == Some(name))?;
            Some(Asset { schema: &s.schema, obj: &a.object, guid: a.object.guid(&s.schema)? })
        })
    }

    pub fn all_of_class<'a>(&'a self, class: &'a str) -> impl Iterator<Item = Asset<'a>> + 'a {
        self.streams.iter().flat_map(move |(_, s)| {
            s.assets_of_class(class)
                .filter_map(|a| Some(Asset { schema: &s.schema, obj: &a.object, guid: a.object.guid(&s.schema)? }))
        })
    }
}

impl AssetDb {
    pub fn character_config(&self) -> Option<Asset<'_>> {
        self.streams
            .iter()
            .skip(1)
            .find_map(|(_, s)| {
                let a = s.assets_of_class("AnimatableConfigAsset").next()?;
                Some(Asset { schema: &s.schema, obj: &a.object, guid: a.object.guid(&s.schema)? })
            })
            .or_else(|| self.by_name("Snowboarder_config"))
    }

    pub fn config_overrides(&self, config: Asset<'_>) -> HashMap<String, Guid> {
        let mut chain = Vec::new();
        let mut next = Some(config);
        while let Some(a) = next.take() {
            chain.push((a.guids("OverrideGameStateTarget"), a.guids("OverrideGameStateData")));
            next = a.guid("Parent").and_then(|p| self.get(p)).filter(|_| chain.len() < 16);
        }
        let mut out = HashMap::new();
        for (targets, data) in chain.iter().rev() {
            for (t, d) in targets.iter().zip(data) {
                if let Some(t) = self.get(*t) {
                    out.insert(t.name().to_string(), *d);
                }
            }
        }
        out
    }
}

impl<'a> Asset<'a> {
    pub fn class(&self) -> &'a str {
        &self.obj.type_desc(self.schema).name
    }

    pub fn is(&self, class: &str) -> bool {
        self.obj.class_chain(self.schema).contains(&class)
    }

    pub fn name(&self) -> &'a str {
        self.obj.name(self.schema).unwrap_or("")
    }

    pub fn value(&self, field: &str) -> Option<&'a Value> {
        self.obj.get(self.schema, field)
    }

    pub fn f32(&self, field: &str) -> Option<f32> {
        self.value(field)?.as_f32()
    }

    pub fn int(&self, field: &str) -> Option<i64> {
        self.value(field)?.as_i64()
    }

    pub fn bool(&self, field: &str) -> Option<bool> {
        self.value(field)?.as_bool()
    }

    pub fn guid(&self, field: &str) -> Option<Guid> {
        self.value(field)?.as_guid().filter(|g| !g.is_null())
    }

    pub fn guids(&self, field: &str) -> Vec<Guid> {
        self.value(field)
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_guid).filter(|g| !g.is_null()).collect())
            .unwrap_or_default()
    }

    pub fn array(&self, field: &str) -> &'a [Value] {
        self.value(field).and_then(Value::as_array).unwrap_or_default()
    }
}

pub fn field<'a>(schema: &'a Schema, v: &'a Value, name: &str) -> Option<&'a Value> {
    match v {
        Value::Struct(s) => s.own(schema, name),
        Value::DataRef(Some(o)) => o.get(schema, name),
        _ => None,
    }
}
