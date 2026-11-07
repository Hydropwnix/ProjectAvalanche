use std::collections::BTreeMap;
use std::sync::Arc;

use serde::Deserialize;

use crate::Kind;
use crate::id::ContentId;
use crate::manifest::line_col;
use crate::material::Problem;
use crate::model::Collision;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeshRecord {
    pub geometry: String,
    pub model: Option<ContentId>,
    pub material: Option<ContentId>,
    pub skeleton: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObjectLods {
    Records(Vec<Vec<MeshRecord>>),
    Model { model: ContentId, materials: BTreeMap<String, ContentId>, material: Option<ContentId> },
}

#[derive(Clone, Debug, PartialEq)]
pub enum ObjectCollision {
    None,
    Decoded { path: String, collision: Arc<Collision> },
    Model(ContentId),
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RigidBody {
    pub offset: [f32; 3],
    pub axis: [f32; 3],
    pub half_height: f32,
    pub radius: f32,
    pub fat: f32,
    pub inv_mass: f32,
    pub inv_inertia: [f32; 3],
    pub max_speed: f32,
    pub max_angular_speed: f32,
    pub linear_damping: f32,
    pub angular_damping: f32,
    pub material: [f32; 3],
}

impl RigidBody {
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        let scalars = [
            self.half_height,
            self.radius,
            self.fat,
            self.inv_mass,
            self.max_speed,
            self.max_angular_speed,
            self.linear_damping,
            self.angular_damping,
        ];
        let mut all =
            self.offset.iter().chain(&self.axis).chain(&self.inv_inertia).chain(&self.material).chain(&scalars);
        if all.any(|v| !v.is_finite()) {
            out.push("a value that is not a finite number".into());
        }
        if !(self.half_height > 0.0 && self.radius > 0.0 && self.inv_mass > 0.0) {
            out.push("half_height, radius and inv_mass must be above 0".into());
        }
        if self.fat < 0.0 || self.inv_inertia.iter().any(|&v| v < 0.0) || self.material.iter().any(|&v| v < 0.0) {
            out.push("fat, inv_inertia and material must not be negative".into());
        }
        let len = self.axis.iter().map(|v| v * v).sum::<f32>().sqrt();
        if (len - 1.0).abs() > 1e-3 {
            out.push(format!("axis of length {len}, not 1"));
        }
        out
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioName {
    #[serde(rename = "type")]
    pub type_name: String,
    pub sub_type: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ObjectDef {
    pub name: String,
    pub lod_thresholds: Vec<f32>,
    pub lods: ObjectLods,
    pub collision: ObjectCollision,
    pub controller: Option<ContentId>,
    pub rigid_body: Option<RigidBody>,
    pub audio: Option<AudioName>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawObject {
    model: toml::Spanned<String>,
    lod_thresholds: toml::Spanned<Vec<f64>>,
    material: Option<toml::Spanned<String>>,
    #[serde(default)]
    materials: BTreeMap<toml::Spanned<String>, toml::Spanned<String>>,
    collision: Option<toml::Spanned<String>>,
    controller: Option<toml::Spanned<String>>,
    audio: Option<toml::Spanned<AudioName>>,
    rigid_body: Option<toml::Spanned<RigidBody>>,
}

pub const MAX_LODS: usize = 8;

pub fn parse_toml(bytes: &[u8], name: &str) -> Result<ObjectDef, Vec<Problem>> {
    crate::package::parse_record_at(bytes).map_err(|p| vec![p])?;
    let text = std::str::from_utf8(bytes).map_err(|e| vec![(0, 0, format!("not UTF-8: {e}"))])?;
    let raw: RawObject = toml::from_str(text).map_err(|e| {
        let (line, column) = e.span().map_or((0, 0), |s| line_col(text, s.start));
        vec![(line, column, ssx_core::text::message(e.message()).into_string())]
    })?;
    let mut problems = Vec::new();
    let mut at = |span: std::ops::Range<usize>, m: String| {
        let (line, column) = line_col(text, span.start);
        problems.push((line, column, m));
    };
    let id_of = |s: &toml::Spanned<String>, kind: Kind, at: &mut dyn FnMut(std::ops::Range<usize>, String)| match s
        .get_ref()
        .parse::<ContentId>()
    {
        Ok(id) if id.kind() == kind => Some(id),
        Ok(id) => {
            at(s.span(), format!("{id} is not a {} id", kind.as_str()));
            None
        }
        Err(e) => {
            at(s.span(), e.to_string());
            None
        }
    };
    let model = id_of(&raw.model, Kind::Model, &mut at);
    let t = raw.lod_thresholds.get_ref();
    if t.is_empty() || t.len() > MAX_LODS || t.iter().any(|x| !x.is_finite() || *x < 0.0 || *x > f64::from(f32::MAX)) {
        at(raw.lod_thresholds.span(), format!("lod_thresholds: 1 to {MAX_LODS} finite numbers, none below 0"));
    }
    let material = raw.material.as_ref().and_then(|m| id_of(m, Kind::Material, &mut at));
    let mut materials = BTreeMap::new();
    for (k, v) in &raw.materials {
        if k.get_ref().is_empty() || k.get_ref().len() > 256 {
            at(k.span(), "a glTF material name of 1 to 256 bytes".into());
        } else if let Some(id) = id_of(v, Kind::Material, &mut at) {
            materials.insert(k.get_ref().clone(), id);
        }
    }
    let collision = match &raw.collision {
        Some(c) if c.get_ref() == "none" => Some(ObjectCollision::None),
        Some(c) => id_of(c, Kind::Model, &mut at).map(ObjectCollision::Model),
        None => model.clone().map(ObjectCollision::Model),
    };
    let controller = raw.controller.as_ref().and_then(|c| id_of(c, Kind::Script, &mut at));
    if let Some(a) = &raw.audio {
        let n = a.get_ref();
        let ok = |s: &str| !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
        if !ok(&n.type_name) || !ok(&n.sub_type) {
            at(a.span(), "audio: type and sub_type are names of 1 to 64 letters, digits or _".into());
        }
    }
    if let Some(b) = &raw.rigid_body {
        for p in b.get_ref().problems() {
            at(b.span(), format!("rigid_body: {p}"));
        }
    }
    if !problems.is_empty() {
        return Err(problems);
    }
    let (Some(model), Some(collision)) = (model, collision) else {
        return Err(vec![(0, 0, "no model".into())]);
    };
    Ok(ObjectDef {
        name: name.to_string(),
        lod_thresholds: t.iter().map(|x| *x as f32).collect(),
        lods: ObjectLods::Model { model, materials, material },
        collision,
        controller,
        rigid_body: raw.rigid_body.map(|b| b.into_inner()),
        audio: raw.audio.map(|a| a.into_inner()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_object_toml_reads_and_every_problem_has_its_line() {
        let ok = br#"
model = "rocks:model/boulder"
lod_thresholds = [0.4, 0.2]
[materials]
"Rock" = "ssx:material/rock_basalt_mat"
[audio]
type = "GenericObject"
sub_type = "Rock"
"#;
        let o = parse_toml(ok, "o").unwrap();
        assert_eq!(o.collision, ObjectCollision::Model("rocks:model/boulder".parse().unwrap()), "its model's own");
        assert_eq!(o.lod_thresholds, [0.4, 0.2]);
        let bad = br#"model = "rocks:texture/boulder"
lod_thresholds = [-1.0]
controller = "ssx:model/x"
[materials]
"Rock" = "not an id"
[rigid_body]
offset = [0, 0, 0]
axis = [0, 0, 2]
half_height = 1
radius = 0
fat = 0
inv_mass = 1
inv_inertia = [1, 1, 1]
max_speed = 10
max_angular_speed = 10
linear_damping = 0.1
angular_damping = 0.1
material = [0.2, 0.2, 0.1]
"#;
        let p = parse_toml(bad, "o").unwrap_err();
        let lines: Vec<usize> = p.iter().map(|(l, _, _)| *l).collect();
        assert_eq!(lines, [1, 2, 5, 3, 6, 6], "{p:?}");
        assert!(
            parse_toml(b"model = \"a:model/b\"\nlod_thresholds = [1]\nlods = 3\n", "o").unwrap_err()[0]
                .2
                .contains("unknown field")
        );
    }
}
