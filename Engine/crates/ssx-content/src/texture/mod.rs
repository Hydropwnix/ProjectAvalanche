pub mod bc;
pub mod cache;
mod dds;
pub mod encode;
mod jpeg;
mod ktx2;
mod load;
pub mod mips;
mod png;
pub mod resolve;
pub mod settings;

pub use load::{FORMATS, TextureProbe, decode_file, image_size, probe_file};
pub use resolve::{Fallback, Found, Sampling, Want, fit, resolve, resolve_in};
pub use settings::{FolderSettings, TextureSettings};

use crate::hash::ContentHash;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Shape {
    Plane,
    Cube,
    Volume,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Level {
    pub width: u32,
    pub height: u32,
    pub images: Vec<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BlockFormat {
    Bc1,
    Bc2,
    Bc3,
    Bc4,
    Bc5,
    Bc7,
}

impl BlockFormat {
    pub const fn bytes_per_block(self) -> usize {
        match self {
            BlockFormat::Bc1 | BlockFormat::Bc4 => 8,
            BlockFormat::Bc2 | BlockFormat::Bc3 | BlockFormat::Bc5 | BlockFormat::Bc7 => 16,
        }
    }

    pub const fn level_bytes(self, width: u32, height: u32) -> u64 {
        width.div_ceil(4) as u64 * height.div_ceil(4) as u64 * self.bytes_per_block() as u64
    }

    pub const fn name(self) -> &'static str {
        match self {
            BlockFormat::Bc1 => "BC1",
            BlockFormat::Bc2 => "BC2",
            BlockFormat::Bc3 => "BC3",
            BlockFormat::Bc4 => "BC4",
            BlockFormat::Bc5 => "BC5",
            BlockFormat::Bc7 => "BC7",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockLevel {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockChain {
    pub format: BlockFormat,
    pub levels: Vec<BlockLevel>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TextureKind {
    Color,
    Normal,
    Data,
}

impl TextureKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            TextureKind::Color => "color",
            TextureKind::Normal => "normal",
            TextureKind::Data => "data",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Texture {
    pub shape: Shape,
    pub levels: Vec<Level>,
    pub blocks: Option<BlockChain>,
    pub kind: Option<TextureKind>,
    pub source: Option<ContentHash>,
}

impl Texture {
    pub fn width(&self) -> u32 {
        self.levels.first().map_or(0, |l| l.width)
    }

    pub fn height(&self) -> u32 {
        self.levels.first().map_or(0, |l| l.height)
    }

    pub fn has_alpha(&self) -> bool {
        self.levels.first().is_some_and(|l| l.images.iter().any(|i| i.as_chunks::<4>().0.iter().any(|t| t[3] != 255)))
    }

    pub fn decoded_bytes(&self) -> u64 {
        self.levels.iter().flat_map(|l| &l.images).map(|i| i.len() as u64).sum()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_file_bytes: u64,
    pub max_side: u32,
    pub max_decoded_bytes: u64,
    pub max_jpeg_scans: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { max_file_bytes: 256 << 20, max_side: 4096, max_decoded_bytes: 256 << 20, max_jpeg_scans: 64 }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{}", ssx_core::text::located(.file, .message))]
pub struct TextureError {
    pub file: String,
    pub message: String,
}

impl TextureError {
    pub fn new(file: &str, message: impl Into<String>) -> Self {
        TextureError { file: file.to_string(), message: message.into() }
    }
}

pub(crate) fn check_size(file: &str, width: u32, height: u32, faces: u32, limits: &Limits) -> Result<(), TextureError> {
    if width == 0 || height == 0 {
        return Err(TextureError::new(file, format!("{width}×{height}: an empty image")));
    }
    if width > limits.max_side || height > limits.max_side {
        let max = limits.max_side;
        return Err(TextureError::new(file, format!("{width}×{height} is larger than {max}×{max}")));
    }
    let bytes = chain_bytes(width, height) * u64::from(faces);
    if bytes > limits.max_decoded_bytes {
        let cap = limits.max_decoded_bytes;
        return Err(TextureError::new(file, format!("{bytes} bytes decoded with mips, over the cap of {cap}")));
    }
    Ok(())
}

pub fn chain_bytes(width: u32, height: u32) -> u64 {
    let (mut w, mut h, mut total) = (u64::from(width), u64::from(height), 0u64);
    loop {
        total += w * h * 4;
        if w == 1 && h == 1 {
            return total;
        }
        (w, h) = ((w / 2).max(1), (h / 2).max(1));
    }
}

pub fn chain_len(width: u32, height: u32) -> u32 {
    32 - width.max(height).max(1).leading_zeros()
}

#[cfg(test)]
mod tests;
