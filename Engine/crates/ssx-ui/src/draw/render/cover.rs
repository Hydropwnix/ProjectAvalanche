use std::collections::HashMap;

use super::{Command, DrawItem, RenderList};
use crate::display::BlendMode;
use crate::library::{BitmapDef, ImageId, Library};
use crate::shape::Paint;

const OPAQUE: f32 = 1.0;
const SLACK: f32 = 1e-3;
const EDGE: f32 = 0.5;

#[derive(Default)]
pub struct OpaqueBitmaps(HashMap<(ImageId, [u32; 4]), bool>);

impl OpaqueBitmaps {
    pub fn is_opaque(&mut self, lib: &Library, b: &BitmapDef) -> bool {
        *self.0.entry((b.image, b.rect)).or_insert_with(|| {
            let Some(img) = lib.images().get(b.image) else { return false };
            let (w, h) = (img.width.min(b.rect[2]), img.height.min(b.rect[3]));
            let (x0, y0) = (b.rect[0], b.rect[1]);
            if x0 >= w || y0 >= h {
                return false;
            }
            (y0..h).all(|y| {
                let row = (y as usize * img.width as usize + x0 as usize) * 4;
                let end = (y as usize * img.width as usize + w as usize) * 4;
                img.rgba.get(row..end).is_some_and(|px| px.as_chunks::<4>().0.iter().all(|p| p[3] == 255))
            })
        })
    }
}

impl RenderList {
    pub fn shows_beside_stage(&self) -> bool {
        let (w, h) = (self.viewport.0 as f32, self.viewport.1 as f32);
        self.stage.is_some_and(|[x0, y0, x1, y1]| x0 > EDGE || y0 > EDGE || x1 < w - EDGE || y1 < h - EDGE)
    }

    pub fn covers_stage(&self, lib: &Library, bitmaps: &mut OpaqueBitmaps) -> bool {
        let Some(stage) = self.stage else { return false };
        unmasked(&self.commands).any(|i| {
            let Command::Draw(d) = &self.commands[i] else { return false };
            opaque(d, lib, bitmaps) && rectangle(d).is_some_and(|b| contains(b, stage))
        })
    }

    pub fn extend_backdrops(&mut self) {
        let Some(stage) = self.stage else { return };
        let window = [0.0, 0.0, self.viewport.0 as f32, self.viewport.1 as f32];
        let draws: Vec<usize> = unmasked(&self.commands).collect();
        for i in draws {
            let Command::Draw(d) = &mut self.commands[i] else { continue };
            let continues = match d.paint {
                Paint::Solid(_) => true,
                Paint::Gradient(_) => d.mesh.paint.is_empty(),
                Paint::Bitmap { .. } | Paint::Glyph { .. } => false,
            };
            if !continues {
                continue;
            }
            let Some(b) = rectangle(d).filter(|&b| contains(b, stage)) else { continue };
            if let Some(mesh) = extended(d, b, window) {
                d.mesh = std::sync::Arc::new(mesh);
            }
        }
    }
}

fn unmasked(commands: &[Command]) -> impl Iterator<Item = usize> + '_ {
    let (mut writing, mut open) = (false, 0u32);
    commands.iter().enumerate().filter_map(move |(i, c)| {
        match c {
            Command::MaskBegin | Command::MaskPopBegin => writing = true,
            Command::MaskEnd => {
                writing = false;
                open += 1;
            }
            Command::MaskPopEnd => {
                writing = false;
                open = open.saturating_sub(1);
            }
            Command::Draw(_) if !writing && open == 0 => return Some(i),
            Command::Draw(_) => {}
        }
        None
    })
}

fn opaque(d: &DrawItem, lib: &Library, bitmaps: &mut OpaqueBitmaps) -> bool {
    let alpha = |a: f32| d.cxform.apply([1.0, 1.0, 1.0, a])[3] >= OPAQUE;
    d.blend == BlendMode::Normal
        && match &d.paint {
            Paint::Solid(c) => alpha(c[3]),
            Paint::Gradient(g) => !g.stops.is_empty() && g.stops.iter().all(|(_, c)| alpha(c[3])),
            Paint::Bitmap { bitmap, .. } => alpha(1.0) && bitmaps.is_opaque(lib, bitmap),
            Paint::Glyph { .. } => false,
        }
}

fn rectangle(d: &DrawItem) -> Option<[f32; 4]> {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
    let mut area = 0.0f32;
    for t in d.mesh.indices.as_chunks::<3>().0 {
        let [Some(a), Some(b), Some(c)] = t.map(|i| d.mesh.vertices.get(i as usize)) else { return None };
        let [a, b, c] = [a, b, c].map(|v| d.transform.apply(v[0], v[1]));
        area += ((b.0 - a.0) * (c.1 - a.1) - (c.0 - a.0) * (b.1 - a.1)).abs() / 2.0;
        for q in [a, b, c] {
            (x0, y0, x1, y1) = (x0.min(q.0), y0.min(q.1), x1.max(q.0), y1.max(q.1));
        }
    }
    let bounds = (x1 - x0) * (y1 - y0);
    (bounds.is_finite() && bounds > 0.0 && (area - bounds).abs() <= SLACK * bounds).then_some([x0, y0, x1, y1])
}

fn contains(b: [f32; 4], r: [f32; 4]) -> bool {
    b[0] <= r[0] + EDGE && b[1] <= r[1] + EDGE && b[2] >= r[2] - EDGE && b[3] >= r[3] - EDGE
}

fn extended(d: &DrawItem, b: [f32; 4], window: [f32; 4]) -> Option<crate::tess::Mesh> {
    let inverse = d.transform.inverse()?;
    let (tx, ty) = (SLACK * (b[2] - b[0]), SLACK * (b[3] - b[1]));
    let out = |p: f32, lo: f32, hi: f32, w_lo: f32, w_hi: f32, tol: f32| {
        if (p - lo).abs() <= tol && lo > w_lo {
            w_lo
        } else if (p - hi).abs() <= tol && hi < w_hi {
            w_hi
        } else {
            p
        }
    };
    let vertices = d
        .mesh
        .vertices
        .iter()
        .map(|v| {
            let (x, y) = d.transform.apply(v[0], v[1]);
            let p = (out(x, b[0], b[2], window[0], window[2], tx), out(y, b[1], b[3], window[1], window[3], ty));
            if p == (x, y) {
                *v
            } else {
                let (sx, sy) = inverse.apply(p.0, p.1);
                [sx, sy]
            }
        })
        .collect();
    Some(crate::tess::Mesh { vertices, indices: d.mesh.indices.clone(), paint: d.mesh.paint.clone() })
}

#[cfg(test)]
mod tests;
