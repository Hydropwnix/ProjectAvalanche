use std::collections::HashMap;
use std::sync::Arc;

use ssx_formats::gfx::shape::ShapeRecord;
use ssx_formats::gfx::text::Font;

use crate::geom::Mat2d;
use crate::tess::Mesh;

pub const TEXTURE_SIZE: u32 = 1024;
pub const MAX_SLOT_HEIGHT: u32 = 48;
pub const SLOT_PADDING: u32 = 2;
pub const MAX_RASTER_SCALE: f32 = 1.0;
pub const SIZE_CLASSES: [u32; 27] =
    [0, 2, 4, 6, 8, 12, 16, 18, 22, 26, 32, 36, 40, 45, 50, 56, 64, 72, 80, 90, 108, 128, 147, 168, 194, 222, 256];
pub const CURVE_TOLERANCE: f32 = 0.25;

pub fn height_ratio(m: &Mat2d) -> f32 {
    let len = (m.a * m.a + m.b * m.b).sqrt();
    let r = if len == 0.0 { (m.c * m.c + m.d * m.d).sqrt() } else { ((m.c - m.a) * m.b - (m.d - m.b) * m.a) / len };
    r.abs().max(1e-8)
}

pub fn snapped_size(screen: f32) -> u16 {
    let n = screen.max(0.0).floor() as u32;
    let m = n + n.div_ceil(4);
    let mut s =
        if m < 256 { SIZE_CLASSES[SIZE_CLASSES.iter().position(|&c| c >= m).unwrap_or(26)] } else { 255 } as f32;
    let max_h = (MAX_SLOT_HEIGHT - 2 * SLOT_PADDING) as f32;
    if s <= MAX_RASTER_SCALE * max_h && max_h < s {
        s = max_h;
    }
    (s * 16.0).floor() as u16
}

type Point = (f32, f32);

#[derive(Clone, Debug, Default)]
struct Path {
    fill0: u32,
    fill1: u32,
    start: Point,
    edges: Vec<(Option<Point>, Point)>,
}

fn paths(records: &[ShapeRecord]) -> Vec<Path> {
    let (mut out, mut cur, mut f0, mut f1) = (Vec::<Path>::new(), (0.0f32, 0.0f32), 0u32, 0u32);
    for r in records {
        match *r {
            ShapeRecord::StyleChange { move_to, fill0, fill1, .. } => {
                if let Some(f) = fill0 {
                    f0 = f;
                }
                if let Some(f) = fill1 {
                    f1 = f;
                }
                if let Some((x, y)) = move_to {
                    cur = (x as f32, y as f32);
                }
                out.push(Path { fill0: f0, fill1: f1, start: cur, edges: Vec::new() });
            }
            ShapeRecord::Straight { dx, dy } => {
                if out.is_empty() {
                    out.push(Path { fill0: f0, fill1: f1, start: cur, ..Default::default() });
                }
                cur = (cur.0 + dx as f32, cur.1 + dy as f32);
                out.last_mut().expect("a path").edges.push((None, cur));
            }
            ShapeRecord::Curved { control_dx, control_dy, anchor_dx, anchor_dy } => {
                if out.is_empty() {
                    out.push(Path { fill0: f0, fill1: f1, start: cur, ..Default::default() });
                }
                let c = (cur.0 + control_dx as f32, cur.1 + control_dy as f32);
                cur = (c.0 + anchor_dx as f32, c.1 + anchor_dy as f32);
                out.last_mut().expect("a path").edges.push((Some(c), cur));
            }
        }
    }
    out
}

fn bounds(paths: &[Path]) -> Option<[f32; 4]> {
    let mut b = [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];
    let mut add = |(x, y): (f32, f32)| {
        b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
    };
    for p in paths.iter().filter(|p| (p.fill0 == 0) != (p.fill1 == 0)) {
        let mut cur = p.start;
        add(cur);
        for &(c, a) in &p.edges {
            if let Some(c) = c {
                let at = |t: f32| {
                    let (l0, l1) = (
                        (cur.0 + (c.0 - cur.0) * t, cur.1 + (c.1 - cur.1) * t),
                        (c.0 + (a.0 - c.0) * t, c.1 + (a.1 - c.1) * t),
                    );
                    (l0.0 + (l1.0 - l0.0) * t, l0.1 + (l1.1 - l0.1) * t)
                };
                for (p0, p1, p2) in [(cur.0, c.0, a.0), (cur.1, c.1, a.1)] {
                    let d = p1 * 2.0 - p0 - p2;
                    if d != 0.0 {
                        let t = (p1 - p0) / d;
                        if 0.0 < t && t < 1.0 {
                            add(at(t));
                        }
                    }
                }
            }
            add(a);
            cur = a;
        }
    }
    (b[0] <= b[2] && b[1] <= b[3]).then_some(b)
}

fn table_bounds(paths: &[Path], units: f32) -> [f32; 4] {
    let Some(b) = bounds(paths) else { return [0.0; 4] };
    let k = 1024.0 / units;
    let [x0, y0, x1, y1] = b.map(|v| v * k);
    let q = |v: f32| (v * 20.0) as i32 as i16;
    let qu = |v: f32| (v * 20.0) as i64 as u16;
    let (x, y) = (f32::from(q(x0)) * 0.05, f32::from(q(y0)) * 0.05);
    [x, y, x + f32::from(qu(x1 - x0)) * 0.05, y + f32::from(qu(y1 - y0)) * 0.05].map(|v| v / k)
}

#[derive(Clone, Copy, Debug, Default)]
struct Cell {
    x: i32,
    y: i32,
    cover: i32,
    area: i32,
}

#[derive(Default)]
struct Rasterizer {
    cells: Vec<Cell>,
    cur: Cell,
    min: (i32, i32),
    max: (i32, i32),
    start: (i32, i32),
    pos: (i32, i32),
    last: (f32, f32),
}

const SHIFT: i32 = 8;
const SCALE: i32 = 1 << SHIFT;
const MASK: i32 = SCALE - 1;

impl Rasterizer {
    fn new() -> Self {
        Rasterizer { min: (i32::MAX, i32::MAX), max: (-i32::MAX, -i32::MAX), ..Default::default() }
    }

    fn fixed(v: f32) -> i32 {
        (f64::from(v) * 256.0) as i32
    }

    fn move_to(&mut self, x: f32, y: f32) {
        self.last = (x, y);
        self.pos = (Self::fixed(x), Self::fixed(y));
        self.start = self.pos;
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.last = (x, y);
        let to = (Self::fixed(x), Self::fixed(y));
        self.line(self.pos.0, self.pos.1, to.0, to.1);
        self.pos = to;
    }

    fn close(&mut self) {
        if self.pos != self.start {
            self.line(self.pos.0, self.pos.1, self.start.0, self.start.1);
            self.pos = self.start;
        }
    }

    fn set_cell(&mut self, x: i32, y: i32) {
        if self.cur.x != x || self.cur.y != y {
            if self.cur.cover != 0 || self.cur.area != 0 {
                self.cells.push(self.cur);
            }
            self.cur = Cell { x, y, cover: 0, area: 0 };
        }
    }

    fn bound(&mut self, ex: i32, ey: i32) {
        self.min = (self.min.0.min(ex), self.min.1.min(ey));
        self.max = (self.max.0.max(ex), self.max.1.max(ey));
    }

    fn line(&mut self, x1: i32, y1: i32, x2: i32, y2: i32) {
        const DX_LIMIT: i64 = 16384 << SHIFT;
        let dx = x2 - x1;
        if i64::from(dx) >= DX_LIMIT || i64::from(dx) <= -DX_LIMIT {
            let (cx, cy) = ((x1 + x2) >> 1, (y1 + y2) >> 1);
            self.line(x1, y1, cx, cy);
            self.line(cx, cy, x2, y2);
            return;
        }
        let mut dy = y2 - y1;
        let (ex1, ex2, mut ey1, ey2) = (x1 >> SHIFT, x2 >> SHIFT, y1 >> SHIFT, y2 >> SHIFT);
        let (fy1, fy2) = (y1 & MASK, y2 & MASK);
        self.bound(ex1, ey1);
        self.bound(ex2, ey2);
        self.set_cell(ex1, ey1);
        if ey1 == ey2 {
            self.hline(ey1, x1, fy1, x2, fy2);
            return;
        }
        let mut incr = 1;
        if dx == 0 {
            let ex = x1 >> SHIFT;
            let two_fx = (x1 - (ex << SHIFT)) << 1;
            let mut first = SCALE;
            if dy < 0 {
                first = 0;
                incr = -1;
            }
            let delta = first - fy1;
            self.cur.cover += delta;
            self.cur.area += two_fx * delta;
            ey1 += incr;
            self.set_cell(ex, ey1);
            let delta = first + first - SCALE;
            let area = two_fx * delta;
            while ey1 != ey2 {
                self.cur.cover = delta;
                self.cur.area = area;
                ey1 += incr;
                self.set_cell(ex, ey1);
            }
            let delta = fy2 - SCALE + first;
            self.cur.cover += delta;
            self.cur.area += two_fx * delta;
            return;
        }
        let mut p = i64::from(SCALE - fy1) * i64::from(dx);
        let mut first = SCALE;
        if dy < 0 {
            p = i64::from(fy1) * i64::from(dx);
            first = 0;
            incr = -1;
            dy = -dy;
        }
        let mut delta = (p / i64::from(dy)) as i32;
        let mut m = (p % i64::from(dy)) as i32;
        if m < 0 {
            delta -= 1;
            m += dy;
        }
        let mut x_from = x1 + delta;
        self.hline(ey1, x1, fy1, x_from, first);
        ey1 += incr;
        self.set_cell(x_from >> SHIFT, ey1);
        if ey1 != ey2 {
            let p = i64::from(SCALE) * i64::from(dx);
            let mut lift = (p / i64::from(dy)) as i32;
            let mut rem = (p % i64::from(dy)) as i32;
            if rem < 0 {
                lift -= 1;
                rem += dy;
            }
            m -= dy;
            while ey1 != ey2 {
                let mut delta = lift;
                m += rem;
                if m >= 0 {
                    m -= dy;
                    delta += 1;
                }
                let x_to = x_from + delta;
                self.hline(ey1, x_from, SCALE - first, x_to, first);
                x_from = x_to;
                ey1 += incr;
                self.set_cell(x_from >> SHIFT, ey1);
            }
        }
        self.hline(ey1, x_from, SCALE - first, x2, fy2);
    }

    fn hline(&mut self, ey: i32, x1: i32, mut y1: i32, x2: i32, y2: i32) {
        let (mut ex1, ex2) = (x1 >> SHIFT, x2 >> SHIFT);
        let (fx1, fx2) = (x1 & MASK, x2 & MASK);
        if y1 == y2 {
            self.set_cell(ex2, ey);
            return;
        }
        if ex1 == ex2 {
            let delta = y2 - y1;
            self.cur.cover += delta;
            self.cur.area += (fx1 + fx2) * delta;
            return;
        }
        let mut p = i64::from(SCALE - fx1) * i64::from(y2 - y1);
        let mut first = SCALE;
        let mut incr = 1;
        let mut dx = x2 - x1;
        if dx < 0 {
            p = i64::from(fx1) * i64::from(y2 - y1);
            first = 0;
            incr = -1;
            dx = -dx;
        }
        let mut delta = (p / i64::from(dx)) as i32;
        let mut m = (p % i64::from(dx)) as i32;
        if m < 0 {
            delta -= 1;
            m += dx;
        }
        self.cur.cover += delta;
        self.cur.area += (fx1 + first) * delta;
        ex1 += incr;
        self.set_cell(ex1, ey);
        y1 += delta;
        if ex1 != ex2 {
            let p = i64::from(SCALE) * i64::from(y2 - y1 + delta);
            let mut lift = (p / i64::from(dx)) as i32;
            let mut rem = (p % i64::from(dx)) as i32;
            if rem < 0 {
                lift -= 1;
                rem += dx;
            }
            m -= dx;
            while ex1 != ex2 {
                let mut delta = lift;
                m += rem;
                if m >= 0 {
                    m -= dx;
                    delta += 1;
                }
                self.cur.cover += delta;
                self.cur.area += SCALE * delta;
                y1 += delta;
                ex1 += incr;
                self.set_cell(ex1, ey);
            }
        }
        let delta = y2 - y1;
        self.cur.cover += delta;
        self.cur.area += (fx2 + SCALE - first) * delta;
    }

    fn quad(&mut self, c: (f32, f32), p2: (f32, f32)) {
        let p1 = self.last;
        let tol = CURVE_TOLERANCE;
        let (dx, dy) = (p2.0 - p1.0, p2.1 - p1.1);
        let len2 = dy * dy + dx * dx;
        let d = ((c.0 - p2.0) * dy - (c.1 - p2.1) * dx).abs();
        if tol * tol * len2 < d * d {
            self.subdivide(tol * tol, p1, c, p2, 0);
            return;
        }
        let t = if len2 == 0.0 { -1.0 } else { ((c.1 - p1.1) * dy + (c.0 - p1.0) * dx) / len2 };
        if !(0.0..=1.0).contains(&t) {
            let d1 = ((c.1 - p1.1).powi(2) + (c.0 - p1.0).powi(2)).sqrt();
            let d2 = ((p2.1 - c.1).powi(2) + (p2.0 - c.0).powi(2)).sqrt() + d1;
            if d2 != 0.0 {
                let t = d1 / d2;
                let a = ((c.0 - p1.0) * t + p1.0, (c.1 - p1.1) * t + p1.1);
                let b = ((p2.0 - c.0) * t + c.0, (p2.1 - c.1) * t + c.1);
                self.line_to((b.0 - a.0) * t + a.0, (b.1 - a.1) * t + a.1);
            }
        }
        self.line_to(p2.0, p2.1);
    }

    fn subdivide(&mut self, tol2: f32, mut p1: (f32, f32), mut c: (f32, f32), p2: (f32, f32), mut level: u32) {
        let dist = |p1: (f32, f32), c: (f32, f32)| ((p2.1 - p1.1) * (c.0 - p2.0) - (p2.0 - p1.0) * (c.1 - p2.1)).abs();
        let mut d = dist(p1, c);
        while d != 0.0 {
            if d * d <= ((p2.1 - p1.1).powi(2) + (p2.0 - p1.0).powi(2)) * tol2 || level > 11 {
                break;
            }
            level += 1;
            let a = ((p1.0 + c.0) * 0.5, (p1.1 + c.1) * 0.5);
            let c2 = ((c.0 + p2.0) * 0.5, (c.1 + p2.1) * 0.5);
            let m = ((a.0 + c2.0) * 0.5, (a.1 + c2.1) * 0.5);
            self.subdivide(tol2, p1, a, m, level);
            (p1, c) = (m, c2);
            d = dist(p1, c);
        }
        self.line_to(p2.0, p2.1);
    }

    fn finish(&mut self) {
        if self.cur.cover != 0 || self.cur.area != 0 {
            self.cells.push(self.cur);
            self.cur = Cell::default();
        }
        self.cells.sort_by_key(|c| (c.y, c.x));
    }

    fn sweep(&self, y: i32, out: &mut [u8]) {
        let start = self.cells.partition_point(|c| c.y < y);
        let end = self.cells.partition_point(|c| c.y <= y);
        let row = &self.cells[start..end];
        let alpha = |v: i32| (v >> 9).unsigned_abs().min(255) as u8;
        let (mut cover, mut i) = (0i32, 0);
        while i < row.len() {
            let x = row[i].x;
            let mut area = 0i32;
            while i < row.len() && row[i].x == x {
                cover = cover.wrapping_add(row[i].cover);
                area = area.wrapping_add(row[i].area);
                i += 1;
            }
            let mut x = x;
            if area != 0 {
                if let Some(px) = out.get_mut((x - self.min.0) as usize) {
                    *px = alpha(cover.wrapping_mul(512).wrapping_sub(area));
                }
                x += 1;
            }
            if let Some(next) = row.get(i)
                && x < next.x
            {
                let a = alpha(cover.wrapping_mul(512));
                if a != 0 {
                    for px in x..next.x {
                        if let Some(p) = out.get_mut((px - self.min.0) as usize) {
                            *p = a;
                        }
                    }
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GlyphRaster {
    pub x0: i32,
    pub y0: i32,
    pub width: u32,
    pub height: u32,
    pub alpha: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Rasterized {
    Raster(Arc<GlyphRaster>),
    Empty,
    TooTall,
}

pub fn rasterize_glyph(font: &Font, index: usize, size: u16) -> Rasterized {
    let Some(shape) = font.glyphs.get(index) else { return Rasterized::Empty };
    let paths = paths(&shape.records);
    let units = crate::text::em_units(font);
    let s = f64::from(f32::from(size) / units) * 0.0625;
    let b = table_bounds(&paths, units);
    let (lo, hi) = (((f64::from(b[1]) * s) as f32).floor(), ((f64::from(b[3]) * s) as f32).ceil());
    let h = if hi <= lo { 0.0 } else { hi - lo };
    if h as u32 + 2 * SLOT_PADDING >= MAX_SLOT_HEIGHT {
        return Rasterized::TooTall;
    }
    let mut ras = Rasterizer::new();
    let sc = |p: (f32, f32)| ((f64::from(p.0) * s) as f32, (f64::from(p.1) * s) as f32);
    for p in paths.iter().filter(|p| p.fill0 != p.fill1 && !p.edges.is_empty()) {
        let (x, y) = sc(p.start);
        ras.move_to(x, y);
        for &(c, a) in &p.edges {
            let (ax, ay) = sc(a);
            match c {
                None => ras.line_to(ax, ay),
                Some(c) => ras.quad(sc(c), (ax, ay)),
            }
        }
        ras.close();
    }
    ras.finish();
    if ras.cells.is_empty() {
        return Rasterized::Empty;
    }
    let pad = SLOT_PADDING as i32;
    let w = (ras.max.0 - ras.min.0 + 1 + 2 * pad) as u32;
    let h = ((ras.max.1 - ras.min.1 + 1 + 2 * pad) as u32).min(MAX_SLOT_HEIGHT);
    let mut slot = vec![0u8; (w * h) as usize];
    for row in 0..=(ras.max.1 - ras.min.1) {
        if h as i32 <= row + pad {
            break;
        }
        let at = ((row + pad) as u32 * w + pad as u32) as usize;
        ras.sweep(ras.min.1 + row, &mut slot[at..at + (w as usize - 2 * pad as usize)]);
    }
    let (rw, rh) = (w - 2, h - 2);
    let mut alpha = Vec::with_capacity((rw * rh) as usize);
    for y in 1..h - 1 {
        alpha.extend_from_slice(&slot[(y * w + 1) as usize..(y * w + w - 1) as usize]);
    }
    Rasterized::Raster(Arc::new(GlyphRaster { x0: ras.min.0 - 1, y0: ras.min.1 - 1, width: rw, height: rh, alpha }))
}

pub type AtlasRect = [u32; 4];

// TODO(guess): one growing texture, never evicted (the game's is one 1024² texture 0x82726538; a full cache ends the glyph run, 0x82725070 returns 0); settle: the packer (0x826EEA88) and the eviction (0x826EF1D0).
#[derive(Clone, Debug)]
pub struct GlyphAtlas {
    pub width: u32,
    pub height: u32,
    pub alpha: Vec<u8>,
    pub generation: u64,
    shelves: Vec<(u32, u32, u32)>,
    used: u32,
}

impl Default for GlyphAtlas {
    fn default() -> Self {
        GlyphAtlas {
            width: TEXTURE_SIZE,
            height: TEXTURE_SIZE,
            alpha: vec![0; (TEXTURE_SIZE * TEXTURE_SIZE) as usize],
            generation: 0,
            shelves: Vec::new(),
            used: 0,
        }
    }
}

impl GlyphAtlas {
    pub fn add(&mut self, r: &GlyphRaster) -> AtlasRect {
        let (w, h) = (r.width + 1, r.height + 1);
        let shelf = self.shelves.iter_mut().find(|s| s.1 >= h && s.1 <= h + h / 4 + 2 && s.2 + w <= self.width);
        let (x, y) = match shelf {
            Some(s) => {
                s.2 += w;
                (s.2 - w, s.0)
            }
            None => {
                let top = self.used;
                self.used += h;
                while self.used > self.height {
                    self.alpha.resize((self.width * self.height * 2) as usize, 0);
                    self.height *= 2;
                }
                self.shelves.push((top, h, w));
                (0, top)
            }
        };
        for row in 0..r.height {
            let at = ((y + row) * self.width + x) as usize;
            let src = (row * r.width) as usize;
            self.alpha[at..at + r.width as usize].copy_from_slice(&r.alpha[src..src + r.width as usize]);
        }
        self.generation += 1;
        [x, y, x + r.width, y + r.height]
    }
}

#[derive(Clone, Debug)]
pub struct RasterGlyph {
    pub raster: Arc<GlyphRaster>,
    pub rect: AtlasRect,
    pub quad: Arc<Mesh>,
}

#[derive(Clone, Debug, Default)]
pub struct GlyphCache {
    glyphs: HashMap<(usize, usize, u16), Option<RasterGlyph>>,
    pub atlas: GlyphAtlas,
    pub rasterized: usize,
    pub too_tall: usize,
}

pub enum CachedGlyph {
    Raster(RasterGlyph),
    Empty,
    Vector,
}

impl GlyphCache {
    pub fn glyph(&mut self, font: &Arc<Font>, index: usize, size: u16) -> CachedGlyph {
        let key = (Arc::as_ptr(font) as usize, index, size);
        if let Some(g) = self.glyphs.get(&key) {
            return match g {
                Some(r) if r.raster.width > 0 => CachedGlyph::Raster(r.clone()),
                Some(_) => CachedGlyph::Empty,
                None => CachedGlyph::Vector,
            };
        }
        let entry = match rasterize_glyph(font, index, size) {
            Rasterized::Raster(r) => {
                self.rasterized += 1;
                let rect = self.atlas.add(&r);
                let (x0, y0, x1, y1) =
                    (r.x0 as f32, r.y0 as f32, (r.x0 + r.width as i32) as f32, (r.y0 + r.height as i32) as f32);
                let quad = Mesh {
                    vertices: vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]],
                    indices: vec![0, 1, 2, 0, 2, 3],
                    paint: Vec::new(),
                };
                Some(RasterGlyph { raster: r, rect, quad: Arc::new(quad) })
            }
            Rasterized::Empty => {
                let raster = Arc::new(GlyphRaster { x0: 0, y0: 0, width: 0, height: 0, alpha: Vec::new() });
                Some(RasterGlyph { raster, rect: [0; 4], quad: Arc::new(Mesh::default()) })
            }
            Rasterized::TooTall => {
                self.too_tall += 1;
                None
            }
        };
        self.glyphs.insert(key, entry);
        self.glyph(font, index, size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn the_size_class_is_a_quarter_above_the_screen_height() {
        let px = |h: f32| f32::from(snapped_size(h)) / 16.0;
        assert_eq!(px(0.5), 0.0);
        assert_eq!(px(1.9), 2.0);
        assert_eq!(px(20.0), 26.0);
        assert_eq!(px(20.99), 26.0);
        assert_eq!(px(36.0), 45.0);
        assert_eq!(px(44.0), 56.0);
        assert_eq!(px(200.0), 256.0);
        assert_eq!(px(205.0), 255.0);
    }

    #[test]
    fn the_height_ratio_is_the_vertical_scale_across_the_x_axis() {
        assert!((height_ratio(&Mat2d::scale(2.0, 3.0)) - 3.0).abs() < 1e-6);
        let rot = Mat2d { a: 0.0, b: 2.0, c: -2.0, d: 0.0, tx: 5.0, ty: 7.0 };
        assert!((height_ratio(&rot) - 2.0).abs() < 1e-6);
        let skew = Mat2d { a: 1.0, b: 0.0, c: 0.5, d: 1.0, tx: 0.0, ty: 0.0 };
        assert!((height_ratio(&skew) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_box_rasterises_to_its_exact_coverage() {
        let f = box_font(512, 700);
        let Rasterized::Raster(r) = rasterize_glyph(&f, 0, 32 * 16) else { panic!("a raster") };
        assert_eq!((r.x0, r.y0, r.width, r.height), (-1, -1, 19, 24));
        let at = |x: u32, y: u32| r.alpha[(y * r.width + x) as usize];
        assert_eq!(at(0, 0), 0);
        assert_eq!(at(1, 1), 255);
        assert_eq!(at(16, 21), 255);
        assert_eq!(at(17, 5), 0);
        assert_eq!(at(18, 5), 0);
        assert_eq!(at(5, 22), 224);
        assert_eq!(at(5, 23), 0);
    }

    #[test]
    fn a_glyph_too_tall_for_a_slot_is_a_vector_glyph() {
        let f = box_font(512, 1024);
        assert!(matches!(rasterize_glyph(&f, 0, 43 * 16), Rasterized::Raster(_)));
        assert_eq!(rasterize_glyph(&f, 0, 44 * 16), Rasterized::TooTall);
        let empty = Font { glyphs: vec![Shape::default()], ..box_font(1, 1) };
        assert_eq!(rasterize_glyph(&empty, 0, 26 * 16), Rasterized::Empty);
    }
}
