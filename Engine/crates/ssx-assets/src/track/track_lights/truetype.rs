use std::collections::HashMap;

use anyhow::{Context, Result, bail, ensure};
use ssx_formats::reader::Reader;

pub struct Font {
    data: Vec<u8>,
    pub units_per_em: u16,
    pub ascender: i16,
    pub descender: i16,
    pub line_gap: i16,
    advances: Vec<u16>,
    offsets: Vec<u32>,
    glyf: usize,
    glyf_len: usize,
    cmap: HashMap<u32, u16>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
    pub on: bool,
}

fn table(data: &[u8], tag: &[u8; 4]) -> Result<(usize, usize)> {
    let mut r = Reader::be(data);
    r.seek(4)?;
    let count = r.u16()? as usize;
    for i in 0..count {
        r.seek(12 + 16 * i)?;
        let t = r.array::<4>()?;
        r.skip(4)?;
        let (offset, len) = (r.u32()? as usize, r.u32()? as usize);
        if &t == tag {
            ensure!(offset + len <= data.len(), "table {} past the end", String::from_utf8_lossy(tag));
            return Ok((offset, len));
        }
    }
    bail!("no `{}` table", String::from_utf8_lossy(tag))
}

fn parse_cmap(data: &[u8], (at, _): (usize, usize)) -> Result<HashMap<u32, u16>> {
    let mut r = Reader::be(data);
    r.seek(at + 2)?;
    let count = r.u16()? as usize;
    let mut sub = None;
    for i in 0..count {
        r.seek(at + 4 + 8 * i)?;
        let (platform, encoding, offset) = (r.u16()?, r.u16()?, r.u32()? as usize);
        if matches!((platform, encoding), (3, 1) | (0, 3)) {
            r.seek(at + offset)?;
            if r.u16()? == 4 {
                sub = Some(at + offset);
                break;
            }
        }
    }
    let s = sub.context("no format-4 Unicode cmap")?;
    r.seek(s + 6)?;
    let segs = r.u16()? as usize / 2;
    let (ends, starts, deltas, ranges) = (s + 14, s + 16 + 2 * segs, s + 16 + 4 * segs, s + 16 + 6 * segs);
    let mut map = HashMap::new();
    let at16 = |r: &mut Reader, pos: usize| -> Result<u16> {
        r.seek(pos)?;
        Ok(r.u16()?)
    };
    for k in 0..segs {
        let (end, start) = (at16(&mut r, ends + 2 * k)?, at16(&mut r, starts + 2 * k)?);
        let delta = at16(&mut r, deltas + 2 * k)?;
        let range = at16(&mut r, ranges + 2 * k)? as usize;
        if start == 0xFFFF {
            continue;
        }
        for c in start..=end {
            let g = if range == 0 {
                c.wrapping_add(delta)
            } else {
                let g = at16(&mut r, ranges + 2 * k + range + 2 * (c - start) as usize)?;
                if g == 0 { 0 } else { g.wrapping_add(delta) }
            };
            if g != 0 {
                map.insert(c as u32, g);
            }
        }
    }
    Ok(map)
}

impl Font {
    pub fn parse(data: Vec<u8>) -> Result<Font> {
        let head = table(&data, b"head")?;
        let hhea = table(&data, b"hhea")?;
        let maxp = table(&data, b"maxp")?;
        let (hmtx, _) = table(&data, b"hmtx")?;
        let (loca, _) = table(&data, b"loca")?;
        let (glyf, glyf_len) = table(&data, b"glyf")?;
        let cmap = parse_cmap(&data, table(&data, b"cmap")?)?;
        let mut r = Reader::be(&data);
        r.seek(head.0 + 18)?;
        let units_per_em = r.u16()?;
        r.seek(head.0 + 50)?;
        let long_loca = r.i16()? == 1;
        r.seek(hhea.0 + 4)?;
        let (ascender, descender, line_gap) = (r.i16()?, r.i16()?, r.i16()?);
        r.seek(hhea.0 + 34)?;
        let metrics = r.u16()? as usize;
        r.seek(maxp.0 + 4)?;
        let glyphs = r.u16()? as usize;
        ensure!(units_per_em > 0 && metrics > 0 && metrics <= glyphs, "bad head / hhea / maxp");
        r.seek(hmtx)?;
        let mut advances = Vec::with_capacity(metrics);
        for _ in 0..metrics {
            advances.push(r.u16()?);
            r.skip(2)?;
        }
        r.seek(loca)?;
        let mut offsets = Vec::with_capacity(glyphs + 1);
        for _ in 0..=glyphs {
            offsets.push(if long_loca { r.u32()? } else { u32::from(r.u16()?) * 2 });
        }
        ensure!(
            offsets.windows(2).all(|w| w[0] <= w[1]) && *offsets.last().unwrap_or(&0) as usize <= glyf_len,
            "loca out of order or past glyf"
        );
        Ok(Font { data, units_per_em, ascender, descender, line_gap, advances, offsets, glyf, glyf_len, cmap })
    }

    pub fn num_glyphs(&self) -> usize {
        self.offsets.len() - 1
    }

    pub fn glyph_index(&self, c: u32) -> u16 {
        self.cmap.get(&c).copied().unwrap_or(0)
    }

    pub fn advance(&self, g: u16) -> u16 {
        self.advances[(g as usize).min(self.advances.len() - 1)]
    }

    pub fn contours(&self, g: u16) -> Result<Vec<Vec<Point>>> {
        let mut out = Vec::new();
        self.append(g, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], 0, &mut out)?;
        Ok(out)
    }

    fn append(&self, g: u16, m: [f32; 6], depth: u32, out: &mut Vec<Vec<Point>>) -> Result<()> {
        ensure!(depth < 8, "composite glyph nesting too deep");
        let (a, b) = (
            *self.offsets.get(g as usize).context("glyph id out of range")? as usize,
            self.offsets[g as usize + 1] as usize,
        );
        if a == b {
            return Ok(());
        }
        ensure!(b <= self.glyf_len, "glyph {g} past glyf");
        let mut r = Reader::be(&self.data[self.glyf + a..self.glyf + b]);
        let contours = r.i16()?;
        r.skip(8)?;
        let tf = |x: f32, y: f32| (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]);
        if contours >= 0 {
            let mut ends = Vec::with_capacity(contours as usize);
            for _ in 0..contours {
                ends.push(r.u16()? as usize);
            }
            let n = ends.last().map_or(0, |e| e + 1);
            ensure!(ends.windows(2).all(|w| w[0] < w[1]), "glyph {g}: contour ends out of order");
            let instructions = r.u16()? as usize;
            r.skip(instructions)?;
            let mut flags = Vec::with_capacity(n);
            while flags.len() < n {
                let f = r.u8()?;
                flags.push(f);
                if f & 8 != 0 {
                    for _ in 0..r.u8()? {
                        flags.push(f);
                    }
                }
            }
            ensure!(flags.len() == n, "glyph {g}: flag repeat overruns the points");
            let mut coords = |short: u8, same: u8| -> Result<Vec<i32>> {
                let mut v = 0i32;
                let mut out = Vec::with_capacity(n);
                for &f in &flags {
                    if f & short != 0 {
                        let d = i32::from(r.u8()?);
                        v += if f & same != 0 { d } else { -d };
                    } else if f & same == 0 {
                        v += i32::from(r.i16()?);
                    }
                    out.push(v);
                }
                Ok(out)
            };
            let xs = coords(2, 16)?;
            let ys = coords(4, 32)?;
            let mut start = 0;
            for &end in &ends {
                let contour = (start..=end)
                    .map(|i| {
                        let (x, y) = tf(xs[i] as f32, ys[i] as f32);
                        Point { x, y, on: flags[i] & 1 != 0 }
                    })
                    .collect();
                out.push(contour);
                start = end + 1;
            }
            return Ok(());
        }
        loop {
            let flags = r.u16()?;
            let glyph = r.u16()?;
            ensure!(flags & 2 != 0, "glyph {g}: point-matched components are not supported");
            let (dx, dy) = if flags & 1 != 0 {
                (f32::from(r.i16()?), f32::from(r.i16()?))
            } else {
                (f32::from(r.i8()?), f32::from(r.i8()?))
            };
            let f2dot14 = |r: &mut Reader| -> Result<f32> { Ok(f32::from(r.i16()?) / 16384.0) };
            let c = if flags & 8 != 0 {
                let s = f2dot14(&mut r)?;
                [s, 0.0, 0.0, s]
            } else if flags & 0x40 != 0 {
                [f2dot14(&mut r)?, 0.0, 0.0, f2dot14(&mut r)?]
            } else if flags & 0x80 != 0 {
                [f2dot14(&mut r)?, f2dot14(&mut r)?, f2dot14(&mut r)?, f2dot14(&mut r)?]
            } else {
                [1.0, 0.0, 0.0, 1.0]
            };
            let (ox, oy) = tf(dx, dy);
            let mm = [
                m[0] * c[0] + m[2] * c[1],
                m[1] * c[0] + m[3] * c[1],
                m[0] * c[2] + m[2] * c[3],
                m[1] * c[2] + m[3] * c[3],
                ox,
                oy,
            ];
            self.append(glyph, mm, depth + 1, out)?;
            if flags & 0x20 == 0 {
                return Ok(());
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct GlyphBitmap {
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
    pub coverage: Vec<u8>,
}

fn segments(contours: &[Vec<Point>], scale: f32, left: f32, top: f32) -> Vec<[f32; 4]> {
    let px = |p: Point| (p.x * scale - left, top - p.y * scale);
    let mut lines = Vec::new();
    for c in contours.iter().filter(|c| !c.is_empty()) {
        let first_on = c.iter().position(|p| p.on);
        let (start, rot) = match first_on {
            Some(i) => (px(c[i]), i),
            None => {
                let (a, b) = (px(c[0]), px(c[1 % c.len()]));
                (((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5), 1)
            }
        };
        let mut cur = start;
        let mut ctrl: Option<(f32, f32)> = None;
        let n = c.len();
        let skip = usize::from(first_on.is_some());
        for k in skip..=n {
            let p = if k == n { start } else { px(c[(rot + k) % n]) };
            let on = k == n || c[(rot + k) % n].on;
            match (ctrl, on) {
                (None, true) => {
                    lines.push([cur.0, cur.1, p.0, p.1]);
                    cur = p;
                }
                (None, false) => ctrl = Some(p),
                (Some(q), true) => {
                    quad(&mut lines, cur, q, p);
                    cur = p;
                    ctrl = None;
                }
                (Some(q), false) => {
                    let mid = ((q.0 + p.0) * 0.5, (q.1 + p.1) * 0.5);
                    quad(&mut lines, cur, q, mid);
                    cur = mid;
                    ctrl = Some(p);
                }
            }
        }
    }
    lines
}

fn quad(lines: &mut Vec<[f32; 4]>, a: (f32, f32), b: (f32, f32), c: (f32, f32)) {
    let dev = ((a.0 - 2.0 * b.0 + c.0).powi(2) + (a.1 - 2.0 * b.1 + c.1).powi(2)).sqrt();
    let n = 1 + (5.0 * dev.sqrt()).ceil() as usize;
    let mut prev = a;
    for i in 1..=n {
        let t = i as f32 / n as f32;
        let u = 1.0 - t;
        let p = (u * u * a.0 + 2.0 * u * t * b.0 + t * t * c.0, u * u * a.1 + 2.0 * u * t * b.1 + t * t * c.1);
        lines.push([prev.0, prev.1, p.0, p.1]);
        prev = p;
    }
}

fn cell_integral(x: f32, c: f32) -> f32 {
    let t = x - c;
    if t <= 0.0 {
        t
    } else if t >= 1.0 {
        0.5
    } else {
        t - 0.5 * t * t
    }
}

fn coverage(lines: &[[f32; 4]], w: usize, h: usize) -> Vec<f32> {
    let stride = w + 2;
    let mut acc = vec![0.0f32; stride * h];
    for &[x0, y0, x1, y1] in lines {
        if (y1 - y0).abs() < 1e-9 {
            continue;
        }
        let (dir, (xa, ya), (xb, yb)) = if y0 < y1 { (1.0, (x0, y0), (x1, y1)) } else { (-1.0, (x1, y1), (x0, y0)) };
        let dxdy = (xb - xa) / (yb - ya);
        let row0 = ya.max(0.0).floor() as usize;
        let row1 = (yb.ceil().max(0.0) as usize).min(h);
        for row in row0..row1 {
            let top = (row as f32).max(ya);
            let bottom = ((row + 1) as f32).min(yb);
            if bottom <= top {
                continue;
            }
            let dy = bottom - top;
            let (xt, xbm) = (xa + (top - ya) * dxdy, xa + (bottom - ya) * dxdy);
            let (l, r) = if xt < xbm { (xt, xbm) } else { (xbm, xt) };
            let line = &mut acc[row * stride..(row + 1) * stride];
            let (cl, cr) = ((l.floor().max(0.0) as usize).min(w), (r.floor().max(0.0) as usize).min(w));
            let mut prev = 0.0;
            for (c, cell) in line.iter_mut().enumerate().take(cr + 1).skip(cl) {
                let cf = c as f32;
                let area = if r - l < 1e-6 {
                    dy * (cf + 1.0 - 0.5 * (l + r)).clamp(0.0, 1.0)
                } else {
                    dy * (cell_integral(r, cf) - cell_integral(l, cf)) / (r - l)
                };
                *cell += dir * area - prev;
                prev = dir * area;
            }
            line[cr + 1] += dir * dy - prev;
        }
    }
    let mut out = vec![0.0; w * h];
    for row in 0..h {
        let mut sum = 0.0;
        for x in 0..w {
            sum += acc[row * stride + x];
            out[row * w + x] = sum.abs().min(1.0);
        }
    }
    out
}

pub fn rasterize(contours: &[Vec<Point>], scale: f32) -> GlyphBitmap {
    let pts = contours.iter().flatten();
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in pts {
        x0 = x0.min(p.x * scale);
        x1 = x1.max(p.x * scale);
        y0 = y0.min(p.y * scale);
        y1 = y1.max(p.y * scale);
    }
    if x0 > x1 {
        return GlyphBitmap::default();
    }
    let (left, right, bottom, top) = (x0.floor() as i32, x1.ceil() as i32, y0.floor() as i32, y1.ceil() as i32);
    let (w, h) = ((right - left).max(1) as usize, (top - bottom).max(1) as usize);
    let lines = segments(contours, scale, left as f32, top as f32);
    let cov = coverage(&lines, w, h);
    GlyphBitmap {
        left,
        top,
        width: w as u32,
        height: h as u32,
        coverage: cov.iter().map(|c| (c * 255.0 + 0.5) as u8).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<Point> {
        [(x0, y0), (x0, y1), (x1, y1), (x1, y0)].map(|(x, y)| Point { x, y, on: true }).to_vec()
    }

    #[test]
    fn square_coverage() {
        let b = rasterize(&[square(0.5, 0.5, 2.5, 2.5)], 1.0);
        assert_eq!((b.left, b.top, b.width, b.height), (0, 3, 3, 3));
        assert_eq!(b.coverage, vec![64, 128, 64, 128, 255, 128, 64, 128, 64]);
    }

    #[test]
    fn triangle_coverage() {
        let tri = [(0.0, 0.0), (0.0, 2.0), (2.0, 0.0)].map(|(x, y)| Point { x, y, on: true }).to_vec();
        let b = rasterize(&[tri], 1.0);
        assert_eq!((b.width, b.height), (2, 2));
        assert_eq!(b.coverage, vec![128, 0, 255, 128]);
    }

    #[test]
    fn all_off_curve_contour() {
        let pts = [(0.0, 1.0), (1.0, 2.0), (2.0, 1.0), (1.0, 0.0)].map(|(x, y)| Point { x, y, on: false }).to_vec();
        let lines = segments(&[pts], 1.0, 0.0, 2.0);
        let first = lines.first().copied().unwrap_or_default();
        let last = lines.last().copied().unwrap_or_default();
        assert!((first[0] - last[2]).abs() < 1e-5 && (first[1] - last[3]).abs() < 1e-5);
        let area: f32 = lines.iter().map(|l| (l[2] - l[0]) * (l[3] + l[1]) * 0.5).sum();
        assert!((area.abs() - 5.0 / 3.0).abs() < 0.025, "{area}");
    }
}
