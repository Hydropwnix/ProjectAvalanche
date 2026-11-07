use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use ssx_content::Kind;
use ssx_content::data::schema::{Class, FieldSchema, FieldType, RecordSchema, Scalar};

use super::LIGHTING_SIM_FIELDS;
use crate::content::GameContent;

fn field(ty: FieldType, class: Class, source: &str) -> FieldSchema {
    FieldSchema { ty, class, source: source.into(), note: String::new(), optional: true }
}

fn float() -> FieldType {
    let m = f64::from(f32::MAX);
    FieldType::Scalar(Scalar::Float { min: -m, max: m, f32: true })
}

const FINITE: &str = "any finite 32-bit float (the record's reader)";
const SIM_SOURCE: &str = "ssx-physics world/spawn.rs 594-602: the sun normalised by try_normalize (a zero or \
                          non-finite direction: no sun), the night flag compared > 0, the lamp's scale a factor";

pub fn lighting(content: &GameContent) -> Result<RecordSchema, String> {
    use ssx_content::lighting::FieldType as L;
    let mut rs = RecordSchema::new("lighting");
    for (name, ty) in content.lighting_schema()?.iter() {
        let (class, source) = if LIGHTING_SIM_FIELDS.contains(&name.as_str()) {
            (Class::Simulation, SIM_SOURCE)
        } else {
            (Class::Presentation, FINITE)
        };
        let ty = match ty {
            L::Float => float(),
            L::Vector4 => FieldType::Tuple(vec![float(); 4]),
            L::Texture => FieldType::Scalar(Scalar::Id(Kind::Texture)),
        };
        rs.fields.insert(name.clone(), field(ty, class, source));
    }
    let sky = field(FieldType::Scalar(Scalar::Id(Kind::Sky)), Class::Presentation, "the sky it draws");
    rs.fields.insert(ssx_content::lighting::SKY_KEY.into(), sky);
    let node = FieldType::Scalar(Scalar::Text { max: 10, one_of: Vec::new() });
    rs.fields
        .insert(ssx_content::lighting::SUN_NODE_KEY.into(), field(node, Class::Presentation, "\"0x<8 hex digits>\""));
    Ok(rs)
}

pub fn postfx(big_root: &Path) -> Result<RecordSchema> {
    let ids = super::super::records::record_ids(big_root, &[(super::super::records::CLASS_POSTFX, Kind::Postfx)])?;
    let mut floats = std::collections::BTreeSet::new();
    let mut vectors: BTreeMap<String, std::collections::BTreeSet<usize>> = BTreeMap::new();
    for (&(_, key), id) in &ids {
        let p = crate::postfx::load_preset_by_key(big_root, key, id.path())?;
        floats.extend(p.floats.keys().cloned());
        for (k, v) in &p.vectors {
            vectors.entry(k.clone()).or_default().insert(v.len());
        }
    }
    let p = Class::Presentation;
    let mut rs = RecordSchema::new("postfx");
    for name in floats {
        rs.fields.insert(name, field(float(), p, FINITE));
    }
    for (name, lens) in vectors {
        let ty = match (lens.first(), lens.last()) {
            (Some(&a), Some(&b)) if a == b => FieldType::Tuple(vec![float(); a]),
            _ => FieldType::List { elem: Box::new(float()), min: 2, max: 4, ascending: false },
        };
        rs.fields.insert(name, field(ty, p, "the disc's presets' length"));
    }
    let name = || FieldType::Scalar(Scalar::Text { max: 63, one_of: Vec::new() });
    rs.fields.insert(ssx_content::postfx::LAYER_KEY.into(), field(name(), p, "a postfx_layers name"));
    let mut blend = RecordSchema::new("ColourCubeBlend");
    let int = |max| FieldType::Scalar(Scalar::Int { min: 0, max });
    for (k, ty) in [
        ("stack", int(255)),
        ("mode", int(2)),
        ("group", name()),
        ("weight", FieldType::Scalar(Scalar::Float { min: 0.0, max: 1.0, f32: false })),
    ] {
        let mut f = field(ty, p, "");
        f.optional = false;
        blend.fields.insert(k.into(), f);
    }
    let blends =
        FieldType::List { elem: Box::new(FieldType::Table(Box::new(blend))), min: 0, max: 64, ascending: false };
    rs.fields.insert(ssx_content::postfx::BLENDS_KEY.into(), field(blends, p, "the colour-cube stacks' groups"));
    let cube = field(FieldType::Scalar(Scalar::Id(Kind::Postfx)), p, "a 1024 × 32 PNG strip");
    rs.fields.insert(ssx_content::postfx::CUBE_KEY.into(), cube);
    Ok(rs)
}
