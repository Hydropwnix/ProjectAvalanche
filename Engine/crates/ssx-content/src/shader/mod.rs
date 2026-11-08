#[cfg(feature = "shader")]
mod analysis;
#[cfg(feature = "shader")]
mod check;
#[cfg(feature = "shader")]
mod terminate;

use std::collections::BTreeMap;

#[cfg(feature = "shader")]
pub use check::{check_composed, check_hook};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum HookFamily {
    Object,
    Sky,
}

pub const OBJECT_TYPES: &str = include_str!("object_types.wgsl");
pub const SKY_TYPES: &str = include_str!("sky_types.wgsl");
pub const OBJECT_TEMPLATE: &str = include_str!("object_template.wgsl");
pub const SKY_TEMPLATE: &str = include_str!("sky_template.wgsl");
pub const OBJECT_IDENTITY: &str = include_str!("object_identity.wgsl");
pub const SKY_IDENTITY: &str = include_str!("sky_identity.wgsl");

pub const INTERFACE_TYPES: [&str; 3] = ["SurfaceIn", "Surface", "VertexIn"];

impl HookFamily {
    pub fn as_str(self) -> &'static str {
        match self {
            HookFamily::Object => "object",
            HookFamily::Sky => "sky",
        }
    }

    pub fn identity(self) -> &'static str {
        match self {
            HookFamily::Object => OBJECT_IDENTITY,
            HookFamily::Sky => SKY_IDENTITY,
        }
    }

    pub fn types(self) -> &'static str {
        match self {
            HookFamily::Object => OBJECT_TYPES,
            HookFamily::Sky => SKY_TYPES,
        }
    }

    pub fn surface_signature(self) -> &'static str {
        match self {
            HookFamily::Object => {
                "fn surface(input: SurfaceIn, base: Surface, diffuse: texture_2d<f32>, diffuse_sampler: sampler, \
                 normal_map: texture_2d<f32>, specular_map: texture_2d<f32>, map_sampler: sampler) -> Surface"
            }
            HookFamily::Sky => {
                "fn surface(input: SurfaceIn, base: Surface, sky_texture: texture_2d<f32>, sky_sampler: sampler) \
                 -> Surface"
            }
        }
    }
}

pub const VERTEX_SIGNATURE: &str = "fn vertex_offset(input: VertexIn) -> vec3<f32>";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_bytes: usize,
    pub max_trips: u64,
    pub max_operations: u64,
    pub max_texture_reads: u64,
    pub max_frame_bytes: u64,
    pub max_value_bytes: u64,
}

pub const WGSL_FUNCTION_MEMORY: u64 = 8192;

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_bytes: 64 << 10,
            max_trips: 256,
            max_operations: 32_768,
            max_texture_reads: 64,
            max_frame_bytes: 2048,
            max_value_bytes: 2048,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cost {
    pub operations: u64,
    pub texture_reads: u64,
    pub frame_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Accepted {
    pub family: HookFamily,
    pub surface: Option<Cost>,
    pub vertex: Option<Cost>,
}

impl Accepted {
    pub fn max(&self, other: &Accepted) -> Accepted {
        let pick = |a: Option<Cost>, b: Option<Cost>| match (a, b) {
            (Some(a), Some(b)) => Some(Cost {
                operations: a.operations.max(b.operations),
                texture_reads: a.texture_reads.max(b.texture_reads),
                frame_bytes: a.frame_bytes.max(b.frame_bytes),
            }),
            (a, b) => a.or(b),
        };
        Accepted {
            family: self.family,
            surface: pick(self.surface, other.surface),
            vertex: pick(self.vertex, other.vertex),
        }
    }

    pub fn summary(&self) -> String {
        let one = |name: &str, c: &Option<Cost>| {
            c.map(|c| {
                format!(
                    "{name}: at most {} scalar operations, {} texture reads, {} bytes of function memory",
                    c.operations, c.texture_reads, c.frame_bytes
                )
            })
        };
        let parts: Vec<String> =
            [one("surface", &self.surface), one("vertex_offset", &self.vertex)].into_iter().flatten().collect();
        format!("{} family hooks, {}", self.family.as_str(), parts.join("; "))
    }
}

pub type Problem = (usize, usize, String);

pub const NOT_CHECKED: &str = "not checked: this program is built without the shader checker (the game, the launcher \
                               and the mod tools check shader hooks; a server refuses GPU code)";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Composed {
    pub accepted: Accepted,
    pub entry_frames: BTreeMap<String, u64>,
    pub entry_work: BTreeMap<String, u64>,
}

pub fn composed_source(types_module: &str, text: &str) -> String {
    format!("import {types_module}::{{{}}};\n{text}", INTERFACE_TYPES.join(", "))
}

#[cfg(all(test, feature = "shader"))]
mod tests;
#[cfg(all(test, feature = "shader"))]
mod tests_premises;
