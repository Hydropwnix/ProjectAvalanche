use std::io::Cursor;

use exr::image::read::image::ReadLayers;
use exr::image::read::layers::ReadChannels;
use exr::meta::MetaData;

use super::{FloatImage, Limits};

pub fn size(head: &[u8]) -> Result<(u32, u32), String> {
    let meta =
        MetaData::read_from_buffered(Cursor::new(head), false).map_err(|e| format!("not an OpenEXR image: {e}"))?;
    layer(&meta)
}

fn layer(meta: &MetaData) -> Result<(u32, u32), String> {
    let [header] = meta.headers.as_slice() else {
        return Err(format!("{} layers (one is read: a sky is one image)", meta.headers.len()));
    };
    if header.deep {
        return Err("deep data (a sky is a flat image)".into());
    }
    for c in ["R", "G", "B"] {
        if !header.channels.list.iter().any(|ch| ch.name.eq(c)) {
            return Err(format!("no {c} channel (an RGB image is read)"));
        }
    }
    let (w, h) = (header.layer_size.width(), header.layer_size.height());
    let w = u32::try_from(w).map_err(|_| format!("a width of {w}"))?;
    let h = u32::try_from(h).map_err(|_| format!("a height of {h}"))?;
    Ok((w, h))
}

pub fn decode(bytes: &[u8], limits: &Limits) -> Result<FloatImage, String> {
    let meta =
        MetaData::read_from_buffered(Cursor::new(bytes), false).map_err(|e| format!("not an OpenEXR image: {e}"))?;
    let (width, height) = layer(&meta)?;
    super::check_source(width, height, limits)?;
    let (w, h) = (width as usize, height as usize);
    let image = exr::prelude::read()
        .no_deep_data()
        .largest_resolution_level()
        .rgba_channels(
            move |size, _| {
                if size.width() == w && size.height() == h { vec![[0f32; 3]; w * h] } else { Vec::new() }
            },
            move |pixels: &mut Vec<[f32; 3]>, at, (r, g, b, _a): (f32, f32, f32, f32)| {
                if let Some(p) = pixels.get_mut(at.y() * w + at.x()) {
                    *p = [r, g, b];
                }
            },
        )
        .first_valid_layer()
        .all_attributes()
        .non_parallel()
        .from_buffered(Cursor::new(bytes))
        .map_err(|e| format!("a damaged OpenEXR image: {e}"))?;
    let rgb = image.layer_data.channel_data.pixels;
    if rgb.len() != w * h {
        return Err(format!("its layer is not the {width}×{height} its headers state"));
    }
    if rgb.iter().flatten().any(|v| !v.is_finite() || *v < 0.0) {
        return Err("a texel that is not a finite value of at least 0".into());
    }
    Ok(FloatImage { width, height, rgb })
}
