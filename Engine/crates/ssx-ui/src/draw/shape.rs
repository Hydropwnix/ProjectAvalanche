use std::collections::BTreeMap;
use std::sync::Arc;

use ssx_formats::gfx::shape::{FillStyle, Gradient, LineStyle, MorphShape, ShapeRecord, Styles};
use ssx_formats::gfx::types::{Matrix, Rgba};

use crate::geom::{Mat2d, rgba};
use crate::library::{BitmapDef, CharRef, Character, Library, MovieId};
use crate::tess::{self, Cap, Edge, FillRule, Join, Mesh, StrokeStyle};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GradientKind {
    Linear,
    Radial,
    Focal(f32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct GradientPaint {
    pub kind: GradientKind,
    pub matrix: Mat2d,
    pub spread: u8,
    pub interpolation: u8,
    pub stops: Vec<(f32, [f32; 4])>,
}

impl GradientPaint {
    // TODO(guess): stops interpolated linearly in the stored (sRGB) values (all 142 gradients have mode 0); settle: Scaleform's gradient images (0x820C81C0) — size and filtering.
    pub fn sample(&self, t: f32) -> [f32; 4] {
        let t = match self.spread {
            1 => {
                let m = t.rem_euclid(2.0);
                if m > 1.0 { 2.0 - m } else { m }
            }
            2 => t.rem_euclid(1.0),
            _ => t.clamp(0.0, 1.0),
        };
        let Some(first) = self.stops.first() else { return [0.0; 4] };
        if t <= first.0 {
            return first.1;
        }
        for w in self.stops.windows(2) {
            if t <= w[1].0 {
                let span = (w[1].0 - w[0].0).max(1e-6);
                let k = (t - w[0].0) / span;
                return std::array::from_fn(|i| w[0].1[i] + (w[1].1[i] - w[0].1[i]) * k);
            }
        }
        self.stops.last().map_or([0.0; 4], |s| s.1)
    }

    pub fn ramp(&self) -> Vec<[u8; 4]> {
        (0..256)
            .map(|i| self.sample((i as f32 + 0.5) / 256.0).map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8))
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Paint {
    Solid([f32; 4]),
    Gradient(Arc<GradientPaint>),
    Bitmap { bitmap: BitmapDef, matrix: Mat2d, repeat: bool, smooth: bool },
    Glyph { raster: Arc<crate::glyph::GlyphRaster>, rect: crate::glyph::AtlasRect, color: [f32; 4] },
}

#[derive(Clone, Debug)]
pub struct ShapeLayer {
    pub paint: Paint,
    pub mesh: Arc<Mesh>,
}

#[derive(Clone, Debug, Default)]
pub struct ShapeMesh {
    pub layers: Vec<ShapeLayer>,
}

impl ShapeMesh {
    pub fn triangle_count(&self) -> usize {
        self.layers.iter().map(|l| l.mesh.triangle_count()).sum()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Tolerance {
    pub curve: f32,
    pub min_line: f32,
}

pub const CURVE_ERROR_PX: f32 = 1.0;

impl Tolerance {
    pub fn for_scale(scale: f32) -> Tolerance {
        let s = scale.max(1e-4);
        // TODO(guess): strokes thinner than a pixel drawn one pixel wide (Flash hairlines); settle: the renderer's stroke mode (hairline / normal / correct).
        Tolerance { curve: CURVE_ERROR_PX / s, min_line: 1.0 / s }
    }
}

fn gradient_paint(kind: u8, matrix: &Matrix, g: &Gradient) -> GradientPaint {
    GradientPaint {
        kind: match kind {
            0x10 => GradientKind::Linear,
            0x13 => GradientKind::Focal(g.focal.map_or(0.0, |f| f32::from(f) / 256.0)),
            _ => GradientKind::Radial,
        },
        matrix: Mat2d::from_swf(matrix),
        spread: g.spread,
        interpolation: g.interpolation,
        stops: g.records.iter().map(|(r, c)| (f32::from(*r) / 255.0, rgba(*c))).collect(),
    }
}

pub fn fill_paint(lib: &Library, movie: MovieId, style: &FillStyle) -> Option<Paint> {
    Some(match style {
        FillStyle::Solid(c) => Paint::Solid(rgba(*c)),
        FillStyle::Gradient { kind, matrix, gradient } => {
            Paint::Gradient(Arc::new(gradient_paint(*kind, matrix, gradient)))
        }
        FillStyle::Bitmap { kind, bitmap_id, matrix } => {
            let bitmap = match lib.resolve(CharRef { movie, id: *bitmap_id }) {
                Some((_, Character::Bitmap(b))) => *b,
                _ => return None,
            };
            let matrix = Mat2d::from_swf(matrix).mul(&bitmap.texel_to_pixel());
            Paint::Bitmap { bitmap, matrix, repeat: kind & 1 == 0, smooth: kind & 2 == 0 }
        }
    })
}

fn line_style(l: &LineStyle, tol: &Tolerance) -> StrokeStyle {
    let cap = |c: u8| match c {
        1 => Cap::None,
        2 => Cap::Square,
        _ => Cap::Round,
    };
    StrokeStyle {
        width: f32::from(l.width).max(tol.min_line),
        start_cap: cap(l.start_cap),
        end_cap: cap(l.end_cap),
        join: match l.join {
            1 => Join::Bevel,
            2 => Join::Miter(l.miter_limit.map_or(3.0, |m| f32::from(m) / 256.0)),
            _ => Join::Round,
        },
    }
}

#[derive(Default)]
struct StyleSet {
    fills: BTreeMap<u32, Vec<Edge>>,
    lines: BTreeMap<u32, Vec<Vec<[f32; 2]>>>,
}

fn collect<'a>(styles: &'a Styles, records: &'a [ShapeRecord], tol: f32) -> Vec<(&'a Styles, StyleSet)> {
    let mut sets: Vec<(&Styles, StyleSet)> = vec![(styles, StyleSet::default())];
    let (mut x, mut y) = (0f32, 0f32);
    let (mut f0, mut f1, mut ln) = (0u32, 0u32, 0u32);
    let mut line_open = false;
    let mut pts: Vec<[f32; 2]> = Vec::new();
    for r in records {
        let start = [x, y];
        match r {
            ShapeRecord::StyleChange { move_to, fill0, fill1, line, new_styles } => {
                if let Some(ns) = new_styles {
                    sets.push((ns, StyleSet::default()));
                    (f0, f1, ln) = (0, 0, 0);
                }
                f0 = fill0.unwrap_or(f0);
                f1 = fill1.unwrap_or(f1);
                if line.is_some_and(|l| l != ln) || new_styles.is_some() {
                    line_open = false;
                }
                ln = line.unwrap_or(ln);
                if let Some((mx, my)) = move_to {
                    (x, y) = (*mx as f32, *my as f32);
                    line_open = false;
                }
                continue;
            }
            ShapeRecord::Straight { dx, dy } => {
                pts.clear();
                (x, y) = (x + *dx as f32, y + *dy as f32);
                pts.push([x, y]);
            }
            ShapeRecord::Curved { control_dx, control_dy, anchor_dx, anchor_dy } => {
                pts.clear();
                let c = [x + *control_dx as f32, y + *control_dy as f32];
                let a = [c[0] + *anchor_dx as f32, c[1] + *anchor_dy as f32];
                tess::flatten_quad([x, y], c, a, tol, &mut pts);
                (x, y) = (a[0], a[1]);
            }
        }
        let set = &mut sets.last_mut().expect("a style set").1;
        for (f, reverse) in [(f1, false), (f0, true)] {
            if f > 0 {
                let edges = set.fills.entry(f).or_default();
                let mut prev = start;
                for &p in &pts {
                    edges.push(if reverse { Edge { from: p, to: prev } } else { Edge { from: prev, to: p } });
                    prev = p;
                }
            }
        }
        if ln > 0 {
            let polys = set.lines.entry(ln).or_default();
            if !line_open || polys.is_empty() {
                polys.push(vec![start]);
            }
            polys.last_mut().expect("a polyline").extend_from_slice(&pts);
            line_open = true;
        } else {
            line_open = false;
        }
    }
    sets
}

pub fn build(
    lib: &Library,
    movie: MovieId,
    styles: &Styles,
    records: &[ShapeRecord],
    non_zero: bool,
    tol: &Tolerance,
) -> ShapeMesh {
    let rule = if non_zero { FillRule::NonZero } else { FillRule::EvenOdd };
    let mut out = ShapeMesh::default();
    for (styles, set) in collect(styles, records, tol.curve) {
        for (f, edges) in &set.fills {
            let Some(style) = styles.fills.get(*f as usize - 1) else { continue };
            let Some(paint) = fill_paint(lib, movie, style) else { continue };
            let mesh = tess::fill(edges, rule);
            if !mesh.is_empty() {
                out.layers.push(ShapeLayer { paint, mesh: Arc::new(mesh) });
            }
        }
        for (l, polys) in &set.lines {
            let Some(style) = styles.lines.get(*l as usize - 1) else { continue };
            let paint = match &style.fill {
                Some(f) => fill_paint(lib, movie, f),
                None => Some(Paint::Solid(rgba(style.color))),
            };
            let Some(paint) = paint else { continue };
            let mesh = tess::stroke(polys, &line_style(style, tol), tol.curve);
            if !mesh.is_empty() {
                out.layers.push(ShapeLayer { paint, mesh: Arc::new(mesh) });
            }
        }
    }
    out
}

pub fn glyph_mesh(records: &[ShapeRecord], tol: f32) -> Mesh {
    let styles = Styles { fills: vec![FillStyle::Solid(Rgba::default())], lines: Vec::new() };
    let mut mesh = Mesh::default();
    for (_, set) in collect(&styles, records, tol) {
        let edges: Vec<Edge> = set.fills.values().flatten().copied().collect();
        mesh.append(&tess::fill(&edges, FillRule::EvenOdd));
    }
    mesh
}

fn lerp_rgba(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let l = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Rgba { r: l(a.r, b.r), g: l(a.g, b.g), b: l(a.b, b.b), a: l(a.a, b.a) }
}

fn lerp_matrix(a: &Matrix, b: &Matrix, t: f32) -> Matrix {
    let l = |x: f32, y: f32| x + (y - x) * t;
    let li = |x: i32, y: i32| (x as f32 + (y - x) as f32 * t).round() as i32;
    Matrix { a: l(a.a, b.a), b: l(a.b, b.b), c: l(a.c, b.c), d: l(a.d, b.d), tx: li(a.tx, b.tx), ty: li(a.ty, b.ty) }
}

// TODO(guess): pairing and the straight-as-curve rule follow the SWF morph model; settle: Scaleform's morph tessellation (`GFxMorphCharacterDef`, loader 0x820CAA84).
pub fn morph_at(ms: &MorphShape, t: f32) -> (Styles, Vec<ShapeRecord>) {
    let fills = ms
        .fills
        .iter()
        .map(|f| match (&f.start, &f.end) {
            (FillStyle::Solid(a), FillStyle::Solid(b)) => FillStyle::Solid(lerp_rgba(*a, *b, t)),
            (
                FillStyle::Gradient { kind, matrix: m0, gradient: g0 },
                FillStyle::Gradient { matrix: m1, gradient: g1, .. },
            ) => {
                let mut g = g0.clone();
                for (r, (s, e)) in g.records.iter_mut().zip(g0.records.iter().zip(&g1.records)) {
                    *r = (
                        (f32::from(s.0) + (f32::from(e.0) - f32::from(s.0)) * t).round() as u8,
                        lerp_rgba(s.1, e.1, t),
                    );
                }
                FillStyle::Gradient { kind: *kind, matrix: lerp_matrix(m0, m1, t), gradient: g }
            }
            (FillStyle::Bitmap { kind, bitmap_id, matrix: m0 }, FillStyle::Bitmap { matrix: m1, .. }) => {
                FillStyle::Bitmap { kind: *kind, bitmap_id: *bitmap_id, matrix: lerp_matrix(m0, m1, t) }
            }
            (s, _) => s.clone(),
        })
        .collect();
    let lines = ms
        .lines
        .iter()
        .map(|l| LineStyle {
            width: (f32::from(l.start_width) + (f32::from(l.end_width) - f32::from(l.start_width)) * t).round() as u16,
            color: lerp_rgba(l.start_color, l.end_color, t),
            start_cap: 0,
            join: 0,
            no_h_scale: false,
            no_v_scale: false,
            pixel_hinting: false,
            unk_reserved: 0,
            no_close: false,
            end_cap: 0,
            miter_limit: None,
            fill: None,
        })
        .collect();
    let curve = |r: &ShapeRecord| match *r {
        ShapeRecord::Straight { dx, dy } => Some([dx / 2, dy / 2, dx - dx / 2, dy - dy / 2]),
        ShapeRecord::Curved { control_dx, control_dy, anchor_dx, anchor_dy } => {
            Some([control_dx, control_dy, anchor_dx, anchor_dy])
        }
        _ => None,
    };
    let mut end = ms
        .end_edges
        .iter()
        .filter(|r| curve(r).is_some() || matches!(r, ShapeRecord::StyleChange { move_to: Some(_), .. }))
        .peekable();
    let li = |a: i32, b: i32| (a as f32 + (b - a) as f32 * t).round() as i32;
    let mut records = Vec::new();
    for r in &ms.start_edges {
        match r {
            ShapeRecord::StyleChange { move_to, fill0, fill1, line, .. } => {
                let mv = move_to.map(|(x0, y0)| match end.peek() {
                    Some(ShapeRecord::StyleChange { move_to: Some((x1, y1)), .. }) => {
                        let p = (li(x0, *x1), li(y0, *y1));
                        end.next();
                        p
                    }
                    _ => (x0, y0),
                });
                records.push(ShapeRecord::StyleChange {
                    move_to: mv,
                    fill0: *fill0,
                    fill1: *fill1,
                    line: *line,
                    new_styles: None,
                });
            }
            edge => {
                let s = curve(edge).unwrap_or_default();
                while matches!(end.peek(), Some(ShapeRecord::StyleChange { .. })) {
                    end.next();
                }
                let e = end.next().and_then(curve).unwrap_or(s);
                records.push(ShapeRecord::Curved {
                    control_dx: li(s[0], e[0]),
                    control_dy: li(s[1], e[1]),
                    anchor_dx: li(s[2], e[2]),
                    anchor_dy: li(s[3], e[3]),
                });
            }
        }
    }
    (Styles { fills, lines }, records)
}

pub fn region_areas(styles: &Styles, records: &[ShapeRecord], non_zero: bool, tol: f32) -> Vec<(usize, u32, f64, f64)> {
    let rule = if non_zero { FillRule::NonZero } else { FillRule::EvenOdd };
    let mut out = Vec::new();
    for (set_index, (_, set)) in collect(styles, records, tol).into_iter().enumerate() {
        for (f, edges) in &set.fills {
            let signed: f64 = edges
                .iter()
                .map(|e| f64::from(e.from[0]) * f64::from(e.to[1]) - f64::from(e.to[0]) * f64::from(e.from[1]))
                .sum::<f64>()
                / 2.0;
            out.push((set_index, *f, signed.abs(), tess::fill(edges, rule).area()));
        }
    }
    out
}
