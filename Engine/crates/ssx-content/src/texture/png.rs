use std::io::Cursor;

use super::{Level, Limits, TextureError, check_size};

pub(super) fn size(bytes: &[u8], file: &str) -> Result<(u32, u32), TextureError> {
    let mut decoder = png::Decoder::new_with_limits(Cursor::new(bytes), png::Limits { bytes: 1 << 20 });
    decoder.set_ignore_text_chunk(true);
    decoder.set_ignore_iccp_chunk(true);
    let header = decoder.read_header_info().map_err(|e| TextureError::new(file, format!("PNG: {e}")))?;
    Ok((header.width, header.height))
}

pub(super) fn decode(bytes: &[u8], file: &str, limits: &Limits) -> Result<Level, TextureError> {
    let fail = |e: &dyn std::fmt::Display| TextureError::new(file, format!("PNG: {e}"));
    let cap = usize::try_from(limits.max_decoded_bytes.saturating_mul(2)).unwrap_or(usize::MAX);
    let mut decoder = png::Decoder::new_with_limits(Cursor::new(bytes), png::Limits { bytes: cap });
    decoder.set_ignore_text_chunk(true);
    decoder.set_ignore_iccp_chunk(true);
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let header = decoder.read_header_info().map_err(|e| fail(&e))?;
    let (width, height) = (header.width, header.height);
    check_size(file, width, height, 1, limits)?;
    let mut reader = decoder.read_info().map_err(|e| fail(&e))?;
    let size = reader.output_buffer_size().ok_or_else(|| fail(&"the image does not fit in memory"))?;
    let (colour, depth) = reader.output_color_type();
    if depth != png::BitDepth::Eight {
        return Err(fail(&format!("{depth:?} channels after the 8-bit conversion")));
    }
    let channels = match colour {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return Err(fail(&"a palette left after the expansion")),
    };
    if size as u64 != u64::from(width) * u64::from(height) * channels {
        return Err(fail(&format!("{size} bytes for {width}×{height} × {channels} channels")));
    }
    let mut buf = vec![0u8; size];
    let info = reader.next_frame(&mut buf).map_err(|e| fail(&e))?;
    if (info.width, info.height) != (width, height) {
        let (w, h) = (info.width, info.height);
        return Err(fail(&format!("the first frame is {w}×{h}, the image {width}×{height}")));
    }
    let rgba: Vec<u8> = match channels {
        1 => buf.iter().flat_map(|&v| [v, v, v, 255]).collect(),
        2 => buf.as_chunks::<2>().0.iter().flat_map(|&[v, a]| [v, v, v, a]).collect(),
        3 => buf.as_chunks::<3>().0.iter().flat_map(|&[r, g, b]| [r, g, b, 255]).collect(),
        _ => buf,
    };
    Ok(Level { width, height, images: vec![rgba] })
}
