use std::collections::HashMap;
use std::sync::Arc;

use ssx_formats::gfx::text::Font;

use crate::display::{BlendMode, Instance, InstanceId, InstanceKind, Player, TextFilter};
use crate::geom::{Cxform, Mat2d, RectF};
use crate::library::{CharRef, Character, Library};
use crate::scale9::Slice9;
use crate::shape::{self, Paint, ShapeMesh, Tolerance};
use crate::tess::Mesh;

#[derive(Clone, Debug)]
pub struct DrawItem {
    pub mesh: Arc<Mesh>,
    pub paint: Paint,
    pub transform: Mat2d,
    pub cxform: Cxform,
    pub blend: BlendMode,
}

#[derive(Clone, Debug)]
pub enum Command {
    Draw(DrawItem),
    MaskBegin,
    MaskEnd,
    MaskPopBegin,
    MaskPopEnd,
    FilterBegin { filter: TextFilter, bounds: RectF, scale: f32 },
    FilterEnd,
}

#[derive(Clone, Debug, Default)]
pub struct RenderList {
    pub commands: Vec<Command>,
    pub viewport: (u32, u32),
    pub background: Option<[f32; 4]>,
}

impl RenderList {
    pub fn draw_count(&self) -> usize {
        self.commands.iter().filter(|c| matches!(c, Command::Draw(_))).count()
    }

    pub fn triangle_count(&self) -> usize {
        self.commands
            .iter()
            .map(|c| match c {
                Command::Draw(d) => d.mesh.triangle_count(),
                _ => 0,
            })
            .sum()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScaleMode {
    NoScale,
    ShowAll,
    ExactFit,
    NoBorder,
}

pub fn stage_transform(stage: RectF, viewport: (u32, u32), mode: ScaleMode) -> Mat2d {
    let (sw, sh) = ((stage.width() / 20.0).max(1.0), (stage.height() / 20.0).max(1.0));
    let (vw, vh) = (viewport.0 as f32, viewport.1 as f32);
    let (kx, ky) = match mode {
        ScaleMode::NoScale => (1.0, 1.0),
        ScaleMode::ExactFit => (vw / sw, vh / sh),
        ScaleMode::ShowAll => {
            let k = (vw / sw).min(vh / sh);
            (k, k)
        }
        ScaleMode::NoBorder => {
            let k = (vw / sw).max(vh / sh);
            (k, k)
        }
    };
    let (ox, oy) = ((vw - sw * kx) / 2.0, (vh - sh * ky) / 2.0);
    Mat2d::translate(ox, oy).mul(&Mat2d::scale(kx / 20.0, ky / 20.0)).mul(&Mat2d::translate(-stage.x0, -stage.y0))
}

// TODO(guess): fixed power-of-two buckets instead of Scaleform's history-dependent reuse band; settle: the mesh cache's reuse test.
fn scale_bucket(scale: f32) -> i32 {
    scale.max(1e-6).log2().round() as i32
}

fn bucket_tolerance(bucket: i32) -> Tolerance {
    Tolerance::for_scale(2f32.powf(bucket as f32))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum MeshKey {
    Shape(CharRef, i32),
    Morph(CharRef, u16, i32),
    Glyph(usize, usize, i32),
    Bitmap(CharRef),
}

#[derive(Default)]
pub struct Renderer {
    meshes: HashMap<MeshKey, Arc<ShapeMesh>>,
    pub tessellated: usize,
    pub glyphs: crate::glyph::GlyphCache,
}

#[derive(Clone, Copy)]
struct Inherited {
    xf: Mat2d,
    cx: Cxform,
    blend: BlendMode,
    mask: bool,
    slice: Option<(Slice9, Mat2d, Mat2d)>,
}

impl Renderer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn build(&mut self, lib: &Library, player: &Player, viewport: (u32, u32), mode: ScaleMode) -> RenderList {
        let mut out = RenderList { commands: Vec::new(), viewport, background: None };
        let Some(root) = player.levels.values().next().and_then(|r| player.get(*r)) else { return out };
        let movie = lib.movie(root.movie);
        out.background = movie.background.map(crate::geom::rgba);
        let stage = stage_transform(movie.stage, viewport, mode);
        for &r in player.levels.values() {
            let inh = Inherited { xf: stage, cx: Cxform::IDENTITY, blend: BlendMode::Normal, mask: false, slice: None };
            self.instance(lib, player, r, inh, &mut out.commands);
        }
        out
    }

    pub fn build_instance(
        &mut self,
        lib: &Library,
        player: &Player,
        id: InstanceId,
        xf: Mat2d,
        out: &mut Vec<Command>,
    ) {
        let inh = Inherited { xf, cx: Cxform::IDENTITY, blend: BlendMode::Normal, mask: false, slice: None };
        self.instance(lib, player, id, inh, out);
    }

    fn instance(&mut self, lib: &Library, player: &Player, id: InstanceId, inh: Inherited, out: &mut Vec<Command>) {
        let Some(inst) = player.get(id) else { return };
        if !inst.visible && !inh.mask {
            return;
        }
        // TODO(guess): an instance with normal (or layer) blending pushes nothing, so the innermost other mode of its ancestors applies; settle: which render-tree nodes push.
        let blend = if inh.mask {
            BlendMode::Normal
        } else if matches!(inst.blend, BlendMode::Normal | BlendMode::Layer) {
            inh.blend
        } else {
            inst.blend
        };
        let here = Inherited {
            xf: inh.xf.mul(&inst.matrix),
            cx: inh.cx.concat(&inst.cxform),
            blend,
            mask: inh.mask,
            slice: None,
        };
        match &inst.kind {
            InstanceKind::Sprite(s) => {
                // TODO(guess): the nine regions' mapping is Flash's model; settle: the consumer of the key data 0x82687D48 fills (kept at the mesh's +0x34, 0x82688BE8 → 0x82687250).
                let slice = match inst.scale9 {
                    Some(grid) => {
                        let (sx, sy, rest) = Slice9::split(&inst.matrix);
                        let scaled = (sx - 1.0).abs() >= 1e-3 || (sy - 1.0).abs() >= 1e-3;
                        let bounds = player.local_bounds(lib, id);
                        (scaled && !bounds.is_empty())
                            .then(|| (Slice9 { grid, bounds, sx, sy }, inh.xf.mul(&rest), Mat2d::IDENTITY))
                    }
                    None => inh.slice.map(|(sl, base, to_grid)| (sl, base, to_grid.mul(&inst.matrix))),
                };
                let here = Inherited { slice, ..here };
                let mut masks: Vec<(i32, InstanceId)> = Vec::new();
                for (&depth, &child) in &s.children {
                    while masks.last().is_some_and(|m| depth > m.0) {
                        let (_, m) = masks.pop().expect("an open mask");
                        self.pop_mask(lib, player, m, here, out);
                    }
                    let Some(c) = player.get(child) else { continue };
                    if let Some(clip) = c.clip_depth.filter(|_| !here.mask) {
                        out.push(Command::MaskBegin);
                        self.instance(lib, player, child, Inherited { mask: true, ..here }, out);
                        out.push(Command::MaskEnd);
                        masks.push((clip, child));
                        continue;
                    }
                    self.instance(lib, player, child, here, out);
                }
                while let Some((_, m)) = masks.pop() {
                    self.pop_mask(lib, player, m, here, out);
                }
            }
            InstanceKind::Shape | InstanceKind::Morph | InstanceKind::Bitmap => {
                let Some(mesh) = self.character_mesh(lib, inst, &here.xf) else { return };
                match &inh.slice {
                    Some((sl, base, to_grid)) => {
                        let sliced = Inherited { xf: *base, ..here };
                        let shape = to_grid.mul(&inst.matrix);
                        for layer in &mesh.layers {
                            let m = Arc::new(sl.slice(&layer.mesh, &shape));
                            let item = DrawItem {
                                mesh: m,
                                paint: layer.paint.clone(),
                                transform: *base,
                                cxform: sliced.cx,
                                blend: sliced.blend,
                            };
                            out.push(Command::Draw(item));
                        }
                    }
                    None => emit(out, &mesh, &here),
                }
            }
            InstanceKind::EditText(t) => {
                let filter = inst.filter.filter(|_| !here.mask);
                let scale = here.xf.max_scale();
                // TODO(guess): the line buffer's flags 0x04 / 0x20 (+0x34, they turn these tests off) are taken as clear; settle: their setters (not among the writers of +0x34 / the view's +0x64).
                let g = crate::text::GUTTER;
                let tr = RectF { x0: t.bounds.x0 + g, y0: t.bounds.y0 + g, x1: t.bounds.x1 - g, y1: t.bounds.y1 - g };
                let limit = tr.height() + 20.0;
                let boxes = &t.layout.line_boxes;
                let visible = boxes
                    .iter()
                    .enumerate()
                    .take_while(|&(i, &(top, h))| if i == 0 { top <= limit } else { top + h <= limit })
                    .count();
                let mut clip = boxes.first().is_some_and(|&(top, h)| h > 0.0 && top <= limit && limit < top + h);
                let mut ended = u32::MAX;
                let mut shown = Vec::with_capacity(t.layout.glyphs.len());
                for gl in &t.layout.glyphs {
                    if gl.line as usize >= visible || gl.line == ended {
                        continue;
                    }
                    let right = gl.x + gl.ink_right;
                    if right <= tr.x0 {
                        continue;
                    }
                    if gl.x >= tr.x1 {
                        ended = gl.line;
                        continue;
                    }
                    clip |= (gl.x < tr.x0 && tr.x0 < right) || (gl.x < tr.x1 && tr.x1 < right);
                    shown.push(gl);
                }
                let clip = clip && !here.mask;
                let b = tr;
                let rect = clip.then(|| {
                    let mesh = Mesh {
                        vertices: vec![[b.x0, b.y0], [b.x1, b.y0], [b.x1, b.y1], [b.x0, b.y1]],
                        indices: vec![0, 1, 2, 0, 2, 3],
                        paint: Vec::new(),
                    };
                    DrawItem {
                        mesh: Arc::new(mesh),
                        paint: Paint::Solid([1.0; 4]),
                        transform: here.xf,
                        cxform: here.cx,
                        blend: BlendMode::Normal,
                    }
                });
                if let Some(r) = &rect {
                    out.extend([Command::MaskBegin, Command::Draw(r.clone()), Command::MaskEnd]);
                }
                if let Some(f) = filter {
                    out.push(Command::FilterBegin { filter: f, bounds: t.bounds.transform(&here.xf), scale });
                }
                for g in shown {
                    let size = g.scale * crate::text::em_units(&g.font);
                    self.text_glyph(&g.font, g.index, size, &here.xf, (g.x, g.y), g.color, &here, out);
                }
                if filter.is_some() {
                    out.push(Command::FilterEnd);
                }
                if let Some(r) = rect {
                    out.extend([Command::MaskPopBegin, Command::Draw(r), Command::MaskPopEnd]);
                }
            }
            InstanceKind::StaticText => self.static_text(lib, inst, here, out),
        }
    }

    fn pop_mask(&mut self, lib: &Library, player: &Player, mask: InstanceId, here: Inherited, out: &mut Vec<Command>) {
        out.push(Command::MaskPopBegin);
        self.instance(lib, player, mask, Inherited { mask: true, ..here }, out);
        out.push(Command::MaskPopEnd);
    }
}

fn emit(out: &mut Vec<Command>, mesh: &ShapeMesh, here: &Inherited) {
    for layer in &mesh.layers {
        out.push(Command::Draw(DrawItem {
            mesh: layer.mesh.clone(),
            paint: layer.paint.clone(),
            transform: here.xf,
            cxform: here.cx,
            blend: here.blend,
        }));
    }
}

impl Renderer {
    fn character_mesh(&mut self, lib: &Library, inst: &Instance, xf: &Mat2d) -> Option<Arc<ShapeMesh>> {
        let r = inst.character?;
        let bucket = scale_bucket(xf.max_scale());
        let key = match inst.kind {
            InstanceKind::Morph => MeshKey::Morph(r, (inst.ratio * 65535.0).round() as u16, bucket),
            InstanceKind::Bitmap => MeshKey::Bitmap(r),
            _ => MeshKey::Shape(r, bucket),
        };
        if let Some(m) = self.meshes.get(&key) {
            return Some(m.clone());
        }
        let tol = bucket_tolerance(bucket);
        let mesh = match lib.resolve(r)?.1 {
            Character::Shape(s) => {
                shape::build(lib, r.movie, &s.shape.styles, &s.shape.records, s.uses_fill_winding_rule, &tol)
            }
            Character::Morph(ms) => {
                let (styles, records) = shape::morph_at(ms, inst.ratio);
                shape::build(lib, r.movie, &styles, &records, false, &tol)
            }
            Character::Bitmap(b) => bitmap_quad(*b),
            _ => return None,
        };
        self.tessellated += 1;
        let mesh = Arc::new(mesh);
        self.meshes.insert(key, mesh.clone());
        Some(mesh)
    }

    fn glyph(&mut self, font: &Arc<Font>, index: usize, scale: f32) -> Arc<ShapeMesh> {
        let bucket = scale_bucket(scale);
        let key = MeshKey::Glyph(Arc::as_ptr(font) as usize, index, bucket);
        if let Some(m) = self.meshes.get(&key) {
            return m.clone();
        }
        let tol = bucket_tolerance(bucket);
        let mesh = font.glyphs.get(index).map(|g| shape::glyph_mesh(&g.records, tol.curve)).unwrap_or_default();
        let layers = if mesh.is_empty() {
            Vec::new()
        } else {
            vec![shape::ShapeLayer { paint: Paint::Solid([1.0; 4]), mesh: Arc::new(mesh) }]
        };
        self.tessellated += 1;
        let mesh = Arc::new(ShapeMesh { layers });
        self.meshes.insert(key, mesh.clone());
        mesh
    }

    // TODO(guess): for a zero-glyph font the font library's font of that name stands in, index for index (never reached on the disc); settle: Scaleform's static text with a substituted font.
    fn static_text(&mut self, lib: &Library, inst: &Instance, here: Inherited, out: &mut Vec<Command>) {
        let Some(r) = inst.character else { return };
        let Some((_, Character::Text(t))) = lib.resolve(r) else { return };
        let base = here.xf.mul(&Mat2d::from_swf(&t.matrix));
        let (mut font, mut color, mut height) = (None, [0.0, 0.0, 0.0, 1.0], 240.0);
        let (mut x, mut y) = (0.0f32, 0.0f32);
        for rec in &t.records {
            if let Some(fid) = rec.font_id {
                font = lib.font_for(r.movie, Some(fid), None);
            }
            if let Some(c) = rec.color {
                color = crate::geom::rgba(c);
            }
            if let Some(h) = rec.height {
                height = f32::from(h);
            }
            if let Some(dx) = rec.x_offset {
                x = f32::from(dx);
            }
            if let Some(dy) = rec.y_offset {
                y = f32::from(dy);
            }
            let Some(f) = font.clone() else { continue };
            for &(g, adv) in &rec.glyphs {
                self.text_glyph(&f, g as usize, height, &base, (x, y), color, &here, out);
                x += adv as f32;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn text_glyph(
        &mut self,
        font: &Arc<Font>,
        index: usize,
        size: f32,
        node: &Mat2d,
        at: (f32, f32),
        color: [f32; 4],
        here: &Inherited,
        out: &mut Vec<Command>,
    ) {
        use crate::glyph::{CachedGlyph, height_ratio, snapped_size};
        let fs = snapped_size(size * height_ratio(node));
        let pen = node.mul(&Mat2d::translate(at.0, at.1));
        match self.glyphs.glyph(font, index, fs) {
            CachedGlyph::Raster(r) => {
                let k = size / (f32::from(fs) * 0.0625).max(1e-5);
                out.push(Command::Draw(DrawItem {
                    mesh: r.quad,
                    paint: Paint::Glyph { raster: r.raster, rect: r.rect, color },
                    transform: pen.mul(&Mat2d::scale(k, k)),
                    cxform: here.cx,
                    blend: here.blend,
                }));
            }
            CachedGlyph::Empty => {}
            CachedGlyph::Vector => {
                let k = size / crate::text::em_units(font);
                let xf = pen.mul(&Mat2d::scale(k, k));
                let mesh = self.glyph(font, index, xf.max_scale());
                for layer in &mesh.layers {
                    out.push(Command::Draw(DrawItem {
                        mesh: layer.mesh.clone(),
                        paint: Paint::Solid(color),
                        transform: xf,
                        cxform: here.cx,
                        blend: here.blend,
                    }));
                }
            }
        }
    }

    pub fn cached(&self) -> usize {
        self.meshes.len()
    }

    pub fn tessellate_all(&mut self, lib: &Library, movie: crate::library::MovieId, scale: f32) -> (usize, usize) {
        let tol = Tolerance::for_scale(scale);
        let (mut n, mut tris) = (0, 0);
        let m = lib.movie(movie);
        let mut ids: Vec<u16> = m.chars.keys().copied().collect();
        ids.sort();
        for id in ids {
            let mesh = match &m.chars[&id] {
                Character::Shape(s) => {
                    shape::build(lib, movie, &s.shape.styles, &s.shape.records, s.uses_fill_winding_rule, &tol)
                }
                Character::Morph(ms) => {
                    let (a, ra) = shape::morph_at(ms, 0.0);
                    let (b, rb) = shape::morph_at(ms, 1.0);
                    let mut mesh = shape::build(lib, movie, &a, &ra, false, &tol);
                    mesh.layers.extend(shape::build(lib, movie, &b, &rb, false, &tol).layers);
                    mesh
                }
                _ => continue,
            };
            n += 1;
            tris += mesh.triangle_count();
        }
        (n, tris)
    }
}

fn bitmap_quad(b: crate::library::BitmapDef) -> ShapeMesh {
    let (w, h) = (b.size[0] as f32 * 20.0, b.size[1] as f32 * 20.0);
    let mesh = Mesh {
        vertices: vec![[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]],
        indices: vec![0, 1, 2, 0, 2, 3],
        paint: Vec::new(),
    };
    let paint = Paint::Bitmap {
        bitmap: b,
        matrix: Mat2d::scale(20.0, 20.0).mul(&b.texel_to_pixel()),
        repeat: false,
        smooth: true,
    };
    ShapeMesh { layers: vec![shape::ShapeLayer { paint, mesh: Arc::new(mesh) }] }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::tests::{empty_lib, movie, po};
    use crate::library::FrameTag;

    #[test]
    fn a_padded_image_maps_the_whole_file_onto_its_target() {
        let b = crate::library::BitmapDef { image: 0, rect: [0, 0, 248, 464], size: [248, 461] };
        let mesh = bitmap_quad(b);
        let l = &mesh.layers[0];
        assert_eq!(l.mesh.vertices[2], [248.0 * 20.0, 461.0 * 20.0]);
        let Paint::Bitmap { matrix, .. } = &l.paint else { panic!("a bitmap paint") };
        let (u, v) = matrix.inverse().expect("invertible").apply(248.0 * 20.0, 461.0 * 20.0);
        assert!((u - 248.0).abs() < 1e-3 && (v - 464.0).abs() < 1e-3, "{u} {v}");
        assert_eq!(crate::library::BitmapDef::of_rect(0, [2, 3, 12, 23]).size, [10, 20]);
    }

    #[test]
    fn clip_layers_bracket_their_depth_range() {
        let mut lib = empty_lib();
        let mut mask = po(1, Some(1), false, 0);
        mask.clip_depth = Some(3);
        mask.flags |= 0x40;
        let m = movie(
            &mut lib,
            vec![vec![
                FrameTag::Place(Arc::new(mask)),
                FrameTag::Place(Arc::new(po(2, Some(2), false, 0))),
                FrameTag::Place(Arc::new(po(5, Some(2), false, 0))),
            ]],
        );
        let mut player = Player::new();
        player.load_level(&lib, m, 0);
        let list = Renderer::new().build(&lib, &player, (1280, 720), ScaleMode::NoBorder);
        let kinds: Vec<&str> = list
            .commands
            .iter()
            .map(|c| match c {
                Command::Draw(_) => "draw",
                Command::MaskBegin => "begin",
                Command::MaskEnd => "end",
                Command::MaskPopBegin => "pop",
                Command::MaskPopEnd => "popend",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, ["begin", "draw", "end", "draw", "pop", "draw", "popend", "draw"]);
    }

    #[test]
    fn no_border_fills_and_centres() {
        let stage = RectF::new(0.0, 0.0, 1280.0 * 20.0, 720.0 * 20.0);
        let m = stage_transform(stage, (1024, 768), ScaleMode::NoBorder);
        let k = 768.0 / 720.0;
        assert!((m.a - k / 20.0).abs() < 1e-6);
        let (x, _) = m.apply(0.0, 0.0);
        assert!((x - (1024.0 - 1280.0 * k) / 2.0).abs() < 1e-3, "{x}");
        let s = stage_transform(stage, (1024, 768), ScaleMode::ShowAll);
        assert!((s.a - 0.8 / 20.0).abs() < 1e-6);
    }
}
