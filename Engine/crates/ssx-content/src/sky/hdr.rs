use std::io::Cursor;

use image::ImageDecoder;
use image::codecs::hdr::HdrDecoder;

use super::{FloatImage, Limits};

pub fn size(head: &[u8]) -> Result<(u32, u32), String> {
    let d = HdrDecoder::new(Cursor::new(head)).map_err(|e| format!("not a Radiance HDR image: {e}"))?;
    Ok(d.dimensions())
}

pub fn decode(bytes: &[u8], limits: &Limits) -> Result<FloatImage, String> {
    let d = HdrDecoder::new(Cursor::new(bytes)).map_err(|e| format!("not a Radiance HDR image: {e}"))?;
    let (width, height) = d.dimensions();
    super::check_source(width, height, limits)?;
    let texels = width as usize * height as usize;
    if d.total_bytes() != texels as u64 * 12 {
        return Err(format!("the decoder states {} bytes for {width}×{height}", d.total_bytes()));
    }
    let mut raw = vec![0u8; texels * 12];
    d.read_image(&mut raw).map_err(|e| format!("a damaged Radiance HDR image: {e}"))?;
    let mut rgb = Vec::with_capacity(texels);
    for t in raw.as_chunks::<12>().0 {
        let lane = |k: usize| f32::from_ne_bytes([t[k], t[k + 1], t[k + 2], t[k + 3]]);
        let v = [lane(0), lane(4), lane(8)];
        if v.iter().any(|x| !x.is_finite() || *x < 0.0) {
            return Err("a texel that is not a finite value of at least 0".into());
        }
        rgb.push(v);
    }
    Ok(FloatImage { width, height, rgb })
}
