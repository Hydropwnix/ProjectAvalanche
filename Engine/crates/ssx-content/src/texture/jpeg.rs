use zune_jpeg::JpegDecoder;
use zune_jpeg::zune_core::bytestream::ZCursor;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::DecoderOptions;

use super::{Level, Limits, TextureError, check_size};

pub(super) fn size(bytes: &[u8], file: &str) -> Result<(u32, u32), TextureError> {
    let fail = |e: &dyn std::fmt::Display| TextureError::new(file, format!("JPEG: {e}"));
    let any = usize::from(u16::MAX);
    let options = DecoderOptions::default().set_max_width(any).set_max_height(any).set_strict_mode(true);
    let mut decoder = JpegDecoder::new_with_options(ZCursor::new(bytes), options);
    decoder.decode_headers().map_err(|e| fail(&e))?;
    let info = decoder.info().ok_or_else(|| fail(&"no image header"))?;
    Ok((u32::from(info.width), u32::from(info.height)))
}

pub(super) fn decode(bytes: &[u8], file: &str, limits: &Limits) -> Result<Level, TextureError> {
    let fail = |e: &dyn std::fmt::Display| TextureError::new(file, format!("JPEG: {e}"));
    let side = limits.max_side as usize;
    let options = DecoderOptions::default()
        .set_max_width(side)
        .set_max_height(side)
        .set_strict_mode(true)
        .jpeg_set_max_scans(limits.max_jpeg_scans)
        .jpeg_set_out_colorspace(ColorSpace::RGBA);
    let mut decoder = JpegDecoder::new_with_options(ZCursor::new(bytes), options);
    decoder.decode_headers().map_err(|e| fail(&e))?;
    let info = decoder.info().ok_or_else(|| fail(&"no image header"))?;
    let (width, height) = (u32::from(info.width), u32::from(info.height));
    check_size(file, width, height, 1, limits)?;
    let size = decoder.output_buffer_size().ok_or_else(|| fail(&"the image does not fit in memory"))?;
    if size as u64 != u64::from(width) * u64::from(height) * 4 {
        return Err(fail(&format!("{size} bytes of output for {width}×{height} RGBA")));
    }
    let mut rgba = vec![0u8; size];
    decoder.decode_into(&mut rgba).map_err(|e| fail(&e))?;
    Ok(Level { width, height, images: vec![rgba] })
}
