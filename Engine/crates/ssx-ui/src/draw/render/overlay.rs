use std::collections::HashMap;
use std::sync::Arc;

use super::{BlendMode, Command, Cxform, DrawItem, Inherited, Mat2d, Paint, RectF, Renderer, TextGlyph, TextLayers};
use crate::library::{BitmapDef, ImageId, Library};
use crate::tess::Mesh;
use crate::text::{self, Align, CharFormat, ParaFormat, TextLayout};

pub const STAGE: (f32, f32) = (1280.0, 720.0);
const KEPT_LAYOUTS: usize = 4096;
const SHADOW_OFFSET: f32 = 2.0;
const SHADOW_ALPHA: f32 = 0.75;

#[derive(Clone, Debug, PartialEq)]
pub enum OverlayDraw {
    Rect { x: f32, y: f32, w: f32, h: f32, color: [f32; 4] },
    Image { x: f32, y: f32, w: f32, h: f32, image: ImageId, texels: [u32; 2], color: [f32; 4] },
    Text { text: String, x: f32, y: f32, size: f32, color: [f32; 4], align: Align, shadow: bool },
    Fill { color: [f32; 4] },
}

struct LastList {
    draws: Vec<OverlayDraw>,
    stage: Mat2d,
    viewport: (u32, u32),
    image_base: usize,
    font: String,
    commands: Vec<Command>,
}

type LayoutKey = (String, u32, u8, String);

#[derive(Default)]
pub(super) struct OverlayCache {
    last: Option<LastList>,
    layouts: HashMap<LayoutKey, Arc<TextLayout>>,
}

pub fn stage_transform(viewport: (u32, u32), mode: super::ScaleMode) -> Mat2d {
    let stage = RectF { x0: 0.0, y0: 0.0, x1: STAGE.0 * 20.0, y1: STAGE.1 * 20.0 };
    super::stage_transform(stage, viewport, mode).mul(&Mat2d::scale(20.0, 20.0))
}

pub fn drawable(lib: &Library, font: &str, text: &str) -> String {
    let font = lib.font_library().get(font);
    text.chars()
        .map(|c| match c {
            '\n' => c,
            c if c.is_control() => '?',
            c if font.is_some_and(|f| text::glyph_index(f, c).is_some()) => c,
            _ => '?',
        })
        .collect()
}

fn quad(x0: f32, y0: f32, x1: f32, y1: f32) -> Arc<Mesh> {
    Arc::new(Mesh {
        vertices: vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]],
        indices: vec![0, 1, 2, 0, 2, 3],
        paint: Vec::new(),
    })
}

fn item(mesh: Arc<Mesh>, paint: Paint, transform: Mat2d, cxform: Cxform) -> Command {
    Command::Draw(DrawItem { mesh, paint, transform, cxform, blend: BlendMode::Normal })
}

impl Renderer {
    pub fn overlay(
        &mut self,
        lib: &Library,
        font: &str,
        draws: &[OverlayDraw],
        stage: Mat2d,
        viewport: (u32, u32),
        image_base: usize,
    ) -> Vec<Command> {
        if let Some(l) = &self.overlay.last
            && l.draws.as_slice() == draws
            && l.stage == stage
            && l.viewport == viewport
            && l.image_base == image_base
            && l.font == font
        {
            return l.commands.clone();
        }
        let mut out = Vec::new();
        let here = Inherited { xf: stage, cx: Cxform::IDENTITY, blend: BlendMode::Normal, mask: false, slice: None };
        for d in draws {
            match d {
                OverlayDraw::Rect { x, y, w, h, color } => {
                    out.push(item(quad(*x, *y, x + w, y + h), Paint::Solid(*color), stage, Cxform::IDENTITY));
                }
                OverlayDraw::Image { x, y, w, h, image, texels, color } => {
                    let [tw, th] = (*texels).map(|t| t.max(1));
                    let bitmap = BitmapDef { image: image + image_base, rect: [0, 0, tw, th], size: [tw, th] };
                    let matrix = Mat2d::translate(*x, *y).mul(&Mat2d::scale(w / tw as f32, h / th as f32));
                    let paint = Paint::Bitmap { bitmap, matrix, repeat: false, smooth: true };
                    let tint = Cxform { mult: *color, add: [0.0; 4] };
                    out.push(item(quad(*x, *y, x + w, y + h), paint, stage, tint));
                }
                OverlayDraw::Text { text, x, y, size, color, align, shadow } => {
                    let Some(layout) = self.overlay_layout(lib, font, text, *size, *align) else { continue };
                    let at = stage.mul(&Mat2d::translate(*x, *y)).mul(&Mat2d::scale(0.05, 0.05));
                    let mut layers = TextLayers::default();
                    if *shadow {
                        let off = stage.mul(&Mat2d::translate(x + SHADOW_OFFSET, y + SHADOW_OFFSET));
                        let off = off.mul(&Mat2d::scale(0.05, 0.05));
                        let dark = [0.0, 0.0, 0.0, color[3] * SHADOW_ALPHA];
                        self.overlay_glyphs(&layout, &off, Some(dark), &here, &mut layers);
                        layers.emit(&mut out);
                        layers = TextLayers::default();
                    }
                    self.overlay_glyphs(&layout, &at, Some(*color), &here, &mut layers);
                    layers.emit(&mut out);
                }
                OverlayDraw::Fill { color } => {
                    let (w, h) = (viewport.0 as f32, viewport.1 as f32);
                    out.push(item(quad(0.0, 0.0, w, h), Paint::Solid(*color), Mat2d::IDENTITY, Cxform::IDENTITY));
                }
            }
        }
        self.overlay.last = Some(LastList {
            draws: draws.to_vec(),
            stage,
            viewport,
            image_base,
            font: font.to_string(),
            commands: out.clone(),
        });
        out
    }

    fn overlay_glyphs(
        &mut self,
        layout: &TextLayout,
        node: &Mat2d,
        color: Option<[f32; 4]>,
        here: &Inherited,
        layers: &mut TextLayers,
    ) {
        for g in &layout.glyphs {
            let size = g.scale * text::em_units(&g.font);
            let glyph =
                TextGlyph { font: &g.font, index: g.index, size, at: (g.x, g.y), color: color.unwrap_or(g.color) };
            self.text_glyph(&glyph, node, here, None, layers);
        }
    }

    fn overlay_layout(
        &mut self,
        lib: &Library,
        font: &str,
        text: &str,
        size: f32,
        align: Align,
    ) -> Option<Arc<TextLayout>> {
        let key = (text.to_string(), size.to_bits(), align as u8, font.to_string());
        if let Some(l) = self.overlay.layouts.get(&key) {
            return Some(l.clone());
        }
        lib.font_library().get(font)?;
        let shown = drawable(lib, font, text);
        let ch = CharFormat {
            face: Some(font.to_string()),
            size: size * 20.0,
            color: [1.0; 4],
            letter_spacing: 0.0,
            kerning: false,
            bold: false,
            italic: false,
            underline: false,
        };
        let para = ParaFormat { align, left_margin: 0.0, right_margin: 0.0, indent: 0.0, leading: 0.0 };
        let (w, g) = (STAGE.0 * 20.0, text::GUTTER);
        let x0 = match align {
            Align::Center => -w / 2.0,
            Align::Right => -w,
            Align::Left | Align::Justify => 0.0,
        } - g;
        let bounds = RectF { x0, y0: -g, x1: x0 + w + 2.0 * g, y1: STAGE.1 * 20.0 };
        let paras = text::plain(&shown, &ch, &para);
        let layout = Arc::new(text::layout(lib, 0, None, &paras, &ch, bounds, false));
        if self.overlay.layouts.len() >= KEPT_LAYOUTS {
            self.overlay.layouts.clear();
        }
        self.overlay.layouts.insert(key, layout.clone());
        Some(layout)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use ssx_formats::gfx::shape::Shape;
    use ssx_formats::gfx::text::Font;

    use super::*;
    use crate::library::{Character, FrameDef, MovieDef, TimelineDef};

    fn library() -> Library {
        let mut lib = crate::display::tests::empty_lib();
        let font = Font {
            version: 3,
            id: 1,
            flags: Font::WIDE_CODES,
            language: 0,
            name: "Test Font".into(),
            glyphs: vec![Shape::default(); 3],
            codes: vec![u16::from(b'A'), u16::from(b'B'), 0xDC],
            layout: None,
            unk_trailing_layout: None,
        };
        let id = lib.movies().len();
        lib.insert_movie(MovieDef {
            path: "gfxfontlib/na/gfxfontlib.gfx".into(),
            dir: String::new(),
            version: 8,
            frame_rate: 30.0,
            stage: RectF::new(0.0, 0.0, 100.0, 100.0),
            background: None,
            swf_name: "gfxfontlib".into(),
            root: Arc::new(TimelineDef { movie: id, id: 0, frames: vec![FrameDef::default()] }),
            chars: HashMap::from([(1, Character::Font(Arc::new(font)))]),
            exports: HashMap::new(),
            imports: HashMap::new(),
            scaling_grids: HashMap::new(),
        });
        lib.load_font_library("na").unwrap();
        lib
    }

    #[test]
    fn what_the_font_lacks_is_drawn_as_a_question_mark() {
        let lib = library();
        assert_eq!(drawable(&lib, "Test Font", "ÜBA\n\u{7}日"), "ÜBA\n??");
        assert_eq!(drawable(&lib, "No Such Font", "AB"), "??");
    }

    #[test]
    fn rects_fills_and_images_become_commands_and_an_unchanged_list_its_last_ones() {
        let lib = library();
        let mut r = Renderer::new();
        let stage = stage_transform((1920, 1080), super::super::ScaleMode::NoBorder);
        let (x, y) = stage.apply(1280.0, 720.0);
        assert!((x - 1920.0).abs() < 1e-3 && (y - 1080.0).abs() < 1e-3, "the stage covers the viewport: {x} {y}");
        let draws = vec![
            OverlayDraw::Fill { color: [0.0, 0.0, 0.0, 0.5] },
            OverlayDraw::Rect { x: 10.0, y: 10.0, w: 100.0, h: 20.0, color: [1.0, 0.0, 0.0, 1.0] },
            OverlayDraw::Image { x: 0.0, y: 0.0, w: 64.0, h: 64.0, image: 2, texels: [32, 16], color: [1.0; 4] },
            OverlayDraw::Text {
                text: "ÜB".into(),
                x: 640.0,
                y: 100.0,
                size: 24.0,
                color: [1.0; 4],
                align: Align::Center,
                shadow: true,
            },
        ];
        let out = r.overlay(&lib, "Test Font", &draws, stage, (1920, 1080), 5);
        assert_eq!(out.len(), 3);
        let Command::Draw(img) = &out[2] else { panic!("a draw") };
        let Paint::Bitmap { bitmap, matrix, .. } = &img.paint else { panic!("a bitmap") };
        assert_eq!((bitmap.image, bitmap.rect), (7, [0, 0, 32, 16]), "numbered after the library's images");
        let (u, v) = matrix.inverse().unwrap().apply(64.0, 64.0);
        assert!((u - 32.0).abs() < 1e-3 && (v - 16.0).abs() < 1e-3, "the texels stretched over the rect");
        let layout = r.overlay_layout(&lib, "Test Font", "ÜB", 24.0, Align::Center).unwrap();
        assert_eq!(layout.glyphs.len(), 2);
        let again = r.overlay(&lib, "Test Font", &draws, stage, (1920, 1080), 5);
        assert_eq!(again.len(), out.len());
        assert!(matches!((&again[1], &out[1]), (Command::Draw(a), Command::Draw(b)) if Arc::ptr_eq(&a.mesh, &b.mesh)));
    }
}
