use std::io::{BufReader, BufWriter};
use std::path::Path;

use anyhow::{Context, Result, bail};
use eframe::egui::ColorImage;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

impl Picture {
    pub fn new(width: usize, height: usize, rgba: Vec<u8>) -> Result<Self> {
        if width == 0 || height == 0 || rgba.len() != width * height * 4 {
            bail!("a {width} x {height} picture needs {} bytes, got {}", width * height * 4, rgba.len());
        }
        Ok(Self { width, height, rgba })
    }

    pub fn crop(&self, x: usize, y: usize, w: usize, h: usize) -> Self {
        let (x, y) = (x.min(self.width - 1), y.min(self.height - 1));
        let (w, h) = (w.clamp(1, self.width - x), h.clamp(1, self.height - y));
        let mut rgba = Vec::with_capacity(w * h * 4);
        for row in y..y + h {
            let at = (row * self.width + x) * 4;
            rgba.extend_from_slice(&self.rgba[at..at + w * 4]);
        }
        Self { width: w, height: h, rgba }
    }

    pub fn resize(&self, w: usize, h: usize) -> Self {
        let (w, h) = (w.max(1), h.max(1));
        if (w, h) == (self.width, self.height) {
            return self.clone();
        }
        let (sx, sy) = (self.width as f64 / w as f64, self.height as f64 / h as f64);
        let spans = |n: usize, step: f64, limit: usize| -> Vec<Vec<(usize, f64)>> {
            (0..n)
                .map(|i| {
                    let (a, b) = (i as f64 * step, (i + 1) as f64 * step);
                    let (a, b) = if step < 1.0 { (a + step / 2.0 - 0.5, a + step / 2.0 + 0.5) } else { (a, b) };
                    let (first, last) = (a.floor().max(0.0) as usize, (b.ceil() as usize).clamp(1, limit));
                    (first..last)
                        .map(|c| (c, (b.min(c as f64 + 1.0) - a.max(c as f64)).max(0.0)))
                        .filter(|&(_, weight)| weight > 0.0)
                        .collect()
                })
                .collect()
        };
        let (cols, rows) = (spans(w, sx, self.width), spans(h, sy, self.height));
        let mut rgba = Vec::with_capacity(w * h * 4);
        for row in &rows {
            for col in &cols {
                let (mut sum, mut alpha, mut total) = ([0f64; 3], 0f64, 0f64);
                for &(y, wy) in row {
                    for &(x, wx) in col {
                        let p = &self.rgba[(y * self.width + x) * 4..][..4];
                        let (weight, a) = (wx * wy, p[3] as f64);
                        for c in 0..3 {
                            sum[c] += p[c] as f64 * a * weight;
                        }
                        alpha += a * weight;
                        total += weight;
                    }
                }
                let colour = |c: usize| if alpha > 0.0 { (sum[c] / alpha).round() as u8 } else { 0 };
                rgba.extend([colour(0), colour(1), colour(2), (alpha / total.max(1e-9)).round() as u8]);
            }
        }
        Self { width: w, height: h, rgba }
    }

    pub fn read_png(path: &Path) -> Result<Self> {
        let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
        let mut decoder = png::Decoder::new(BufReader::new(file));
        decoder.set_transformations(png::Transformations::normalize_to_color8());
        let mut reader = decoder.read_info().with_context(|| format!("{} is not a PNG", path.display()))?;
        let mut buf = vec![0; reader.output_buffer_size().context("PNG too large")?];
        let info = reader.next_frame(&mut buf)?;
        let data = &buf[..info.buffer_size()];
        let rgba: Vec<u8> = match info.color_type {
            png::ColorType::Rgba => data.to_vec(),
            png::ColorType::Rgb => data.as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
            png::ColorType::Grayscale => data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
            png::ColorType::GrayscaleAlpha => {
                data.as_chunks::<2>().0.iter().flat_map(|p| [p[0], p[0], p[0], p[1]]).collect()
            }
            png::ColorType::Indexed => bail!("{}: palette not expanded", path.display()),
        };
        Self::new(info.width as usize, info.height as usize, rgba)
    }

    pub fn write_png(&self, path: &Path) -> Result<()> {
        ssx_files::replace_with(path, &ssx_files::Options::new(), |file| -> Result<()> {
            let mut out = BufWriter::new(file);
            let mut encoder = png::Encoder::new(&mut out, self.width as u32, self.height as u32);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_compression(png::Compression::Fast);
            let mut writer = encoder.write_header()?;
            writer.write_image_data(&self.rgba)?;
            writer.finish()?;
            std::io::Write::flush(&mut out).with_context(|| format!("writing {}", path.display()))
        })
    }

    pub fn to_egui(&self) -> ColorImage {
        ColorImage::from_rgba_unmultiplied([self.width, self.height], &self.rgba)
    }
}
