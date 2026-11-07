pub mod shapes;

use serde::Deserialize;

use crate::id::{ContentId, Kind, PORT_NAMESPACE};
use crate::manifest::line_col;
use crate::material::Problem;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Shape {
    Ring,
    Cylinder,
    Arrow,
    Chevron,
    Beam,
    Gate,
}

impl Shape {
    pub const ALL: [Shape; 6] = [Shape::Ring, Shape::Cylinder, Shape::Arrow, Shape::Chevron, Shape::Beam, Shape::Gate];

    pub fn name(self) -> &'static str {
        match self {
            Shape::Ring => "ring",
            Shape::Cylinder => "cylinder",
            Shape::Arrow => "arrow",
            Shape::Chevron => "chevron",
            Shape::Beam => "beam",
            Shape::Gate => "gate",
        }
    }

    pub fn parse(s: &str) -> Option<Shape> {
        Shape::ALL.into_iter().find(|k| k.name() == s)
    }

    pub fn id(self) -> ContentId {
        ContentId::new(PORT_NAMESPACE, Kind::Marker, self.name()).expect("a valid id")
    }

    pub fn default_hit(self) -> Hit {
        match self {
            Shape::Ring => Hit::Ring,
            Shape::Gate => Hit::Gate,
            Shape::Cylinder => Hit::Cylinder,
            Shape::Arrow | Shape::Chevron | Shape::Beam => Hit::Sphere,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    None,
    Sphere,
    Cylinder,
    Ring,
    Gate,
}

impl Hit {
    pub fn parse(s: &str) -> Option<Hit> {
        Some(match s {
            "none" => Hit::None,
            "sphere" => Hit::Sphere,
            "cylinder" => Hit::Cylinder,
            "ring" => Hit::Ring,
            "gate" => Hit::Gate,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Look {
    Shape(Shape),
    Model(ContentId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Facing {
    World,
    Camera,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Animation {
    pub spin: f32,
    pub bob: f32,
    pub bob_period: f32,
    pub pulse: f32,
    pub pulse_period: f32,
}

impl Default for Animation {
    fn default() -> Self {
        Animation { spin: 0.0, bob: 0.0, bob_period: 2.0, pulse: 0.0, pulse_period: 1.0 }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MarkerDef {
    pub look: Look,
    pub size: f32,
    pub colour: u32,
    pub glow: f32,
    pub facing: Facing,
    pub draw_distance: f32,
    pub icon: Option<ContentId>,
    pub sound: Option<ContentId>,
    pub hit: Hit,
    pub animation: Animation,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawAnimation {
    spin: Option<toml::Spanned<f64>>,
    bob: Option<toml::Spanned<f64>>,
    bob_period: Option<toml::Spanned<f64>>,
    pulse: Option<toml::Spanned<f64>>,
    pulse_period: Option<toml::Spanned<f64>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawMarker {
    shape: Option<toml::Spanned<String>>,
    model: Option<toml::Spanned<String>>,
    size: Option<toml::Spanned<f64>>,
    color: Option<toml::Spanned<toml::Value>>,
    glow: Option<toml::Spanned<f64>>,
    facing: Option<toml::Spanned<String>>,
    draw_distance: Option<toml::Spanned<f64>>,
    icon: Option<toml::Spanned<String>>,
    sound: Option<toml::Spanned<String>>,
    hit: Option<toml::Spanned<String>>,
    animation: Option<RawAnimation>,
}

fn ranged(
    v: &Option<toml::Spanned<f64>>,
    what: &str,
    (lo, hi): (f64, f64),
    default: f32,
    at: &mut dyn FnMut(std::ops::Range<usize>, String),
) -> f32 {
    match v {
        Some(x) if x.get_ref().is_finite() && (lo..=hi).contains(x.get_ref()) => *x.get_ref() as f32,
        Some(x) => {
            at(x.span(), format!("{what}: a number from {lo} to {hi}"));
            default
        }
        None => default,
    }
}

fn colour(v: &toml::Value) -> Result<u32, String> {
    let bytes: Vec<u8> = match v {
        toml::Value::String(s) => {
            let hex = s.strip_prefix('#').ok_or("a colour starts with #")?;
            if !(hex.len() == 6 || hex.len() == 8) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("a colour is #RRGGBB or #RRGGBBAA".into());
            }
            (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0)).collect()
        }
        toml::Value::Array(a) if a.len() == 3 || a.len() == 4 => a
            .iter()
            .map(|x| x.as_integer().and_then(|n| u8::try_from(n).ok()).ok_or("colour parts are whole numbers 0–255"))
            .collect::<Result<_, _>>()?,
        _ => return Err("a colour is \"#RRGGBB[AA]\" or [r, g, b(, a)]".into()),
    };
    let a = bytes.get(3).copied().unwrap_or(255);
    Ok(u32::from_be_bytes([a, bytes[0], bytes[1], bytes[2]]))
}

fn id_of(
    s: &toml::Spanned<String>,
    kind: Kind,
    what: &str,
    at: &mut dyn FnMut(std::ops::Range<usize>, String),
) -> Option<ContentId> {
    match s.get_ref().parse::<ContentId>() {
        Ok(id) if id.kind() == kind => Some(id),
        Ok(id) => {
            at(s.span(), format!("{what}: {id} is not a {kind} id"));
            None
        }
        Err(e) => {
            at(s.span(), format!("{what}: {e}"));
            None
        }
    }
}

pub fn parse_toml(bytes: &[u8]) -> Result<MarkerDef, Vec<Problem>> {
    crate::package::parse_record_at(bytes).map_err(|p| vec![p])?;
    let text = std::str::from_utf8(bytes).map_err(|e| vec![(0, 0, format!("not UTF-8: {e}"))])?;
    let raw: RawMarker = toml::from_str(text).map_err(|e| {
        let (line, column) = e.span().map_or((0, 0), |s| line_col(text, s.start));
        vec![(line, column, ssx_core::text::message(e.message()).into_string())]
    })?;
    let mut problems = Vec::new();
    let mut at = |span: std::ops::Range<usize>, m: String| {
        let (line, column) = line_col(text, span.start);
        problems.push((line, column, m));
    };
    let look = match (&raw.shape, &raw.model) {
        (Some(s), None) => match Shape::parse(s.get_ref()) {
            Some(shape) => Some(Look::Shape(shape)),
            None => {
                at(s.span(), "shape: one of ring, cylinder, arrow, chevron, beam, gate".into());
                None
            }
        },
        (None, Some(m)) => id_of(m, Kind::Model, "model", &mut at).map(Look::Model),
        (Some(s), Some(_)) => {
            at(s.span(), "a marker is a `shape` or a `model`, not both".into());
            None
        }
        (None, None) => {
            at(0..0, "a marker names a `shape` or a `model`".into());
            None
        }
    };
    let size = ranged(&raw.size, "size", (0.1, 100.0), 4.0, &mut at);
    let colour = match &raw.color {
        Some(c) => colour(c.get_ref()).unwrap_or_else(|e| {
            at(c.span(), format!("color: {e}"));
            0xFFFF_FFFF
        }),
        None => 0xFFFF_FFFF,
    };
    let glow = ranged(&raw.glow, "glow", (0.0, 8.0), 0.0, &mut at);
    let facing = match raw.facing.as_ref().map(|f| (f.get_ref().as_str(), f.span())) {
        None | Some(("world", _)) => Facing::World,
        Some(("camera", _)) => Facing::Camera,
        Some((_, span)) => {
            at(span, "facing: \"world\" or \"camera\"".into());
            Facing::World
        }
    };
    let draw_distance = ranged(&raw.draw_distance, "draw_distance", (1.0, 5000.0), 500.0, &mut at);
    let icon = raw.icon.as_ref().and_then(|s| id_of(s, Kind::Texture, "icon", &mut at));
    let sound = raw.sound.as_ref().and_then(|s| id_of(s, Kind::Sound, "sound", &mut at));
    let hit = match &raw.hit {
        Some(h) => Hit::parse(h.get_ref()).unwrap_or_else(|| {
            at(h.span(), "hit: one of none, sphere, cylinder, ring, gate".into());
            Hit::Sphere
        }),
        None => match &look {
            Some(Look::Shape(s)) => s.default_hit(),
            _ => Hit::Sphere,
        },
    };
    let animation = match &raw.animation {
        Some(a) => Animation {
            spin: ranged(&a.spin, "animation.spin", (-720.0, 720.0), 0.0, &mut at),
            bob: ranged(&a.bob, "animation.bob", (0.0, 10.0), 0.0, &mut at),
            bob_period: ranged(&a.bob_period, "animation.bob_period", (0.1, 60.0), 2.0, &mut at),
            pulse: ranged(&a.pulse, "animation.pulse", (0.0, 1.0), 0.0, &mut at),
            pulse_period: ranged(&a.pulse_period, "animation.pulse_period", (0.1, 60.0), 1.0, &mut at),
        },
        None => Animation::default(),
    };
    match look {
        Some(look) if problems.is_empty() => {
            Ok(MarkerDef { look, size, colour, glow, facing, draw_distance, icon, sound, hit, animation })
        }
        _ => {
            problems.sort();
            Err(problems)
        }
    }
}

pub fn from_table(table: &toml::Table) -> Result<MarkerDef, Vec<Problem>> {
    let text = toml::to_string(table).map_err(|e| vec![(0, 0, format!("the record cannot be written: {e}"))])?;
    parse_toml(text.as_bytes())
}

#[cfg(test)]
mod tests;
