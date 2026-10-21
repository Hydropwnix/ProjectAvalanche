use anyhow::{Context, Result, bail};
use ssx_assets::texture::{self, DxnExport};
use ssx_formats::dds::{DdsFile, DdsFormat};
use ssx_formats::xenos::TextureFormat;
use ssx_formats::xpr::XprFile;

use crate::fs::{FileSource, normalise};

#[derive(Clone, Debug)]
pub struct ImageData {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub source: String,
}

pub fn decode_xpr(data: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
    let xpr = XprFile::parse(data)?;
    let res = xpr.resources.first().context("empty .xpr")?;
    let levels = texture::decode_resource(data, &xpr, res, false, DxnExport::Raw)?;
    let level = levels.into_iter().next().context("no level")?;
    Ok((level.image.width, level.image.height, level.image.rgba))
}

pub fn decode_dds(data: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
    let dds = DdsFile::parse(data)?;
    let format = match dds.format {
        DdsFormat::Dxt1 => TextureFormat::Dxt1,
        DdsFormat::Dxt3 => TextureFormat::Dxt23,
        DdsFormat::Dxt5 => TextureFormat::Dxt45,
    };
    let texels = texture::decode_level(format, dds.level0(data), dds.width, dds.height)?;
    Ok((dds.width, dds.height, texels.into_iter().flatten().collect()))
}

pub fn resolve_external(fs: &dyn FileSource, dir: &str, name: &str) -> Option<String> {
    let full = normalise(&format!("{dir}{name}"));
    let stem = full.rsplit_once('.').map_or(full.as_str(), |(s, _)| s);
    let xpr = format!("{stem}.xpr");
    if fs.exists(&xpr) {
        return Some(xpr);
    }
    fs.exists(&full).then_some(full)
}

pub fn load(fs: &dyn FileSource, path: &str) -> Result<ImageData> {
    let data = fs.read(path).with_context(|| format!("{path} not found"))?;
    let lower = path.to_lowercase();
    let (width, height, rgba) = if lower.ends_with(".xpr") {
        decode_xpr(&data)
    } else if lower.ends_with(".dds") {
        decode_dds(&data)
    } else {
        bail!("{path}: no reader for this image type")
    }
    .with_context(|| format!("decoding {path}"))?;
    Ok(ImageData { width, height, rgba, source: path.to_string() })
}
