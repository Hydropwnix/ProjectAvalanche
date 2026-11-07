mod cube;
mod exr;
mod hdr;

use std::sync::Arc;

use serde::Deserialize;

use crate::id::{ContentId, Kind, PORT_NAMESPACE};
use crate::manifest::line_col;
use crate::material::Problem;
use crate::model::FileReader;
use crate::texture::{self, TextureSettings};

pub use cube::{CubeLevel, SkyCube, cube_bytes, equirect_texel, equirect_to_cube, face_direction};

pub fn scattering_id() -> ContentId {
    ContentId::new(PORT_NAMESPACE, Kind::Sky, "scattering").expect("a valid id")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorEncoding {
    Srgb,
    Linear,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CubeSource {
    Cube(String),
    Faces([String; 6]),
    Equirect(String),
}

impl CubeSource {
    pub fn files(&self) -> Vec<&str> {
        match self {
            CubeSource::Cube(f) | CubeSource::Equirect(f) => vec![f.as_str()],
            CubeSource::Faces(fs) => fs.iter().map(String::as_str).collect(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SkyKind {
    Scattering,
    Cubemap { source: CubeSource, color: ColorEncoding, face_size: Option<u32> },
    Dome { model: ContentId, material: ContentId },
}

#[derive(Clone, Debug, PartialEq)]
pub struct SkyDef {
    pub name: String,
    pub kind: SkyKind,
    pub tint: [f32; 3],
    pub exposure: f32,
    pub rotation: f32,
    pub sun: bool,
    pub clouds: bool,
    pub midground: bool,
    pub reflection: bool,
}

impl SkyDef {
    pub fn scattering() -> SkyDef {
        SkyDef {
            name: "the game's scattering sky".into(),
            kind: SkyKind::Scattering,
            tint: [1.0; 3],
            exposure: 1.0,
            rotation: 0.0,
            sun: true,
            clouds: true,
            midground: true,
            reflection: false,
        }
    }

    pub fn files(&self) -> Vec<&str> {
        match &self.kind {
            SkyKind::Cubemap { source, .. } => source.files(),
            _ => Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Sky {
    pub def: SkyDef,
    pub cube: Option<Arc<SkyCube>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_file_bytes: u64,
    pub max_equirect_side: u32,
    pub max_source_bytes: u64,
    pub max_face: u32,
    pub max_decoded_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_file_bytes: 256 << 20,
            max_equirect_side: 8192,
            max_source_bytes: 256 << 20,
            max_face: 2048,
            max_decoded_bytes: 512 << 20,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SkyCost {
    pub decoded_bytes: u64,
    pub face: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSky {
    #[serde(rename = "type")]
    kind: toml::Spanned<String>,
    cube: Option<toml::Spanned<String>>,
    faces: Option<toml::Spanned<Vec<String>>>,
    equirect: Option<toml::Spanned<String>>,
    color: Option<ColorEncoding>,
    face_size: Option<toml::Spanned<i64>>,
    model: Option<toml::Spanned<String>>,
    material: Option<toml::Spanned<String>>,
    tint: Option<toml::Spanned<Vec<f64>>>,
    exposure: Option<toml::Spanned<f64>>,
    rotation: Option<toml::Spanned<f64>>,
    sun: Option<bool>,
    clouds: Option<bool>,
    midground: Option<bool>,
    reflection: Option<bool>,
}

const CUBE_FILES: [&str; 2] = ["ktx2", "dds"];
const FACE_FILES: [&str; 3] = ["png", "jpg", "jpeg"];
const EQUIRECT_FILES: [&str; 2] = ["hdr", "exr"];

fn ext(p: &str) -> String {
    p.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

pub fn parse_toml(bytes: &[u8], name: &str) -> Result<SkyDef, Vec<Problem>> {
    crate::package::parse_record_at(bytes).map_err(|p| vec![p])?;
    let text = std::str::from_utf8(bytes).map_err(|e| vec![(0, 0, format!("not UTF-8: {e}"))])?;
    let raw: RawSky = toml::from_str(text).map_err(|e| {
        let (line, column) = e.span().map_or((0, 0), |s| line_col(text, s.start));
        vec![(line, column, ssx_core::text::message(e.message()).into_string())]
    })?;
    let mut problems = Vec::new();
    let mut at = |span: std::ops::Range<usize>, m: String| {
        let (line, column) = line_col(text, span.start);
        problems.push((line, column, m));
    };
    let path = |s: &toml::Spanned<String>, exts: &[&str], at: &mut dyn FnMut(std::ops::Range<usize>, String)| {
        let p = s.get_ref();
        if let Err(e) = crate::path::check_package_path(p, false) {
            at(s.span(), e.to_string());
        } else if !exts.contains(&ext(p).as_str()) {
            let list = exts.iter().map(|e| format!(".{e}")).collect::<Vec<_>>().join(" ");
            at(s.span(), format!("{}: not one of {list}", ssx_core::text::quoted(p)));
        }
        p.clone()
    };
    let finite = |x: f64| x.is_finite() && x.abs() <= f64::from(f32::MAX);
    let tint = match &raw.tint {
        Some(t) if t.get_ref().len() == 3 && t.get_ref().iter().all(|x| finite(*x) && *x >= 0.0) => {
            [t.get_ref()[0] as f32, t.get_ref()[1] as f32, t.get_ref()[2] as f32]
        }
        Some(t) => {
            at(t.span(), "tint: three finite numbers of at least 0".into());
            [1.0; 3]
        }
        None => [1.0; 3],
    };
    let exposure = match &raw.exposure {
        Some(e) if finite(*e.get_ref()) && *e.get_ref() >= 0.0 => *e.get_ref() as f32,
        Some(e) => {
            at(e.span(), "exposure: a finite number of at least 0".into());
            1.0
        }
        None => 1.0,
    };
    let rotation = match &raw.rotation {
        Some(r) if finite(*r.get_ref()) && r.get_ref().abs() <= 360.0 => *r.get_ref() as f32,
        Some(r) => {
            at(r.span(), "rotation: degrees from -360 to 360".into());
            0.0
        }
        None => 0.0,
    };
    let only =
        |what: &str, given: bool, span: std::ops::Range<usize>, at: &mut dyn FnMut(std::ops::Range<usize>, String)| {
            if given {
                at(span, what.to_string());
            }
        };
    let kind_span = raw.kind.span();
    let kind = match raw.kind.get_ref().as_str() {
        "scattering" => {
            at(
                kind_span.clone(),
                "type \"scattering\" is the game's own sky (`ava:sky/scattering`): name it, do not define it".into(),
            );
            SkyKind::Scattering
        }
        "cubemap" => {
            let sources: Vec<CubeSource> = [
                raw.cube.as_ref().map(|c| CubeSource::Cube(path(c, &CUBE_FILES, &mut at))),
                raw.equirect.as_ref().map(|e| CubeSource::Equirect(path(e, &EQUIRECT_FILES, &mut at))),
                raw.faces.as_ref().map(|f| {
                    let list = f.get_ref();
                    if list.len() != 6 {
                        at(f.span(), format!("faces: six files (+X, -X, +Y, -Y, +Z, -Z), not {}", list.len()));
                    }
                    let mut six: [String; 6] = Default::default();
                    for (k, p) in list.iter().take(6).enumerate() {
                        let spanned = toml::Spanned::new(f.span(), p.clone());
                        six[k] = path(&spanned, &FACE_FILES, &mut at);
                    }
                    CubeSource::Faces(six)
                }),
            ]
            .into_iter()
            .flatten()
            .collect();
            if sources.len() != 1 {
                at(kind_span.clone(), "a cubemap sky names one of `cube`, `faces`, `equirect`".into());
            }
            let source = sources.into_iter().next().unwrap_or(CubeSource::Cube(String::new()));
            let face_size = match &raw.face_size {
                Some(s) if matches!(source, CubeSource::Equirect(_)) => match u32::try_from(*s.get_ref()) {
                    Ok(v) if (1..=8192).contains(&v) && v.is_power_of_two() => Some(v),
                    _ => {
                        at(s.span(), "face_size: a power of two from 1 to 8192".into());
                        None
                    }
                },
                Some(s) => {
                    at(s.span(), "face_size: only an equirectangular image is converted".into());
                    None
                }
                None => None,
            };
            if let (Some(_), CubeSource::Equirect(_)) = (&raw.color, &source) {
                at(
                    kind_span.clone(),
                    "color: HDR and EXR images are linear (the setting is for 8-bit faces and cubes)".into(),
                );
            }
            for (what, given) in [("model", raw.model.is_some()), ("material", raw.material.is_some())] {
                only(&format!("{what}: a dome's key (type = \"dome\")"), given, kind_span.clone(), &mut at);
            }
            SkyKind::Cubemap { source, color: raw.color.unwrap_or(ColorEncoding::Srgb), face_size }
        }
        "dome" => {
            let id = |s: &Option<toml::Spanned<String>>,
                      kind: Kind,
                      what: &str,
                      at: &mut dyn FnMut(std::ops::Range<usize>, String)| {
                match s {
                    Some(s) => match s.get_ref().parse::<ContentId>() {
                        Ok(id) if id.kind() == kind => Some(id),
                        Ok(id) => {
                            at(s.span(), format!("{what}: {id} is not a {kind} id"));
                            None
                        }
                        Err(e) => {
                            at(s.span(), format!("{what}: {e}"));
                            None
                        }
                    },
                    None => {
                        at(kind_span.clone(), format!("a dome names its {what}"));
                        None
                    }
                }
            };
            let model = id(&raw.model, Kind::Model, "model", &mut at);
            let material = id(&raw.material, Kind::Material, "material", &mut at);
            for (what, given) in [
                ("cube", raw.cube.is_some()),
                ("faces", raw.faces.is_some()),
                ("equirect", raw.equirect.is_some()),
                ("face_size", raw.face_size.is_some()),
                ("tint", raw.tint.is_some()),
                ("exposure", raw.exposure.is_some()),
                ("reflection", raw.reflection.is_some()),
            ] {
                only(
                    &format!("{what}: not a dome's key (its look is its material's)"),
                    given,
                    kind_span.clone(),
                    &mut at,
                );
            }
            match (model, material) {
                (Some(model), Some(material)) => SkyKind::Dome { model, material },
                _ => SkyKind::Scattering,
            }
        }
        other => {
            at(kind_span.clone(), format!("type {}: \"cubemap\" or \"dome\"", ssx_core::text::quoted(other)));
            SkyKind::Scattering
        }
    };
    if !problems.is_empty() {
        problems.sort();
        return Err(problems);
    }
    let reflection = raw.reflection.unwrap_or(false);
    Ok(SkyDef {
        name: name.to_string(),
        kind,
        tint,
        exposure,
        rotation,
        sun: raw.sun.unwrap_or(true),
        clouds: raw.clouds.unwrap_or(false),
        midground: raw.midground.unwrap_or(true),
        reflection,
    })
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{}", ssx_core::text::located(.file, .message))]
pub struct SkyError {
    pub file: String,
    pub message: String,
}

impl SkyError {
    pub fn new(file: &str, message: impl Into<String>) -> Self {
        SkyError { file: file.to_string(), message: message.into() }
    }
}

fn face_limits(limits: &Limits) -> texture::Limits {
    texture::Limits {
        max_file_bytes: limits.max_file_bytes,
        max_side: limits.max_face,
        max_decoded_bytes: limits.max_decoded_bytes,
        ..texture::Limits::default()
    }
}

fn equirect_face(width: u32, face_size: Option<u32>, limits: &Limits) -> Result<u32, String> {
    let face = face_size.unwrap_or_else(|| {
        let q = (width / 4).max(1);
        1 << (31 - q.leading_zeros())
    });
    if face > limits.max_face {
        return Err(format!("a face of {face}, larger than {}", limits.max_face));
    }
    Ok(face)
}

pub fn probe(def: &SkyDef, files: &dyn FileReader, limits: &Limits) -> Result<SkyCost, SkyError> {
    let SkyKind::Cubemap { source, face_size, .. } = &def.kind else { return Ok(SkyCost::default()) };
    let head = |rel: &str, n: u64| -> Result<(u64, Vec<u8>), SkyError> {
        let size = files.size(rel).map_err(|e| SkyError::new(rel, e))?;
        if size > limits.max_file_bytes {
            return Err(SkyError::new(rel, format!("{size} bytes, over the cap of {}", limits.max_file_bytes)));
        }
        let bytes = files.read_range(rel, 0, size.min(n)).map_err(|e| SkyError::new(rel, e))?;
        Ok((size, bytes))
    };
    let tex = face_limits(limits);
    let (source_bytes, face) = match source {
        CubeSource::Equirect(rel) => {
            let (_, bytes) = head(rel, 1 << 20)?;
            let (w, h) = match ext(rel).as_str() {
                "hdr" => hdr::size(&bytes).map_err(|e| SkyError::new(rel, e))?,
                _ => exr::size(&bytes).map_err(|e| SkyError::new(rel, e))?,
            };
            check_source(w, h, limits).map_err(|e| SkyError::new(rel, e))?;
            let face = equirect_face(w, *face_size, limits).map_err(|e| SkyError::new(rel, e))?;
            (u64::from(w) * u64::from(h) * 12, face)
        }
        CubeSource::Cube(rel) => {
            let size = files.size(rel).map_err(|e| SkyError::new(rel, e))?;
            let mut read = |len: u64| files.read_range(rel, 0, size.min(len));
            let p = texture::probe_file(rel, size, &mut read, &tex).map_err(|e| SkyError::new(rel, e.message))?;
            if p.faces != 6 || p.width != p.height {
                return Err(SkyError::new(rel, format!("not a cube ({}×{}, {} faces)", p.width, p.height, p.faces)));
            }
            (p.decoded_bytes(), p.width)
        }
        CubeSource::Faces(six) => {
            let mut face = None;
            let mut bytes = 0u64;
            for rel in six {
                let size = files.size(rel).map_err(|e| SkyError::new(rel, e))?;
                let mut read = |len: u64| files.read_range(rel, 0, size.min(len));
                let p = texture::probe_file(rel, size, &mut read, &tex).map_err(|e| SkyError::new(rel, e.message))?;
                if p.width != p.height || face.is_some_and(|f| f != p.width) {
                    return Err(SkyError::new(
                        rel,
                        format!("{}×{}: the faces are squares of one size", p.width, p.height),
                    ));
                }
                face = Some(p.width);
                bytes += p.decoded_bytes();
            }
            (bytes, face.unwrap_or(0))
        }
    };
    let decoded_bytes = source_bytes + cube_bytes(face);
    if decoded_bytes > limits.max_decoded_bytes {
        return Err(SkyError::new(
            &def.name,
            format!(
                "{decoded_bytes} bytes decoded (sources and the cube with mips), over the cap of {}",
                limits.max_decoded_bytes
            ),
        ));
    }
    Ok(SkyCost { decoded_bytes, face })
}

pub(crate) fn check_source(w: u32, h: u32, limits: &Limits) -> Result<(), String> {
    let max = limits.max_equirect_side;
    if w == 0 || h == 0 || w > max || h > max {
        return Err(format!("{w}×{h}: an equirectangular image is 1 to {max} a side"));
    }
    let bytes = u64::from(w) * u64::from(h) * 12;
    if bytes > limits.max_source_bytes {
        return Err(format!("{bytes} bytes as f32 RGB, over the cap of {}", limits.max_source_bytes));
    }
    Ok(())
}

pub fn decode(def: &SkyDef, files: &dyn FileReader, limits: &Limits) -> Result<Option<SkyCube>, SkyError> {
    let SkyKind::Cubemap { source, color, face_size } = &def.kind else { return Ok(None) };
    probe(def, files, limits)?;
    let tex = face_limits(limits);
    let read = |rel: &str| files.read(rel, limits.max_file_bytes).map_err(|e| SkyError::new(rel, e));
    let settings = TextureSettings::default();
    let cube = match source {
        CubeSource::Equirect(rel) => {
            let bytes = read(rel)?;
            let image = match ext(rel).as_str() {
                "hdr" => hdr::decode(&bytes, limits).map_err(|e| SkyError::new(rel, e))?,
                _ => exr::decode(&bytes, limits).map_err(|e| SkyError::new(rel, e))?,
            };
            let face = equirect_face(image.width, *face_size, limits).map_err(|e| SkyError::new(rel, e))?;
            equirect_to_cube(&image, face)
        }
        CubeSource::Cube(rel) => {
            let t =
                texture::decode_file(rel, &read(rel)?, &settings, &tex).map_err(|e| SkyError::new(rel, e.message))?;
            if t.shape != texture::Shape::Cube {
                return Err(SkyError::new(rel, "not a cube"));
            }
            cube::from_rgba8_levels(&t.levels, *color).map_err(|e| SkyError::new(rel, e))?
        }
        CubeSource::Faces(six) => {
            let mut faces = Vec::with_capacity(6);
            for rel in six {
                let t = texture::decode_file(rel, &read(rel)?, &settings, &tex)
                    .map_err(|e| SkyError::new(rel, e.message))?;
                let level0 = t.levels.into_iter().next().ok_or_else(|| SkyError::new(rel, "no image"))?;
                faces.push(level0);
            }
            cube::from_faces(faces, *color).map_err(|e| SkyError::new(&def.name, e))?
        }
    };
    Ok(Some(cube))
}

#[derive(Clone, Debug)]
pub struct FloatImage {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<[f32; 3]>,
}

#[cfg(test)]
pub(crate) mod tests;
