mod encode;
mod gltf;
mod sample;
pub mod template;
mod toml;
pub mod write;

#[cfg(test)]
mod tests;

pub use encode::{DOMAIN, sim_hash};
pub use gltf::{limits as gltf_limits, read_gltf};
pub use sample::nlerp;
pub use toml::{AnimToml, RawAnim, RawAnimTable, RawFloat, parse_toml};

use crate::model::FileReader;

pub const ANIM_FILE: &str = "anim.toml";

pub const TICKS_PER_SECOND: f32 = 60.0;
pub const MAX_DURATION_S: f32 = 60.0;
pub const MAX_KEYS: usize = 3_601;
pub const MAX_JOINT_TRACKS: usize = 286 * 3;
pub const MAX_FLOAT_TRACKS: usize = 128;
pub const MAX_NUMBERS: usize = 2 << 20;
pub const QUAT_LENGTH_TOLERANCE: f32 = 0.1;
pub const MAX_MAGNITUDE: f32 = 4096.0;
pub const MIN_SCALE: f32 = 1.0 / 1024.0;
pub const MAX_SCALE: f32 = 1024.0;
pub const MAX_NAME_BYTES: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Path {
    Rotation,
    Translation,
    Scale,
}

impl Path {
    pub fn suffix(self) -> char {
        match self {
            Path::Rotation => 'q',
            Path::Translation => 't',
            Path::Scale => 's',
        }
    }

    pub fn width(self) -> usize {
        if self == Path::Rotation { 4 } else { 3 }
    }

    pub fn name(self) -> &'static str {
        match self {
            Path::Rotation => "rotation",
            Path::Translation => "translation",
            Path::Scale => "scale",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Interpolation {
    Step,
    Linear,
    CubicSpline,
}

impl Interpolation {
    pub fn name(self) -> &'static str {
        match self {
            Interpolation::Step => "step",
            Interpolation::Linear => "linear",
            Interpolation::CubicSpline => "cubic",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        [Interpolation::Step, Interpolation::Linear, Interpolation::CubicSpline].into_iter().find(|i| i.name() == s)
    }

    pub fn per_key(self) -> usize {
        if self == Interpolation::CubicSpline { 3 } else { 1 }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Keys {
    pub interpolation: Interpolation,
    pub times: Vec<f32>,
    pub values: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct JointTrack {
    pub joint: String,
    pub path: Path,
    pub keys: Keys,
    pub parent: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FloatTrack {
    pub dof: String,
    pub keys: Keys,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    pub joints: Vec<JointTrack>,
    pub floats: Vec<FloatTrack>,
    pub additive: bool,
    pub duration: f32,
}

impl Clip {
    pub fn joint_at(&self, track: usize, seconds: f32) -> [f32; 4] {
        let t = &self.joints[track];
        let blend = if t.path == Path::Rotation { sample::Blend::Rotation } else { sample::Blend::Vector };
        sample::sample(&t.keys, blend, seconds)
    }

    pub fn float_at(&self, track: usize, seconds: f32) -> f32 {
        sample::sample(&self.floats[track].keys, sample::Blend::Scalar, seconds)[0]
    }

    pub fn ticks(&self) -> u32 {
        (self.duration * TICKS_PER_SECOND).ceil() as u32
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnimError {
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl std::fmt::Display for AnimError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (file, message) = (ssx_core::text::shown_max(&self.file, ssx_core::text::PATH_CHARS), &self.message);
        let line = match self.line {
            0 => ssx_core::text::Text::new(format_args!("{file}: {message}")),
            l => ssx_core::text::Text::new(format_args!("{file}:{l}:{}: {message}", self.column)),
        };
        f.write_str(&line)
    }
}

pub fn read_anim(bytes: &[u8], file: &str, files: &dyn FileReader) -> Result<Clip, Vec<AnimError>> {
    let toml = parse_toml(bytes).map_err(|p| {
        p.into_iter()
            .map(|(line, column, message)| AnimError { file: file.into(), line, column, message })
            .collect::<Vec<_>>()
    })?;
    build(&toml, file, files).map_err(|e| vec![e])
}

pub fn build(toml: &AnimToml, file: &str, files: &dyn FileReader) -> Result<Clip, AnimError> {
    let fail = |m: String| AnimError { file: file.into(), line: 0, column: 0, message: m };
    let joints = match &toml.gltf {
        Some(rel) => {
            let caps = gltf_limits();
            let bytes = files.read(rel, caps.max_file_bytes).map_err(|e| AnimError {
                file: rel.clone(),
                line: 0,
                column: 0,
                message: e,
            })?;
            read_gltf(rel, &bytes, files, toml.animation.as_deref())?
        }
        None => Vec::new(),
    };
    if joints.is_empty() && toml.floats.is_empty() {
        return Err(fail("the clip has no track: name a glTF animation (`gltf`) or give float tracks".into()));
    }
    let numbers: usize = joints.iter().map(|t| t.keys.times.len() + t.keys.values.len()).sum::<usize>()
        + toml.floats.iter().map(|t| t.keys.times.len() + t.keys.values.len()).sum::<usize>();
    if numbers > MAX_NUMBERS {
        return Err(fail(format!("the clip holds {numbers} numbers; at most {MAX_NUMBERS}")));
    }
    let last = |k: &Keys| k.times.last().copied().unwrap_or(0.0);
    let duration =
        joints.iter().map(|t| last(&t.keys)).chain(toml.floats.iter().map(|t| last(&t.keys))).fold(0.0, f32::max);
    Ok(Clip { joints, floats: toml.floats.clone(), additive: toml.additive, duration })
}

pub fn check_keys(keys: &Keys, path: Option<Path>, what: &str) -> Result<(), String> {
    let n = keys.times.len();
    if n == 0 || n > MAX_KEYS {
        return Err(format!("{what}: {n} keys (1 to {MAX_KEYS})"));
    }
    let width = path.map_or(1, Path::width);
    let want = n.checked_mul(width * keys.interpolation.per_key());
    if want != Some(keys.values.len()) {
        return Err(format!(
            "{what}: {} values for {n} keys of {width} components ({} interpolation needs {})",
            keys.values.len(),
            keys.interpolation.name(),
            n * width * keys.interpolation.per_key()
        ));
    }
    let mut prev = None;
    for (k, &t) in keys.times.iter().enumerate() {
        if !t.is_finite() || !(0.0..=MAX_DURATION_S).contains(&t) {
            return Err(format!("{what}: key {k} at {t} s (0 to {MAX_DURATION_S} s)"));
        }
        if prev.is_some_and(|p| t <= p) {
            return Err(format!("{what}: key {k} at {t} s is not after the key before it (times strictly increase)"));
        }
        prev = Some(t);
    }
    let stride = width * keys.interpolation.per_key();
    for (k, key) in keys.values.chunks_exact(stride).enumerate() {
        let value_at = if keys.interpolation == Interpolation::CubicSpline { width } else { 0 };
        if let Some(v) = key.iter().find(|v| !v.is_finite() || v.abs() > MAX_MAGNITUDE * 4.0) {
            return Err(format!("{what}: key {k}: {v} is not a finite number within ±{}", MAX_MAGNITUDE * 4.0));
        }
        let v = &key[value_at..value_at + width];
        match path {
            Some(Path::Rotation) => {
                let len = v.iter().map(|x| x * x).sum::<f32>().sqrt();
                if (len - 1.0).abs() > QUAT_LENGTH_TOLERANCE {
                    return Err(format!(
                        "{what}: key {k}: a rotation of length {len} (a unit quaternion within {QUAT_LENGTH_TOLERANCE})"
                    ));
                }
            }
            Some(Path::Scale) => {
                if let Some(x) = v.iter().find(|x| !(MIN_SCALE..=MAX_SCALE).contains(*x)) {
                    return Err(format!("{what}: key {k}: a scale of {x} ({MIN_SCALE} to {MAX_SCALE})"));
                }
            }
            _ => {
                if let Some(x) = v.iter().find(|x| x.abs() > MAX_MAGNITUDE) {
                    return Err(format!("{what}: key {k}: {x} lies beyond ±{MAX_MAGNITUDE}"));
                }
            }
        }
    }
    Ok(())
}
