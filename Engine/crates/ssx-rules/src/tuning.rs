use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use ssx_formats::ant::{Guid, Object, Schema, Stream, Value};

use crate::curve::Curve;

pub const BUNDLE_PATH: &str = "cache0/data/common/anim/anim_all.cba";

pub struct Bundle {
    pub stream: Stream,
    guids: HashMap<Guid, usize>,
    data_root: Option<std::path::PathBuf>,
}

impl Bundle {
    pub fn load(big_root: &Path) -> Result<Self> {
        let path = big_root.join(BUNDLE_PATH);
        let data = crate::read::read_file(&path)?;
        Self::parse(&data).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn parse(data: &[u8]) -> Result<Self> {
        Ok(Self::from_stream(Stream::parse(data)?))
    }

    pub fn from_stream(stream: Stream) -> Self {
        let guids = stream.guid_index();
        Self { stream, guids, data_root: None }
    }

    pub fn with_data_root(mut self, root: &Path) -> Self {
        self.data_root = Some(root.to_path_buf());
        self
    }

    pub fn data_root(&self) -> Option<&Path> {
        self.data_root.as_deref()
    }

    pub fn schema(&self) -> &Schema {
        &self.stream.schema
    }

    pub fn view(&self, name: &str, class: &str) -> Result<AssetView<'_>> {
        let asset = self.stream.find(name).with_context(|| format!("no asset {name:?} in the bundle"))?;
        let view = AssetView { bundle: self, object: &asset.object };
        let actual = view.class();
        if actual != class {
            bail!("asset {name:?} is a {actual}, expected {class}");
        }
        Ok(view)
    }

    fn by_guid(&self, guid: Guid) -> Option<&Object> {
        self.guids.get(&guid).map(|&i| &self.stream.assets[i].object)
    }

    pub fn object_by_guid(&self, guid: Guid) -> Option<&Object> {
        self.by_guid(guid)
    }
}

#[derive(Clone, Copy)]
pub struct AssetView<'a> {
    pub bundle: &'a Bundle,
    pub object: &'a Object,
}

impl<'a> AssetView<'a> {
    pub fn name(&self) -> &'a str {
        self.object.name(self.bundle.schema()).unwrap_or("")
    }

    pub fn class(&self) -> &'a str {
        &self.object.type_desc(self.bundle.schema()).name
    }

    pub fn value(&self, field: &str) -> Result<&'a Value> {
        self.object
            .get(self.bundle.schema(), field)
            .ok_or_else(|| anyhow!("{} {:?} has no field {field}", self.class(), self.name()))
    }

    pub fn f32(&self, field: &str) -> Result<f32> {
        match self.value(field)? {
            Value::Float(x) => Ok(*x),
            other => bail!("{}.{field} is {other:?}, expected a Float", self.name()),
        }
    }

    pub fn int(&self, field: &str) -> Result<i64> {
        let v = self.value(field)?;
        v.as_i64().ok_or_else(|| anyhow!("{}.{field} is {v:?}, expected an integer", self.name()))
    }

    pub fn asset_opt(&self, field: &str) -> Result<Option<AssetView<'a>>> {
        let guid = match self.value(field)? {
            Value::Guid(g) => *g,
            other => bail!("{}.{field} is {other:?}, expected a Guid reference", self.name()),
        };
        if guid.is_null() {
            return Ok(None);
        }
        let object = self
            .bundle
            .by_guid(guid)
            .ok_or_else(|| anyhow!("{}.{field} refers to an asset outside this bundle", self.name()))?;
        Ok(Some(AssetView { bundle: self.bundle, object }))
    }

    pub fn asset(&self, field: &str) -> Result<AssetView<'a>> {
        self.asset_opt(field)?.ok_or_else(|| anyhow!("{}.{field} is a null reference", self.name()))
    }

    pub fn curve(&self, field: &str) -> Result<Curve> {
        self.asset(field)?.as_curve()
    }

    pub fn as_curve(&self) -> Result<Curve> {
        if self.class() != "CurveAsset" {
            bail!("{:?} is a {}, expected a CurveAsset", self.name(), self.class());
        }
        let points = match self.value("RuntimeControlPoints")? {
            Value::Array(items) => items
                .iter()
                .map(|v| match v {
                    Value::Vector2(p) => Ok(*p),
                    other => bail!("{}: control point {other:?}", self.name()),
                })
                .collect::<Result<Vec<_>>>()?,
            other => bail!("{}.RuntimeControlPoints is {other:?}", self.name()),
        };
        Ok(Curve { name: self.name().to_string(), points })
    }

    pub fn asset_list(&self, field: &str) -> Result<Vec<AssetView<'a>>> {
        let Value::Array(items) = self.value(field)? else {
            bail!("{}.{field} is not an array", self.name());
        };
        items
            .iter()
            .map(|v| {
                let guid = v.as_guid().ok_or_else(|| anyhow!("{}.{field}: {v:?} is not a Guid", self.name()))?;
                let object = self.bundle.by_guid(guid).ok_or_else(|| anyhow!("{}.{field}: unresolved", self.name()))?;
                Ok(AssetView { bundle: self.bundle, object })
            })
            .collect()
    }

    pub fn floats(&self) -> Vec<(&'a str, f32)> {
        let schema = self.bundle.schema();
        let mut out = Vec::new();
        let mut o = Some(self.object);
        while let Some(obj) = o {
            for (f, v) in obj.body.fields(schema) {
                if let Value::Float(x) = v {
                    out.push((f.name.as_str(), *x));
                }
            }
            o = obj.base(schema);
        }
        out
    }
}
