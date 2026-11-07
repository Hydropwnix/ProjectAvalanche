use crate::display::BlendMode;
use crate::geom::{Mat2d, RectF};
use crate::library::Library;
use crate::render::{Command, DrawItem, RenderList};
use crate::shape::{GradientKind, Paint};

pub struct Canvas {
    pub width: usize,
    pub height: usize,
    pub px: Vec<[f32; 4]>,
}

impl Canvas {
    pub fn to_rgba8(&self) -> Vec<u8> {
        self.px
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 1.0].map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8))
            .collect()
    }
}

const SS: usize = 1;

fn coverage(item: &DrawItem, w: usize, h: usize) -> Vec<(usize, u16, usize)> {
    let pts: Vec<(f32, f32)> = item.mesh.vertices.iter().map(|v| item.transform.apply(v[0], v[1])).collect();
    let mut masks: std::collections::HashMap<usize, (u16, usize)> = std::collections::HashMap::new();
    for (ti, t) in item.mesh.indices.as_chunks::<3>().0.iter().enumerate() {
        let [a, b, c] = t.map(|i| pts[i as usize]);
        let area = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
        if area == 0.0 || !area.is_finite() {
            continue;
        }
        let (x0, x1) = (a.0.min(b.0).min(c.0).floor().max(0.0), a.0.max(b.0).max(c.0).ceil().min(w as f32));
        let (y0, y1) = (a.1.min(b.1).min(c.1).floor().max(0.0), a.1.max(b.1).max(c.1).ceil().min(h as f32));
        if x0 >= x1 || y0 >= y1 {
            continue;
        }
        let s = area.signum();
        let inside = |p: (f32, f32), q: (f32, f32), x: f32, y: f32| {
            let e = ((q.0 - p.0) * (y - p.1) - (q.1 - p.1) * (x - p.0)) * s;
            e > 0.0 || (e == 0.0 && ((q.1 - p.1) * s > 0.0 || (q.1 == p.1 && (q.0 - p.0) * s < 0.0)))
        };
        for py in y0 as usize..y1 as usize {
            for px in x0 as usize..x1 as usize {
                let mut m = 0u16;
                for sy in 0..SS {
                    let y = py as f32 + (sy as f32 + 0.5) / SS as f32;
                    for sx in 0..SS {
                        let x = px as f32 + (sx as f32 + 0.5) / SS as f32;
                        if inside(a, b, x, y) && inside(b, c, x, y) && inside(c, a, x, y) {
                            m |= 1 << (sy * SS + sx);
                        }
                    }
                }
                if m != 0 {
                    masks.entry(py * w + px).or_insert((0, ti)).0 |= m;
                }
            }
        }
    }
    masks.into_iter().map(|(i, (m, t))| (i, m, t)).collect()
}

fn paint_coord(item: &DrawItem, inv: &Mat2d, tri: usize, x: f32, y: f32) -> (f32, f32) {
    let m = &item.mesh;
    if m.paint.is_empty() {
        return inv.apply(x, y);
    }
    let t = m.indices.as_chunks::<3>().0[tri];
    let [a, b, c] = t.map(|i| item.transform.apply(m.vertices[i as usize][0], m.vertices[i as usize][1]));
    let [pa, pb, pc] = t.map(|i| m.paint_at(i as usize));
    let det = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
    if det.abs() < 1e-12 {
        return (pa[0], pa[1]);
    }
    let u = ((x - a.0) * (c.1 - a.1) - (y - a.1) * (c.0 - a.0)) / det;
    let v = ((b.0 - a.0) * (y - a.1) - (b.1 - a.1) * (x - a.0)) / det;
    (pa[0] + u * (pb[0] - pa[0]) + v * (pc[0] - pa[0]), pa[1] + u * (pb[1] - pa[1]) + v * (pc[1] - pa[1]))
}

fn lerp4(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}

fn sample(lib: &Library, item: &DrawItem, (sx, sy): (f32, f32)) -> [f32; 4] {
    match &item.paint {
        Paint::Solid(c) => *c,
        Paint::Gradient(g) => {
            let Some(gi) = g.matrix.inverse() else { return [0.0; 4] };
            let (gx, gy) = gi.apply(sx, sy);
            let t = match g.kind {
                GradientKind::Linear => (gx + 16384.0) / 32768.0,
                GradientKind::Radial => gx.hypot(gy) / 16384.0,
                GradientKind::Focal(f) => focal_t(gx / 16384.0, gy / 16384.0, f),
            };
            g.sample(t)
        }
        Paint::Bitmap { bitmap, matrix, repeat, smooth } => {
            let Some(bi) = matrix.inverse() else { return [0.0; 4] };
            let (u, v) = bi.apply(sx, sy);
            let img = lib.image(bitmap.image);
            let [x0, y0, _, _] = bitmap.rect;
            let (bw, bh) = (bitmap.width() as f32, bitmap.height() as f32);
            let texel = |tx: f32, ty: f32| -> [f32; 4] {
                let (tx, ty) = if *repeat {
                    (tx.rem_euclid(bw), ty.rem_euclid(bh))
                } else {
                    (tx.clamp(0.0, bw - 1.0), ty.clamp(0.0, bh - 1.0))
                };
                let (ix, iy) = ((x0 + tx as u32).min(img.width - 1), (y0 + ty as u32).min(img.height - 1));
                let o = ((iy * img.width + ix) * 4) as usize;
                [0, 1, 2, 3].map(|k| f32::from(img.rgba[o + k]) / 255.0)
            };
            if !*smooth {
                return texel(u.floor(), v.floor());
            }
            let (fu, fv) = (u - 0.5, v - 0.5);
            let (iu, iv) = (fu.floor(), fv.floor());
            let (du, dv) = (fu - iu, fv - iv);
            lerp4(
                lerp4(texel(iu, iv), texel(iu + 1.0, iv), du),
                lerp4(texel(iu, iv + 1.0), texel(iu + 1.0, iv + 1.0), du),
                dv,
            )
        }
        Paint::Glyph { color, .. } => *color,
    }
}

pub fn glyph_coverage(r: &crate::glyph::GlyphRaster, u: f32, v: f32) -> f32 {
    if r.width == 0 || r.height == 0 {
        return 0.0;
    }
    let (w, h) = (r.width as f32, r.height as f32);
    let (tu, tv) = ((u - r.x0 as f32).clamp(0.5, w - 0.5) - 0.5, (v - r.y0 as f32).clamp(0.5, h - 0.5) - 0.5);
    let (iu, iv) = (tu.floor(), tv.floor());
    let (du, dv) = (tu - iu, tv - iv);
    let at = |x: f32, y: f32| {
        let (x, y) = ((x as u32).min(r.width - 1), (y as u32).min(r.height - 1));
        f32::from(r.alpha[(y * r.width + x) as usize]) / 255.0
    };
    let top = at(iu, iv) + (at(iu + 1.0, iv) - at(iu, iv)) * du;
    let bottom = at(iu, iv + 1.0) + (at(iu + 1.0, iv + 1.0) - at(iu, iv + 1.0)) * du;
    top + (bottom - top) * dv
}

pub fn focal_t(x: f32, y: f32, f: f32) -> f32 {
    let (dx, dy) = (x - f, y);
    let a = dx * dx + dy * dy;
    if a <= 0.0 {
        return 0.0;
    }
    let b = 2.0 * (f * dx);
    let c = f * f - 1.0;
    let k = (-b + (b * b - 4.0 * a * c).max(0.0).sqrt()) / (2.0 * a);
    if k <= 0.0 { 1.0 } else { 1.0 / k }
}

pub fn rasterize(lib: &Library, list: &RenderList, background: [f32; 3]) -> Canvas {
    let (w, h) = (list.viewport.0 as usize, list.viewport.1 as usize);
    let mut canvas =
        Canvas { width: w, height: h, px: vec![[background[0], background[1], background[2], 1.0]; w * h] };
    let full = vec![u16::MAX; w * h];
    let mut masks: Vec<Vec<u16>> = Vec::new();
    enum Mode {
        Draw,
        Write(Vec<u16>),
        Erase,
    }
    let mut mode = Mode::Draw;
    for cmd in &list.commands {
        match cmd {
            Command::MaskBegin => mode = Mode::Write(vec![0; w * h]),
            Command::MaskEnd => {
                if let Mode::Write(m) = std::mem::replace(&mut mode, Mode::Draw) {
                    let top = masks.last().unwrap_or(&full);
                    masks.push(m.iter().zip(top).map(|(a, b)| a & b).collect());
                }
            }
            Command::MaskPopBegin => mode = Mode::Erase,
            Command::MaskPopEnd => {
                masks.pop();
                mode = Mode::Draw;
            }
            Command::Draw(item) => match &mut mode {
                Mode::Erase => {}
                Mode::Write(m) => {
                    for (i, c, _) in coverage(item, w, h) {
                        m[i] |= c;
                    }
                }
                Mode::Draw => {
                    let Some(inv) = item.transform.inverse() else { continue };
                    let clip = masks.last().unwrap_or(&full);
                    for (i, c, tri) in coverage(item, w, h) {
                        let k = (c & clip[i]).count_ones() as f32 / (SS * SS) as f32;
                        if k <= 0.0 {
                            continue;
                        }
                        let (x, y) = ((i % w) as f32 + 0.5, (i / w) as f32 + 0.5);
                        let q = paint_coord(item, &inv, tri, x, y);
                        let s = item.cxform.apply(sample(lib, item, q));
                        let k = match &item.paint {
                            Paint::Glyph { raster, .. } => k * glyph_coverage(raster, q.0, q.1),
                            _ => k,
                        };
                        let a = s[3] * k;
                        let src = [s[0] * a, s[1] * a, s[2] * a, a];
                        let d = &mut canvas.px[i];
                        let white = [s[0] * a + 1.0 - a, s[1] * a + 1.0 - a, s[2] * a + 1.0 - a];
                        *d = match item.blend {
                            BlendMode::Multiply => [white[0] * d[0], white[1] * d[1], white[2] * d[2], d[3]],
                            BlendMode::Darken => [white[0].min(d[0]), white[1].min(d[1]), white[2].min(d[2]), d[3]],
                            BlendMode::Lighten => [src[0].max(d[0]), src[1].max(d[1]), src[2].max(d[2]), d[3]],
                            BlendMode::Add => std::array::from_fn(|k| (src[k] + d[k]).min(1.0)),
                            BlendMode::Subtract => std::array::from_fn(|k| (d[k] - src[k]).max(0.0)),
                            BlendMode::Alpha | BlendMode::Erase => *d,
                            _ => std::array::from_fn(|k| src[k] + d[k] * (1.0 - src[3])),
                        };
                    }
                }
            },
        }
    }
    canvas
}

pub fn drawn_bounds(list: &RenderList) -> RectF {
    let mut r = RectF::EMPTY;
    for c in &list.commands {
        if let Command::Draw(d) = c {
            for v in &d.mesh.vertices {
                let (x, y) = d.transform.apply(v[0], v[1]);
                r.add_point(x, y);
            }
        }
    }
    r
}
