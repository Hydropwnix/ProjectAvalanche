use std::sync::Arc;

use ssx_formats::gfx::text::Font;

use crate::display::TextFilter;
use crate::glyph::{MAX_SLOT_HEIGHT, Rasterized, SIZE_CLASSES, SLOT_PADDING, Slot, outline_cells, paths, table_bounds};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct GlyphFilter {
    pub flags: u16,
    pub blur_x: u16,
    pub blur_y: u16,
    pub strength: u16,
}

impl ShadowParam {
    pub fn drawn(&self) -> bool {
        self.color != [0.0; 4]
    }
}

impl GlyphFilter {
    pub const KNOCKOUT: u16 = 0x20;
    pub const HIDE_OBJECT: u16 = 0x40;
    pub const FINE_BLUR: u16 = 0x80;

    pub fn blurred(&self) -> bool {
        self.blur_x != 0 || self.blur_y != 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowParam {
    pub filter: GlyphFilter,
    pub color: [f32; 4],
    pub offset: (f32, f32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextFieldParam {
    pub text: GlyphFilter,
    pub shadow: Option<ShadowParam>,
}

fn fixed(v: f32) -> u16 {
    v.mul_add(16.0, 0.5) as i64 as u16
}

impl TextFieldParam {
    pub fn new(f: &TextFilter) -> TextFieldParam {
        let text = GlyphFilter {
            flags: GlyphFilter::FINE_BLUR,
            blur_x: fixed(f.blur.0),
            blur_y: fixed(f.blur.1),
            strength: 16,
        };
        let shadow = f.shadow.map(|s| {
            let flag = |on: bool, bit: u16| if on { bit } else { 0 };
            let flags = flag(s.knockout, GlyphFilter::KNOCKOUT)
                | flag(s.hide_object, GlyphFilter::HIDE_OBJECT)
                | flag(s.high_quality, GlyphFilter::FINE_BLUR);
            let filter =
                GlyphFilter { flags, blur_x: fixed(s.blur_x), blur_y: fixed(s.blur_y), strength: fixed(s.strength) };
            ShadowParam { filter, color: s.color, offset: s.offset }
        });
        TextFieldParam { text, shadow }
    }
}

pub fn shadow_size(screen: f32) -> u16 {
    let n = screen.max(0.0).floor() as u32;
    let class = if n < 256 { SIZE_CLASSES[SIZE_CLASSES.iter().position(|&c| c >= n).unwrap_or(26)] } else { 255 };
    let s = (class as f32).min((MAX_SLOT_HEIGHT - 2 * SLOT_PADDING) as f32);
    (s * 16.0).floor() as u16
}

#[derive(Clone, Debug, Default)]
pub struct BlurRows {
    w: Vec<f32>,
    out: Vec<u8>,
}

impl BlurRows {
    fn pass(&mut self, t: &mut [u8], (count, step): (usize, usize), (lines, line_step): (usize, usize), radius: f32) {
        let k = |bits: u32| f32::from_bits(bits);
        let r = if radius < k(0x3F1E_B852) { k(0x3F1E_B852) } else { radius };
        let len = (f64::from(r).ceil() as f32 as i32).saturating_add(count as i32);
        if len < 3 {
            return;
        }
        let len = len as usize;
        let sigma = r * 0.5;
        let q = if sigma < 2.5 {
            sigma.mul_add(-k(0x3E89_AE92), 1.0).sqrt().mul_add(-k(0x4084_A844), k(0x407E_2E0A))
        } else {
            sigma.mul_add(k(0x3F7C_B33E), -k(0x3F76_9AD4))
        };
        let (q2, q1) = (q * q, q * k(0x401C_6CA0));
        let q3 = q2 * q;
        let c3 = q3 * k(0x3FA2_2047);
        let b3 = q3 * k(0x3ED8_2B41);
        let b2 = q2.mul_add(-k(0x3FB6_CBFB), -c3);
        let b1 = q2.mul_add(k(0x4036_CBD1), c3) + q1;
        let b0 = q2.mul_add(k(0x3FB6_CBFB), q1) + b3 + k(0x3FCA_0419);
        let sum = b3 + b2 + b1;
        let inv = 1.0 / b0;
        let (n1, n2, n3) = (b1 * inv, b2 * inv, b3 * inv);
        let big_b = sum.mul_add(-inv, 1.0);
        let s32 = n3 + n2;
        let s321 = s32 + n1;
        let g = big_b + n3 + n2 + n1;
        self.w.resize(len, 0.0);
        self.out.resize(len, 0);
        for line in 0..lines {
            let base = line * line_step;
            let at = |i: usize| if i < count { f32::from(t[base + i * step]) } else { 0.0 };
            let w = &mut self.w;
            w[0] = at(0) * g;
            let (mut f0, mut f13) = (w[0], at(1).mul_add(big_b, s321 * w[0]));
            w[1] = f13;
            let mut f12 = f13.mul_add(n1, at(2).mul_add(big_b, s32 * w[0]));
            w[2] = f12;
            for (i, wi) in w.iter_mut().enumerate().skip(3) {
                let t11 = f13.mul_add(n2, at(i).mul_add(big_b, f0 * n3));
                (f0, f13) = (f13, f12);
                f12 = f12.mul_add(n1, t11);
                *wi = f12;
            }
            let mut f13 = w[len - 1] * g;
            let mut f0 = w[len - 2].mul_add(big_b, s321 * f13);
            let mut f12 = f0.mul_add(n1, w[len - 3].mul_add(big_b, s32 * f13));
            for i in (0..len - 3).rev() {
                let t11 = f0.mul_add(n2, w[i].mul_add(big_b, f13 * n3));
                (f13, f0) = (f0, f12);
                f12 = f12.mul_add(n1, t11);
                self.out[i] = (f12 + 0.5) as i64 as u8;
            }
            for i in 0..count {
                t[base + i * step] = self.out[i];
            }
        }
    }
}

fn apply_strength(t: &mut [u8], strength: f32, reference: i32) {
    if strength == 1.0 {
        return;
    }
    for v in t {
        let x = ((i32::from(*v) - reference) as f32).mul_add(strength, 0.5) as i32 + reference;
        *v = x.clamp(0, 255) as u8;
    }
}

fn knock_out(t: &mut [u8], original: &[u8]) {
    for (v, o) in t.iter_mut().zip(original) {
        *v = ((u32::from(*v) * (255 - u32::from(*o)) + 255) >> 8) as u8;
    }
}

pub fn rasterize_filtered(
    font: &Font,
    index: usize,
    size: u16,
    f: &GlyphFilter,
    ratio: f32,
    screen: f32,
    rows: &mut BlurRows,
) -> (Rasterized, f32) {
    let Some(shape) = font.glyphs.get(index) else { return (Rasterized::Empty, 1.0) };
    let paths = paths(&shape.records);
    let units = crate::text::em_units(font);
    let s = f32::from(size) * 0.0625;
    let per = s / screen;
    let mut k = s / units;
    let mut bx = ((f32::from(f.blur_x) * ratio) * per) * 0.0625;
    let mut by = ((f32::from(f.blur_y) * ratio) * per) * 0.0625;
    let b = table_bounds(&paths, units);
    let (y0, y1) = if b[1] < b[3] { (b[1], b[3]) } else { (0.0, 0.0) };
    let max_h = (MAX_SLOT_HEIGHT - 2 * SLOT_PADDING) as f32;
    let tall = y1.mul_add(k, by) - y0.mul_add(k, -by);
    let mut scale = 1.0;
    if tall >= max_h {
        let fit = max_h / tall;
        (k, bx, by, scale) = (k * fit, bx * fit, by * fit, 1.0 / fit);
    }
    let ceil = |v: f32| f64::from(v).ceil() as f32 as i32;
    let ras = outline_cells(&paths, f64::from(k));
    if ras.is_empty() {
        return (Rasterized::Empty, scale);
    }
    let pad = SLOT_PADDING as i32;
    let mut slot = Slot::new(&ras, (pad + ceil(bx), pad + ceil(by)));
    if slot.width <= 1 || slot.height <= 1 {
        return (Rasterized::Empty, scale);
    }
    let original = (f.flags & GlyphFilter::KNOCKOUT != 0).then(|| slot.texels.clone());
    let blurred = bx > 0.0 || by > 0.0;
    if blurred {
        let (w, h) = (slot.width as usize, slot.height as usize);
        rows.pass(&mut slot.texels, (w, 1), (h, w), bx);
        rows.pass(&mut slot.texels, (h, w), (w, 1), by);
    }
    let strength = f32::from(f.strength) * 0.0625;
    apply_strength(&mut slot.texels, strength, if strength > 1.0 && blurred { 8 } else { 0 });
    if let Some(o) = original {
        knock_out(&mut slot.texels, &o);
    }
    (Rasterized::Raster(Arc::new(slot.quad_texels())), scale)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::TextShadow;
    use ssx_formats::gfx::shape::Shape;

    fn box_font(w: i32, h: i32) -> Font {
        let records = crate::display::tests::rect_shape(1, w, h).shape.records;
        Font {
            version: 2,
            id: 1,
            flags: 0,
            language: 0,
            name: "Box".into(),
            glyphs: vec![Shape { styles: Default::default(), records }],
            codes: vec![u16::from(b'A')],
            layout: None,
            unk_trailing_layout: None,
        }
    }

    fn raster(r: (Rasterized, f32)) -> (Arc<crate::glyph::GlyphRaster>, f32) {
        match r {
            (Rasterized::Raster(g), s) => (g, s),
            other => panic!("a raster, not {other:?}"),
        }
    }

    #[test]
    fn the_filter_object_becomes_the_text_fields_parameter() {
        let f = TextFilter {
            blur: (60.0, 30.5),
            blur_high_quality: true,
            shadow: Some(TextShadow {
                color: [0.0, 0.0, 0.0, 1.0],
                blur_x: 100.0,
                blur_y: 100.0,
                strength: 2.5,
                offset: (14.0, 14.0),
                knockout: true,
                hide_object: false,
                high_quality: true,
            }),
        };
        let p = TextFieldParam::new(&f);
        assert_eq!(p.text, GlyphFilter { flags: 0x80, blur_x: 960, blur_y: 488, strength: 16 });
        let s = p.shadow.expect("a shadow");
        assert_eq!(s.filter, GlyphFilter { flags: 0xA0, blur_x: 1600, blur_y: 1600, strength: 40 });
        assert!(s.drawn());
        let clear = TextShadow { color: [0.0; 4], ..f.shadow.unwrap() };
        assert!(!TextFieldParam::new(&TextFilter { shadow: Some(clear), ..f }).shadow.unwrap().drawn());
    }

    #[test]
    fn a_shadow_offset_is_whole_twips() {
        assert_eq!(crate::display::shadow_offset(4.0, 51472.0 / 65536.0), (56.0, 56.0));
        assert_eq!(crate::display::shadow_offset(1.0, 308831.0 / 65536.0), (0.0, -19.0));
        assert_eq!(crate::display::shadow_offset(0.0, 1.0), (0.0, 0.0));
    }

    #[test]
    fn the_shadow_size_is_the_class_of_the_height_at_most_44() {
        let px = |h: f32| f32::from(shadow_size(h)) / 16.0;
        assert_eq!(px(20.9), 22.0);
        assert_eq!(px(26.0), 26.0);
        assert_eq!(px(36.5), 36.0);
        assert_eq!(px(40.5), 40.0);
        assert_eq!(px(41.0), 44.0);
        assert_eq!(px(300.0), 44.0);
    }

    #[test]
    fn strength_and_knockout_as_the_game_rounds_them() {
        let mut t = [0u8, 7, 8, 100, 200, 255];
        apply_strength(&mut t, 2.0, 8);
        assert_eq!(t, [0, 7, 8, 192, 255, 255]);
        let mut t = [10u8, 100, 255];
        apply_strength(&mut t, 0.5, 0);
        assert_eq!(t, [5, 50, 128]);
        let mut t = [255u8, 255, 128];
        knock_out(&mut t, &[255, 0, 128]);
        assert_eq!(t, [0, 255, 64]);
    }

    #[test]
    fn the_recursive_gaussian_spreads_and_keeps_the_mass() {
        let mut rows = BlurRows::default();
        let mut line = [0u8; 21];
        line[10] = 255;
        rows.pass(&mut line, (21, 1), (1, 21), 4.0);
        let sum: u32 = line.iter().map(|&v| u32::from(v)).sum();
        assert!((250..=262).contains(&sum), "{sum} {line:?}");
        assert!(line[10] < 120 && line[8] > 20 && line[12] > 20, "{line:?}");
        assert!(line[8].abs_diff(line[12]) <= 3, "{line:?}");
        let mut flat = [200u8; 40];
        rows.pass(&mut flat, (40, 1), (1, 40), 3.0);
        assert!(flat[5..30].iter().all(|&v| v.abs_diff(200) <= 1), "{flat:?}");
    }

    #[test]
    fn a_blurred_raster_grows_by_its_blur_and_tall_ones_are_fitted() {
        let f = box_font(512, 700);
        let plain = GlyphFilter { flags: 0x80, blur_x: 0, blur_y: 0, strength: 16 };
        let mut rows = BlurRows::default();
        let (g0, s0) = raster(rasterize_filtered(&f, 0, 32 * 16, &plain, 0.05, 32.0, &mut rows));
        let blur = GlyphFilter { blur_x: 960, blur_y: 960, ..plain };
        let (g1, s1) = raster(rasterize_filtered(&f, 0, 32 * 16, &blur, 0.05, 32.0, &mut rows));
        assert_eq!((s0, s1), (1.0, 1.0));
        assert_eq!((g1.width, g1.height), (g0.width + 6, g0.height + 6));
        assert_eq!((g1.x0, g1.y0), (g0.x0 - 3, g0.y0 - 3));
        let mass = |g: &crate::glyph::GlyphRaster| g.alpha.iter().map(|&v| u64::from(v)).sum::<u64>() as f64;
        assert!((mass(&g1) / mass(&g0) - 1.0).abs() < 0.02, "{} {}", mass(&g0), mass(&g1));
        let (_, s2) = raster(rasterize_filtered(&f, 0, 64 * 16, &blur, 0.05, 64.0, &mut rows));
        assert!(s2 > 1.0, "{s2}");
    }
}
