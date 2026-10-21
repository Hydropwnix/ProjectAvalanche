use std::path::Path;

use anyhow::{Context, Result};
use ssx_formats::t3db::LocStrings;

use super::truetype::{Font, GlyphBitmap, rasterize};
use crate::texture::Image;

pub const WIDTH: u32 = 512;
pub const HEIGHT: u32 = 256;
pub const LEVELS: usize = 3;
pub const FONT_FILE: &str = "data/common/font/dinengschriftstd.ttf";
const LOC_FILE: &str = "data/xenon/loc/eng_us.db";
const DPI: i64 = 96;

fn archive_file(big_root: &Path, rel: &str) -> Option<std::path::PathBuf> {
    let mut dirs: Vec<_> = std::fs::read_dir(big_root).ok()?.flatten().map(|e| e.path()).collect();
    dirs.sort();
    dirs.into_iter().map(|d| d.join(rel)).find(|p| p.is_file())
}

pub fn load_font(big_root: &Path) -> Result<Font> {
    let path = archive_file(big_root, FONT_FILE).with_context(|| format!("{FONT_FILE} not found"))?;
    Font::parse(std::fs::read(&path)?).with_context(|| format!("parsing {}", path.display()))
}

pub struct Strings(LocStrings);

impl Strings {
    pub fn load(big_root: &Path) -> Result<Strings> {
        let path = archive_file(big_root, LOC_FILE).with_context(|| format!("{LOC_FILE} not found"))?;
        Ok(Strings(LocStrings::parse(&std::fs::read(&path)?).with_context(|| format!("parsing {}", path.display()))?))
    }

    pub fn text(&self, key: &str) -> String {
        self.0.get(key).unwrap_or(key).to_string()
    }
}

pub fn ppem(size: f32) -> f32 {
    let s = (size * 65536.0) as i64;
    let k = (DPI << 16) / 72;
    ((s * k + 0x8000) >> 16) as f32 / 65536.0
}

pub fn font_size(n: usize) -> f32 {
    let s = (WIDTH as f32 * 2.0) / n.max(1) as f32;
    if s - 60.0 >= 0.0 { 60.0 } else { s }
}

struct Placed {
    bitmap: GlyphBitmap,
    x: f32,
    y: f32,
}

fn layout(font: &Font, text: &str) -> Result<Vec<Placed>> {
    let units: Vec<u16> = text.encode_utf16().collect();
    let size = font_size(units.len());
    let k = ppem(size) / f32::from(font.units_per_em);
    let (a, d, l) = (f32::from(font.ascender) * k, f32::from(font.descender) * k, f32::from(font.line_gap) * k);
    // TODO(guess): advances are the unhinted scaled widths and glyph images the unhinted outline's exact area coverage over its pixel bounds (FontFusion's scan converter, hinting and integer advances not decoded); settle: EAText's OutlineFont glyph fetch (the FontFusion graymap call behind the GlyphMetricsMap) and FS_set_flags.
    let advance = |c: u16| f32::from(font.advance(font.glyph_index(u32::from(c)))) * k;
    let trimmed = units.iter().rposition(|&c| !matches!(c, 0x20 | 0xA0 | 0x3000)).map_or(0, |i| i + 1);
    let last_line = units[..trimmed].rsplit(|&c| c == 10).next().unwrap_or(&[]);
    let width: f32 = last_line.iter().map(|&c| advance(c)).sum();
    let lines = 1 + units.iter().filter(|&&c| c == 10).count();
    let x0 = WIDTH as f32 * 0.5 - width * 0.5;
    let mut baseline = HEIGHT as f32 * 0.5 - a * 0.5 * lines as f32 + (l + a).floor();
    let line_height = (a - d + l + 0.5).floor();
    let mut pen = x0;
    let mut out = Vec::new();
    for &c in &units {
        if c == 10 {
            baseline += line_height;
            pen = x0;
            continue;
        }
        let g = font.glyph_index(u32::from(c));
        let bitmap = rasterize(&font.contours(g)?, k);
        if bitmap.width > 0 && bitmap.coverage.iter().any(|&v| v > 0) {
            out.push(Placed { x: pen + bitmap.left as f32, y: baseline - bitmap.top as f32, bitmap });
        }
        pen += advance(c);
    }
    Ok(out)
}

fn glyph_alpha(b: &GlyphBitmap, u: f32, v: f32) -> f32 {
    let (x, y) = (u - 0.5, v - 0.5);
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let at = |i: i32, j: i32| -> f32 {
        if i < 0 || j < 0 || i >= b.width as i32 || j >= b.height as i32 {
            0.0
        } else {
            f32::from(b.coverage[(j as u32 * b.width + i as u32) as usize]) / 255.0
        }
    };
    let (i, j) = (x0 as i32, y0 as i32);
    let top = at(i, j) * (1.0 - fx) + at(i + 1, j) * fx;
    let bottom = at(i, j + 1) * (1.0 - fx) + at(i + 1, j + 1) * fx;
    top * (1.0 - fy) + bottom * fy
}

fn render_level0(font: &Font, text: &str, colour: [f32; 3]) -> Result<Image> {
    let c8 = colour.map(|c| ((c * 255.0) as i64).clamp(0, 255) as f32 / 255.0);
    let mut px = vec![[0.0f32; 4]; (WIDTH * HEIGHT) as usize];
    // TODO(guess): pixel centres at +0.5 for the glyph quads, the glyph cache's padding as zero texels, the 8-bit blend rounded as Xenia's ROV model (local_lights.wgsl blend8) — the Xenos half-pixel convention and the cache packing are not decoded; settle: the glyph mesh build (0x8346E3E8) and the device's pixel-centre state.
    for g in layout(font, text)? {
        let b = &g.bitmap;
        let (x1, y1) = (g.x + b.width as f32, g.y + b.height as f32);
        let cols = (g.x - 0.5).ceil().max(0.0) as u32..((x1 - 0.5).ceil().max(0.0) as u32).min(WIDTH);
        for row in (g.y - 0.5).ceil().max(0.0) as u32..((y1 - 0.5).ceil().max(0.0) as u32).min(HEIGHT) {
            for col in cols.clone() {
                let a = glyph_alpha(b, col as f32 + 0.5 - g.x, row as f32 + 0.5 - g.y);
                if a <= 0.0 {
                    continue;
                }
                let dst = &mut px[(row * WIDTH + col) as usize];
                let src = [c8[0], c8[1], c8[2], a];
                for (k, d) in dst.iter_mut().enumerate() {
                    *d = ((src[k] * a + *d * (1.0 - a)) * 255.0 + 0.5).floor().clamp(0.0, 255.0) / 255.0;
                }
            }
        }
    }
    let rgba = px.iter().flat_map(|p| p.map(|v| (v * 255.0 + 0.5) as u8)).collect();
    Ok(Image { width: WIDTH, height: HEIGHT, rgba })
}

pub fn half(level: &Image) -> Image {
    let (w, h) = ((level.width / 2).max(1), (level.height / 2).max(1));
    let at = |x: u32, y: u32, k: usize| {
        u32::from(level.rgba[((y.min(level.height - 1) * level.width + x.min(level.width - 1)) * 4) as usize + k])
    };
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            for k in 0..4 {
                let s = at(2 * x, 2 * y, k)
                    + at(2 * x + 1, 2 * y, k)
                    + at(2 * x, 2 * y + 1, k)
                    + at(2 * x + 1, 2 * y + 1, k);
                rgba.push(((s as f32 / 4.0) + 0.5) as u8);
            }
        }
    }
    Image { width: w, height: h, rgba }
}

pub fn render_text_texture(font: &Font, text: &str, colour: [f32; 3]) -> Result<Vec<Image>> {
    let mut levels = vec![render_level0(font, text, colour)?];
    while levels.len() < LEVELS {
        let next = half(levels.last().unwrap_or(&Image { width: 1, height: 1, rgba: vec![0; 4] }));
        levels.push(next);
    }
    Ok(levels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_rule() {
        assert_eq!(font_size(8), 60.0);
        assert_eq!(font_size(17), 60.0);
        assert!((font_size(20) - 51.2).abs() < 1e-4);
        assert!((ppem(60.0) - 80.0).abs() < 1e-3);
    }

    #[test]
    fn mip_box() {
        let l = Image { width: 2, height: 2, rgba: vec![0, 0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 0, 0, 0, 0] };
        let h = half(&l);
        assert_eq!((h.width, h.height), (1, 1));
        assert_eq!(h.rgba, vec![128, 64, 64, 64]);
    }
}
