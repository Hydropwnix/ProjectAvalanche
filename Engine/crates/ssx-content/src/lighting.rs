use std::collections::BTreeMap;

use crate::id::{ContentId, Kind};
use crate::manifest::line_col;
use crate::material::Problem;

pub const SKY_KEY: &str = "sky";
pub const SUN_NODE_KEY: &str = "sun_node";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FieldType {
    Float,
    Vector4,
    Texture,
}

impl FieldType {
    pub fn as_str(self) -> &'static str {
        match self {
            FieldType::Float => "a number",
            FieldType::Vector4 => "four numbers",
            FieldType::Texture => "a texture id",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LightingRecord {
    pub floats: BTreeMap<String, f32>,
    pub vectors: BTreeMap<String, [f32; 4]>,
    pub textures: BTreeMap<String, ContentId>,
    pub sky: Option<ContentId>,
    pub sun_node: Option<u32>,
}

impl LightingRecord {
    pub fn fields(&self) -> impl Iterator<Item = (&str, FieldType)> {
        let f = self.floats.keys().map(|k| (k.as_str(), FieldType::Float));
        let v = self.vectors.keys().map(|k| (k.as_str(), FieldType::Vector4));
        let t = self.textures.keys().map(|k| (k.as_str(), FieldType::Texture));
        f.chain(v).chain(t)
    }

    pub fn to_table(&self) -> toml::Table {
        let mut t = toml::Table::new();
        for (k, v) in &self.floats {
            t.insert(k.clone(), toml::Value::Float(f64::from(*v)));
        }
        for (k, v) in &self.vectors {
            t.insert(k.clone(), toml::Value::Array(v.iter().map(|x| toml::Value::Float(f64::from(*x))).collect()));
        }
        for (k, v) in &self.textures {
            t.insert(k.clone(), toml::Value::String(v.to_string()));
        }
        if let Some(s) = &self.sky {
            t.insert(SKY_KEY.into(), toml::Value::String(s.to_string()));
        }
        if let Some(n) = self.sun_node {
            t.insert(SUN_NODE_KEY.into(), toml::Value::String(format!("0x{n:08x}")));
        }
        t
    }

    pub fn from_table(t: &toml::Table) -> Result<LightingRecord, Vec<String>> {
        let mut r = LightingRecord::default();
        let mut problems = Vec::new();
        for (k, v) in t {
            if let Err(e) = r.set(k, v) {
                problems.push(e);
            }
        }
        if problems.is_empty() { Ok(r) } else { Err(problems) }
    }

    fn set(&mut self, key: &str, v: &toml::Value) -> Result<(), String> {
        let finite = |x: f64| x.is_finite() && x.abs() <= f64::from(f32::MAX);
        match key {
            SKY_KEY => {
                let id = id_of(v, Kind::Sky).map_err(|e| format!("{key}: {e}"))?;
                self.sky = Some(id);
                return Ok(());
            }
            SUN_NODE_KEY => {
                let s = v.as_str().ok_or_else(|| format!("{key}: a node's key as \"0x<8 hex digits>\""))?;
                self.sun_node = Some(
                    hex_key(s)
                        .ok_or_else(|| format!("{key}: {} is not \"0x<8 hex digits>\"", ssx_core::text::quoted(s)))?,
                );
                return Ok(());
            }
            _ => {}
        }
        if !field_name(key) {
            return Err(format!(
                "{}: not a field name (letters, digits and `_`, at most 64; an unnamed field as \"0x<8 hex digits>\")",
                ssx_core::text::quoted(key)
            ));
        }
        match v {
            toml::Value::Float(x) if finite(*x) => {
                self.floats.insert(key.into(), *x as f32);
            }
            toml::Value::Integer(i) if (i.unsigned_abs()) < (1 << 24) => {
                self.floats.insert(key.into(), *i as f32);
            }
            toml::Value::Array(a) => {
                let xs: Option<Vec<f64>> = a.iter().map(number).collect();
                match xs.filter(|x| x.len() == 4 && x.iter().all(|v| finite(*v))) {
                    Some(x) => {
                        self.vectors.insert(key.into(), [x[0] as f32, x[1] as f32, x[2] as f32, x[3] as f32]);
                    }
                    None => return Err(format!("{key}: a vector is four finite numbers")),
                }
            }
            toml::Value::String(_) => {
                let id = id_of(v, Kind::Texture).map_err(|e| format!("{key}: {e}"))?;
                self.textures.insert(key.into(), id);
            }
            _ => return Err(format!("{key}: a finite number, four numbers or a texture id")),
        }
        Ok(())
    }
}

fn number(v: &toml::Value) -> Option<f64> {
    match v {
        toml::Value::Float(x) => Some(*x),
        toml::Value::Integer(i) if i.unsigned_abs() < (1 << 24) => Some(*i as f64),
        _ => None,
    }
}

fn id_of(v: &toml::Value, kind: Kind) -> Result<ContentId, String> {
    let s = v.as_str().ok_or_else(|| format!("a {kind} id"))?;
    let id: ContentId = s.parse().map_err(|e| format!("{e}"))?;
    if id.kind() != kind {
        return Err(format!("{id} is not a {kind} id"));
    }
    Ok(id)
}

pub fn hex_key(s: &str) -> Option<u32> {
    let h = s.strip_prefix("0x")?;
    (h.len() == 8 && h.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)))
        .then(|| u32::from_str_radix(h, 16).ok())
        .flatten()
}

pub fn field_name(s: &str) -> bool {
    hex_key(s).is_some()
        || (!s.is_empty()
            && s.len() <= 64
            && s.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
}

pub fn parse_toml(bytes: &[u8]) -> Result<LightingRecord, Vec<Problem>> {
    let table = crate::package::parse_record_at(bytes).map_err(|p| vec![p])?;
    let text = std::str::from_utf8(bytes).map_err(|e| vec![(0, 0, format!("not UTF-8: {e}"))])?;
    let spans: BTreeMap<String, std::ops::Range<usize>> =
        toml::from_str::<BTreeMap<toml::Spanned<String>, toml::Value>>(text)
            .map(|m| m.into_keys().map(|k| (k.get_ref().clone(), k.span())).collect())
            .unwrap_or_default();
    let mut r = LightingRecord::default();
    let mut problems = Vec::new();
    for (k, v) in &table {
        if let Err(e) = r.set(k, v) {
            let (line, col) = spans.get(k).map_or((0, 0), |s| line_col(text, s.start));
            problems.push((line, col, e));
        }
    }
    if problems.is_empty() {
        Ok(r)
    } else {
        problems.sort();
        Err(problems)
    }
}

pub fn misfit(name: &str, ty: FieldType, schema: &BTreeMap<String, FieldType>) -> Option<String> {
    match schema.get(name) {
        Some(t) if *t == ty => None,
        Some(t) => Some(format!("{name} is {} on the disc's presets, not {}", t.as_str(), ty.as_str())),
        None => Some(format!("{name}: no lighting preset of the disc has this field")),
    }
}

pub fn key_lines(bytes: &[u8]) -> BTreeMap<String, (usize, usize)> {
    let Ok(text) = std::str::from_utf8(bytes) else { return BTreeMap::new() };
    toml::from_str::<BTreeMap<toml::Spanned<String>, toml::Value>>(text)
        .map(|m| m.into_keys().map(|k| (k.get_ref().clone(), line_col(text, k.span().start))).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lighting_record_reads_round_trips_and_refuses_with_lines() {
        let text = br#"rampIntensity = 1.25
dirLight_Direction = [0.1, 0.9, 0.2, 1.0]
cloudDensityTexture = "ssx:texture/worlds/textures/sky_cloud_density"
0xceafcae0 = 0.6
sky = "aurora:sky/night"
sun_node = "0x01c45447"
"#;
        let r = parse_toml(text).unwrap();
        assert_eq!(r.floats["rampIntensity"], 1.25);
        assert_eq!(r.floats["0xceafcae0"], 0.6);
        assert_eq!(r.vectors["dirLight_Direction"], [0.1, 0.9, 0.2, 1.0]);
        assert_eq!(r.sky.as_ref().unwrap().as_str(), "aurora:sky/night");
        assert_eq!(r.sun_node, Some(0x01c4_5447));
        assert_eq!(LightingRecord::from_table(&r.to_table()).unwrap(), r, "a record survives its table");
        let bad = br#"rampIntensity = "x"
dirLight_Direction = [0.1, 0.9]
sky = "a:texture/x"
"bad name" = 1.0
fog_data = nan
sun_node = "123"
"#;
        let p = parse_toml(bad).unwrap_err();
        let lines: Vec<usize> = p.iter().map(|(l, _, _)| *l).collect();
        assert_eq!(lines, [1, 2, 3, 4, 5, 6], "{p:?}");
        let schema: BTreeMap<String, FieldType> = [("rampIntensity".to_string(), FieldType::Float)].into();
        assert!(misfit("rampIntensity", FieldType::Vector4, &schema).unwrap().contains("a number on the disc"));
        assert!(misfit("rampIntensty", FieldType::Float, &schema).unwrap().contains("no lighting preset"));
        assert_eq!(misfit("rampIntensity", FieldType::Float, &schema), None);
    }
}
