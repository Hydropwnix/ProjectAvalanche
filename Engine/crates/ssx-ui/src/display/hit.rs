use std::collections::HashMap;
use std::sync::Arc;

use super::{InstanceId, InstanceKind, Player};
use crate::geom::{Mat2d, RectF};
use crate::library::{CharRef, Character, Library};
use crate::scale9::Slice9;
use crate::shape::{self, Tolerance};
use crate::tess::{Budget, Mesh, OverBudget, Work};

const MAX_MESHES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Key {
    Shape(CharRef),
    Morph(CharRef, u16),
    Glyph(usize, usize),
}

#[derive(Default)]
pub struct HitMeshes {
    map: HashMap<Key, Arc<Mesh>>,
    vertices: u64,
    frame_work: Work,
    pub deferred: u64,
    pub refused: u64,
}

impl HitMeshes {
    pub fn new_frame(&mut self) {
        self.frame_work = Work::default();
    }

    fn get(&mut self, key: Key, make: impl FnOnce(&mut Budget) -> Result<Mesh, OverBudget>) -> Arc<Mesh> {
        if let Some(m) = self.map.get(&key) {
            return m.clone();
        }
        if self.frame_work.reached(&crate::render::FRAME_TESSELLATION) {
            self.deferred += 1;
            return Arc::new(Mesh::default());
        }
        let mut budget = Budget::shape();
        let made = make(&mut budget);
        self.frame_work.add(budget.used());
        let mesh = made.unwrap_or_else(|_| {
            self.refused += 1;
            Mesh::default()
        });
        let n = mesh.vertices.len() as u64;
        if self.map.len() >= MAX_MESHES || self.vertices + n > crate::render::CACHE_VERTICES {
            self.map.clear();
            self.vertices = 0;
        }
        self.vertices += n;
        let m = Arc::new(mesh);
        self.map.insert(key, m.clone());
        m
    }
}

fn in_mesh(mesh: &Mesh, p: (f32, f32)) -> bool {
    let (px, py) = (f64::from(p.0), f64::from(p.1));
    let vertex = |i: u32| mesh.vertices.get(i as usize).copied();
    mesh.indices.as_chunks::<3>().0.iter().any(|t| {
        let (Some(a), Some(b), Some(c)) = (vertex(t[0]), vertex(t[1]), vertex(t[2])) else { return false };
        let side = |u: [f32; 2], v: [f32; 2]| {
            (f64::from(v[0]) - f64::from(u[0])) * (py - f64::from(u[1]))
                - (f64::from(v[1]) - f64::from(u[1])) * (px - f64::from(u[0]))
        };
        let (d0, d1, d2) = (side(a, b), side(b, c), side(c, a));
        (d0 >= 0.0 && d1 >= 0.0 && d2 >= 0.0) || (d0 <= 0.0 && d1 <= 0.0 && d2 <= 0.0)
    })
}

#[derive(Clone, Copy)]
struct Sliced {
    slice: Slice9,
    point: (f32, f32),
    to_grid: Mat2d,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visibility {
    Respect,
    Ignore,
}

pub struct Hit<'a> {
    pub player: &'a Player,
    pub lib: &'a Library,
    pub meshes: &'a mut HitMeshes,
}

impl Player {
    pub fn world_matrix(&self, id: InstanceId) -> Mat2d {
        let mut m = Mat2d::IDENTITY;
        let mut cur = Some(id);
        while let Some(c) = cur {
            let Some(i) = self.get(c) else { break };
            m = i.matrix.mul(&m);
            cur = i.parent;
        }
        m
    }

    pub fn to_local(&self, id: InstanceId, stage: (f32, f32)) -> Option<(f32, f32)> {
        let inv = self.world_matrix(id).inverse()?;
        Some(inv.apply(stage.0, stage.1))
    }
}

impl Hit<'_> {
    pub fn shape(&mut self, id: InstanceId, stage: (f32, f32), vis: Visibility) -> bool {
        let Some(inst) = self.player.get(id) else { return false };
        let parent = inst.parent.map_or(Mat2d::IDENTITY, |p| self.player.world_matrix(p));
        let Some(inv) = parent.inverse() else { return false };
        self.geometry(id, inv.apply(stage.0, stage.1), None, vis)
    }

    pub fn bounds(&self, id: InstanceId, stage: (f32, f32)) -> bool {
        let Some(p) = self.player.to_local(id, stage) else { return false };
        let b = self.player.local_bounds(self.lib, id);
        !b.is_empty() && b.contains(p.0, p.1)
    }

    pub fn drawn(&self, id: InstanceId, stage: (f32, f32)) -> bool {
        let Some(p) = self.player.to_local(id, stage) else { return false };
        let b = drawn_bounds(self.player, self.lib, id);
        !b.is_empty() && b.contains(p.0, p.1)
    }

    pub fn shown_at(&mut self, id: InstanceId, stage: (f32, f32)) -> bool {
        let mut cur = id;
        loop {
            let Some(inst) = self.player.get(cur) else { return false };
            if !inst.visible {
                return false;
            }
            let Some(parent) = inst.parent else { return true };
            let depth = inst.depth;
            for (d, m) in self.player.children(parent) {
                let covers =
                    self.player.get(m).and_then(|i| i.clip_depth).is_some_and(|clip| d < depth && depth <= clip);
                if covers && !self.shape(m, stage, Visibility::Ignore) {
                    return false;
                }
            }
            cur = parent;
        }
    }

    fn geometry(&mut self, id: InstanceId, p: (f32, f32), sliced: Option<Sliced>, vis: Visibility) -> bool {
        let Some(inst) = self.player.get(id) else { return false };
        if vis == Visibility::Respect && !inst.visible {
            return false;
        }
        let Some(local) = inst.matrix.inverse().map(|m| m.apply(p.0, p.1)) else { return false };
        match &inst.kind {
            InstanceKind::Sprite(s) => {
                let sliced = match inst.scale9 {
                    Some(grid) => {
                        let (sx, sy, rest) = Slice9::split(&inst.matrix);
                        let scaled = (sx - 1.0).abs() >= 1e-3 || (sy - 1.0).abs() >= 1e-3;
                        let bounds = self.player.local_bounds(self.lib, id);
                        match rest.inverse() {
                            Some(r) if scaled && !bounds.is_empty() => Some(Sliced {
                                slice: Slice9 { grid, bounds, sx, sy },
                                point: r.apply(p.0, p.1),
                                to_grid: Mat2d::IDENTITY,
                            }),
                            _ => None,
                        }
                    }
                    None => sliced.map(|s| Sliced { to_grid: s.to_grid.mul(&inst.matrix), ..s }),
                };
                let children: Vec<(i32, InstanceId)> = s.children.iter().map(|(d, c)| (*d, *c)).collect();
                let mut masked_to: Option<i32> = None;
                for (depth, c) in children {
                    if masked_to.is_some_and(|m| depth <= m) {
                        continue;
                    }
                    masked_to = None;
                    let Some(clip) = self.player.get(c).and_then(|i| i.clip_depth) else {
                        if self.geometry(c, local, sliced, vis) {
                            return true;
                        }
                        continue;
                    };
                    if !self.geometry(c, local, sliced, Visibility::Ignore) {
                        masked_to = Some(clip);
                    }
                }
                false
            }
            InstanceKind::Shape | InstanceKind::Morph => {
                let Some(r) = inst.character else { return false };
                let ratio = (inst.ratio.clamp(0.0, 1.0) * 65535.0).round() as u16;
                let Some(mesh) = self.character_mesh(r, ratio) else { return false };
                match sliced {
                    Some(s) => in_mesh(&s.slice.slice(&mesh, &s.to_grid.mul(&inst.matrix)), s.point),
                    None => in_mesh(&mesh, local),
                }
            }
            InstanceKind::EditText(t) => !t.bounds.is_empty() && t.bounds.contains(local.0, local.1),
            InstanceKind::Bitmap => {
                let b = self.player.local_bounds(self.lib, id);
                !b.is_empty() && b.contains(local.0, local.1)
            }
            InstanceKind::StaticText => self.static_text(inst.character, local),
        }
    }

    fn character_mesh(&mut self, r: CharRef, ratio: u16) -> Option<Arc<Mesh>> {
        let (real, ch) = self.lib.resolve(r)?;
        let tol = Tolerance::for_scale(1.0 / 20.0);
        Some(match ch {
            Character::Shape(s) => self.meshes.get(Key::Shape(real), |b| {
                shape::coverage(&s.shape.styles, &s.shape.records, s.uses_fill_winding_rule, &tol, b)
            }),
            Character::Morph(ms) => self.meshes.get(Key::Morph(real, ratio), |b| {
                let (styles, records) = shape::morph_at(ms, f32::from(ratio) / 65535.0);
                shape::coverage(&styles, &records, false, &tol, b)
            }),
            _ => return None,
        })
    }

    fn static_text(&mut self, r: Option<CharRef>, p: (f32, f32)) -> bool {
        let Some(r) = r else { return false };
        let Some((_, Character::Text(t))) = self.lib.resolve(r) else { return false };
        let Some(base) = Mat2d::from_swf(&t.matrix).inverse() else { return false };
        let p = base.apply(p.0, p.1);
        let (mut font, mut height) = (None, 240.0f32);
        let (mut x, mut y) = (0.0f32, 0.0f32);
        for rec in &t.records {
            if let Some(fid) = rec.font_id {
                font = self.lib.font_for(r.movie, Some(fid), None);
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
            let k = height / crate::text::em_units(&f);
            for &(g, adv) in &rec.glyphs {
                let index = g as usize;
                let pen = Mat2d::translate(x, y).mul(&Mat2d::scale(k, k));
                x += adv as f32;
                let (Some(inv), Some(glyph)) = (pen.inverse(), f.glyphs.get(index)) else { continue };
                let tol = Tolerance::for_scale(k / 20.0);
                let mesh = self.meshes.get(Key::Glyph(Arc::as_ptr(&f) as usize, index), |b| {
                    shape::glyph_mesh(&glyph.records, tol.curve, b)
                });
                if in_mesh(&mesh, inv.apply(p.0, p.1)) {
                    return true;
                }
            }
        }
        false
    }

    pub fn pick(
        &mut self,
        stage: (f32, f32),
        button: &mut dyn FnMut(InstanceId) -> Option<Option<InstanceId>>,
    ) -> Option<InstanceId> {
        let roots: Vec<InstanceId> = self.player.levels.values().rev().copied().collect();
        roots.into_iter().find_map(|r| self.pick_in(r, stage, button))
    }

    fn pick_in(
        &mut self,
        id: InstanceId,
        stage: (f32, f32),
        button: &mut dyn FnMut(InstanceId) -> Option<Option<InstanceId>>,
    ) -> Option<InstanceId> {
        let inst = self.player.get(id)?;
        if !inst.visible || inst.clip_depth.is_some() {
            return None;
        }
        let InstanceKind::Sprite(s) = &inst.kind else { return None };
        let children: Vec<(i32, InstanceId)> = s.children.iter().rev().map(|(d, c)| (*d, *c)).collect();
        if let Some(area) = button(id) {
            let hit = match area {
                Some(h) => self.shape(h, stage, Visibility::Ignore),
                None => self.shape(id, stage, Visibility::Respect),
            };
            if hit {
                return Some(id);
            }
        }
        let masks: Vec<(i32, i32, InstanceId)> = children
            .iter()
            .filter_map(|&(d, c)| self.player.get(c).and_then(|i| i.clip_depth).map(|clip| (d, clip, c)))
            .collect();
        let mut inside: HashMap<InstanceId, bool> = HashMap::new();
        for (depth, c) in children {
            if masks.iter().any(|&(_, _, m)| m == c) {
                continue;
            }
            let mut shown = true;
            for &(d, clip, m) in &masks {
                if d < depth && depth <= clip {
                    let hit = match inside.get(&m) {
                        Some(h) => *h,
                        None => {
                            let h = self.shape(m, stage, Visibility::Ignore);
                            inside.insert(m, h);
                            h
                        }
                    };
                    shown &= hit;
                }
            }
            if !shown {
                continue;
            }
            if let Some(t) = self.pick_in(c, stage, button) {
                return Some(t);
            }
        }
        None
    }
}

pub fn drawn_bounds(player: &Player, lib: &Library, id: InstanceId) -> RectF {
    fn walk(player: &Player, lib: &Library, id: InstanceId, cx: crate::geom::Cxform, depth: u32) -> RectF {
        use super::BlendMode;
        let Some(inst) = player.get(id) else { return RectF::EMPTY };
        let light = matches!(inst.blend, BlendMode::Add | BlendMode::Screen | BlendMode::Lighten);
        if !inst.visible || inst.clip_depth.is_some() || light || depth > 64 {
            return RectF::EMPTY;
        }
        let cx = cx.concat(&inst.cxform);
        match &inst.kind {
            InstanceKind::Sprite(s) => s.children.values().fold(RectF::EMPTY, |acc, &c| {
                let m = player.get(c).map_or(Mat2d::IDENTITY, |i| i.matrix);
                let b = walk(player, lib, c, cx, depth + 1);
                if b.is_empty() { acc } else { acc.union(&b.transform(&m)) }
            }),
            _ if cx.apply([1.0; 4])[3] <= 1.0 / 255.0 => RectF::EMPTY,
            InstanceKind::EditText(t) => {
                let top = t.bounds.y0 + crate::text::GUTTER;
                t.layout.glyphs.iter().fold(RectF::EMPTY, |acc, g| {
                    let Some(&(y, h)) = t.layout.line_boxes.get(g.line as usize) else { return acc };
                    acc.union(&RectF::new(g.x, top + y, g.x + g.ink_right.max(0.0), top + y + h))
                })
            }
            _ => player.local_bounds(lib, id),
        }
    }
    walk(player, lib, id, crate::geom::Cxform::IDENTITY, 0)
}

pub fn text_char_at(player: &Player, id: InstanceId, stage: (f32, f32)) -> Option<u32> {
    let p = player.to_local(id, stage)?;
    let t = player.get(id)?.text()?;
    let top = t.bounds.y0 + crate::text::GUTTER;
    t.layout
        .glyphs
        .iter()
        .find(|g| {
            let Some(&(y, h)) = t.layout.line_boxes.get(g.line as usize) else { return false };
            let right = g.x + g.advance.max(g.ink_right);
            p.0 >= g.x && p.0 < right && p.1 >= top + y && p.1 < top + y + h
        })
        .map(|g| g.char_index)
}

pub fn stage_bounds(player: &Player, lib: &Library, id: InstanceId) -> RectF {
    let b = player.local_bounds(lib, id);
    if b.is_empty() { b } else { b.transform(&player.world_matrix(id)) }
}

#[cfg(test)]
pub(crate) mod tests;
