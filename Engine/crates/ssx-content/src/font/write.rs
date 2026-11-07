#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GlyphDef {
    Empty,
    Simple(Vec<Vec<(i16, i16, bool)>>),
    Composite(Vec<(u16, i16, i16)>),
}

#[derive(Clone, Debug)]
pub enum PairSubtable {
    Pairs(Vec<(u16, Vec<(u16, i16)>)>),
    Classes { covered: Vec<u16>, class1: Vec<(u16, u16)>, class2: Vec<(u16, u16)>, values: Vec<Vec<i16>> },
}

#[derive(Clone, Debug)]
pub struct FontDef {
    pub family: String,
    pub units_per_em: u16,
    pub ascender: i16,
    pub descender: i16,
    pub line_gap: i16,
    pub glyphs: Vec<(GlyphDef, u16)>,
    pub cmap: Vec<(u16, u16)>,
    pub cmap12: Vec<(u32, u32, u32)>,
    pub kern: Vec<(u16, u16, i16)>,
    pub gpos_kern: Vec<Vec<PairSubtable>>,
    pub claimed_glyphs: Option<u16>,
}

impl FontDef {
    pub fn new(family: &str, units_per_em: u16) -> Self {
        FontDef {
            family: family.into(),
            units_per_em,
            ascender: (i32::from(units_per_em) * 4 / 5) as i16,
            descender: -((i32::from(units_per_em) / 5) as i16),
            line_gap: 0,
            glyphs: vec![(GlyphDef::Empty, units_per_em / 2)],
            cmap: Vec::new(),
            cmap12: Vec::new(),
            kern: Vec::new(),
            gpos_kern: Vec::new(),
            claimed_glyphs: None,
        }
    }

    pub fn glyph(&mut self, ch: char, glyph: GlyphDef, advance: u16) -> u16 {
        let id = self.glyphs.len() as u16;
        self.glyphs.push((glyph, advance));
        if let Ok(c) = u16::try_from(u32::from(ch)) {
            self.cmap.push((c, id));
        }
        id
    }
}

fn be16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_be_bytes());
}

fn be32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_be_bytes());
}

fn checksum(data: &[u8]) -> u32 {
    data.chunks(4).fold(0u32, |s, c| {
        let mut w = [0u8; 4];
        w[..c.len()].copy_from_slice(c);
        s.wrapping_add(u32::from_be_bytes(w))
    })
}

fn glyph_bytes(g: &GlyphDef) -> Vec<u8> {
    let mut v = Vec::new();
    match g {
        GlyphDef::Empty => {}
        GlyphDef::Simple(contours) => {
            let pts: Vec<&(i16, i16, bool)> = contours.iter().flatten().collect();
            let (xs, ys): (Vec<i16>, Vec<i16>) = pts.iter().map(|p| (p.0, p.1)).unzip();
            be16(&mut v, contours.len() as u16);
            for b in [xs.iter().min(), ys.iter().min(), xs.iter().max(), ys.iter().max()] {
                be16(&mut v, b.copied().unwrap_or(0) as u16);
            }
            let mut end = 0u16;
            for c in contours {
                end += c.len() as u16;
                be16(&mut v, end.wrapping_sub(1));
            }
            be16(&mut v, 0);
            for p in &pts {
                v.push(u8::from(p.2));
            }
            let (mut px, mut py) = (0i16, 0i16);
            for p in &pts {
                be16(&mut v, p.0.wrapping_sub(px) as u16);
                px = p.0;
            }
            for p in &pts {
                be16(&mut v, p.1.wrapping_sub(py) as u16);
                py = p.1;
            }
        }
        GlyphDef::Composite(parts) => {
            be16(&mut v, 0xFFFF);
            for _ in 0..4 {
                be16(&mut v, 0);
            }
            for (i, (glyph, dx, dy)) in parts.iter().enumerate() {
                let more = if i + 1 < parts.len() { 0x0020 } else { 0 };
                be16(&mut v, 0x0001 | 0x0002 | more);
                be16(&mut v, *glyph);
                be16(&mut v, *dx as u16);
                be16(&mut v, *dy as u16);
            }
        }
    }
    while v.len() % 4 != 0 {
        v.push(0);
    }
    v
}

fn cmap(def: &FontDef) -> Vec<u8> {
    let mut map = def.cmap.clone();
    map.sort_unstable();
    map.dedup_by_key(|m| m.0);
    let mut f4 = Vec::new();
    let segs = map.len() + 1;
    be16(&mut f4, 4);
    be16(&mut f4, 0);
    be16(&mut f4, 0);
    be16(&mut f4, (segs * 2) as u16);
    let pow = segs.next_power_of_two().max(1) / if segs.is_power_of_two() { 1 } else { 2 };
    be16(&mut f4, (pow * 2) as u16);
    be16(&mut f4, pow.trailing_zeros() as u16);
    be16(&mut f4, ((segs - pow) * 2) as u16);
    for (c, _) in &map {
        be16(&mut f4, *c);
    }
    be16(&mut f4, 0xFFFF);
    be16(&mut f4, 0);
    for (c, _) in &map {
        be16(&mut f4, *c);
    }
    be16(&mut f4, 0xFFFF);
    for (c, g) in &map {
        be16(&mut f4, g.wrapping_sub(*c));
    }
    be16(&mut f4, 1);
    for _ in 0..segs {
        be16(&mut f4, 0);
    }
    let len = f4.len() as u16;
    f4[2..4].copy_from_slice(&len.to_be_bytes());
    let mut f12 = Vec::new();
    if !def.cmap12.is_empty() {
        be16(&mut f12, 12);
        be16(&mut f12, 0);
        be32(&mut f12, (16 + 12 * def.cmap12.len()) as u32);
        be32(&mut f12, 0);
        be32(&mut f12, def.cmap12.len() as u32);
        for &(a, b, g) in &def.cmap12 {
            be32(&mut f12, a);
            be32(&mut f12, b);
            be32(&mut f12, g);
        }
    }
    let n = if f12.is_empty() { 1 } else { 2 };
    let mut v = Vec::new();
    be16(&mut v, 0);
    be16(&mut v, n);
    let at = 4 + 8 * u32::from(n);
    be16(&mut v, 3);
    be16(&mut v, 1);
    be32(&mut v, at);
    if !f12.is_empty() {
        be16(&mut v, 3);
        be16(&mut v, 10);
        be32(&mut v, at + f4.len() as u32);
    }
    v.extend(f4);
    v.extend(f12);
    v
}

fn name(family: &str) -> Vec<u8> {
    let s: Vec<u8> = family.encode_utf16().flat_map(u16::to_be_bytes).collect();
    let mut v = Vec::new();
    be16(&mut v, 0);
    be16(&mut v, 1);
    be16(&mut v, 6 + 12);
    for x in [3u16, 1, 0x409, 1, s.len() as u16, 0] {
        be16(&mut v, x);
    }
    v.extend(s);
    v
}

pub fn ttf(def: &FontDef) -> Vec<u8> {
    let n = def.glyphs.len();
    let (mut glyf, mut loca) = (Vec::new(), Vec::new());
    for (g, _) in &def.glyphs {
        be32(&mut loca, glyf.len() as u32);
        glyf.extend(glyph_bytes(g));
    }
    be32(&mut loca, glyf.len() as u32);
    let mut head = Vec::new();
    for x in [0x0001_0000u32, 0x0001_0000, 0, 0x5F0F_3CF5] {
        be32(&mut head, x);
    }
    be16(&mut head, 0x000B);
    be16(&mut head, def.units_per_em);
    head.extend_from_slice(&[0; 16]);
    for _ in 0..4 {
        be16(&mut head, 0);
    }
    for x in [0u16, 8, 2, 1, 0] {
        be16(&mut head, x);
    }
    let mut hhea = Vec::new();
    be32(&mut hhea, 0x0001_0000);
    for x in [def.ascender, def.descender, def.line_gap] {
        be16(&mut hhea, x as u16);
    }
    be16(&mut hhea, def.glyphs.iter().map(|g| g.1).max().unwrap_or(0));
    for x in [0u16, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0] {
        be16(&mut hhea, x);
    }
    be16(&mut hhea, n as u16);
    let mut maxp = Vec::new();
    be32(&mut maxp, 0x0001_0000);
    be16(&mut maxp, def.claimed_glyphs.unwrap_or(n as u16));
    for _ in 0..13 {
        be16(&mut maxp, 0);
    }
    let mut hmtx = Vec::new();
    for (_, adv) in &def.glyphs {
        be16(&mut hmtx, *adv);
        be16(&mut hmtx, 0);
    }
    let mut post = Vec::new();
    be32(&mut post, 0x0003_0000);
    post.extend_from_slice(&[0; 28]);
    let mut os2 = Vec::new();
    be16(&mut os2, 4);
    os2.extend_from_slice(&[0; 94]);
    let mut tables: Vec<([u8; 4], Vec<u8>)> = vec![
        (*b"OS/2", os2),
        (*b"cmap", cmap(def)),
        (*b"glyf", glyf),
        (*b"head", head),
        (*b"hhea", hhea),
        (*b"hmtx", hmtx),
        (*b"loca", loca),
        (*b"maxp", maxp),
        (*b"name", name(&def.family)),
        (*b"post", post),
    ];
    if !def.kern.is_empty() {
        let mut k = Vec::new();
        be16(&mut k, 0);
        be16(&mut k, 1);
        be16(&mut k, 0);
        be16(&mut k, (14 + 6 * def.kern.len()) as u16);
        be16(&mut k, 0x0001);
        be16(&mut k, def.kern.len() as u16);
        for _ in 0..3 {
            be16(&mut k, 0);
        }
        let mut pairs = def.kern.clone();
        pairs.sort_unstable();
        for (l, r, v) in pairs {
            be16(&mut k, l);
            be16(&mut k, r);
            be16(&mut k, v as u16);
        }
        tables.push((*b"kern", k));
        tables.sort_by_key(|t| t.0);
    }
    if !def.gpos_kern.is_empty() {
        tables.push((*b"GPOS", gpos(&def.gpos_kern)));
        tables.sort_by_key(|t| t.0);
    }
    let count = tables.len() as u16;
    let mut out = Vec::new();
    be32(&mut out, 0x0001_0000);
    be16(&mut out, count);
    let pow = 1u16 << (15 - count.leading_zeros());
    be16(&mut out, pow * 16);
    be16(&mut out, pow.trailing_zeros() as u16);
    be16(&mut out, count * 16 - pow * 16);
    let mut offset = 12 + 16 * u32::from(count);
    let mut body = Vec::new();
    for (tag, data) in &tables {
        out.extend_from_slice(tag);
        be32(&mut out, checksum(data));
        be32(&mut out, offset);
        be32(&mut out, data.len() as u32);
        let mut d = data.clone();
        while d.len() % 4 != 0 {
            d.push(0);
        }
        offset += d.len() as u32;
        body.extend(d);
    }
    out.extend(body);
    let adjust = 0xB1B0_AFBAu32.wrapping_sub(checksum(&out));
    if let Some(i) = tables.iter().position(|t| &t.0 == b"head") {
        let at = 12 + 16 * i;
        let head_at = u32::from_be_bytes([out[at + 8], out[at + 9], out[at + 10], out[at + 11]]) as usize;
        out[head_at + 8..head_at + 12].copy_from_slice(&adjust.to_be_bytes());
    }
    out
}

pub fn rect(x: i16, y: i16, w: i16, h: i16) -> Vec<(i16, i16, bool)> {
    vec![(x, y, true), (x, y + h, true), (x + w, y + h, true), (x + w, y, true)]
}

fn coverage(glyphs: &[u16]) -> Vec<u8> {
    let mut c = Vec::new();
    be16(&mut c, 1);
    be16(&mut c, glyphs.len() as u16);
    for &g in glyphs {
        be16(&mut c, g);
    }
    c
}

fn class_def(classes: &[(u16, u16)]) -> Vec<u8> {
    let mut c = Vec::new();
    be16(&mut c, 2);
    be16(&mut c, classes.len() as u16);
    for &(g, k) in classes {
        be16(&mut c, g);
        be16(&mut c, g);
        be16(&mut c, k);
    }
    c
}

fn pair_subtable(sub: &PairSubtable) -> Vec<u8> {
    let mut t = Vec::new();
    match sub {
        PairSubtable::Pairs(sets) => {
            let firsts: Vec<u16> = sets.iter().map(|s| s.0).collect();
            let head = 10 + 2 * sets.len();
            let cov = coverage(&firsts);
            let mut tail = cov.clone();
            let mut offsets = Vec::new();
            for (_, pairs) in sets {
                offsets.push((head + tail.len()) as u16);
                be16(&mut tail, pairs.len() as u16);
                for &(g, v) in pairs {
                    be16(&mut tail, g);
                    be16(&mut tail, v as u16);
                }
            }
            be16(&mut t, 1);
            be16(&mut t, head as u16);
            be16(&mut t, 0x0004);
            be16(&mut t, 0);
            be16(&mut t, sets.len() as u16);
            for o in offsets {
                be16(&mut t, o);
            }
            t.extend(tail);
        }
        PairSubtable::Classes { covered, class1, class2, values } => {
            let n2 = values.first().map_or(0, Vec::len);
            let head = 16 + 2 * values.len() * n2;
            let (cov, c1, c2) = (coverage(covered), class_def(class1), class_def(class2));
            be16(&mut t, 2);
            be16(&mut t, head as u16);
            be16(&mut t, 0x0004);
            be16(&mut t, 0);
            be16(&mut t, (head + cov.len()) as u16);
            be16(&mut t, (head + cov.len() + c1.len()) as u16);
            be16(&mut t, values.len() as u16);
            be16(&mut t, n2 as u16);
            for row in values {
                for &v in row {
                    be16(&mut t, v as u16);
                }
            }
            t.extend(cov);
            t.extend(c1);
            t.extend(c2);
        }
    }
    t
}

fn gpos(lookups: &[Vec<PairSubtable>]) -> Vec<u8> {
    let mut features = Vec::new();
    be16(&mut features, 1);
    features.extend_from_slice(b"kern");
    be16(&mut features, 8);
    be16(&mut features, 0);
    be16(&mut features, lookups.len() as u16);
    for i in 0..lookups.len() {
        be16(&mut features, i as u16);
    }
    let mut list = Vec::new();
    be16(&mut list, lookups.len() as u16);
    let mut bodies: Vec<u8> = Vec::new();
    let head = 2 + 2 * lookups.len();
    for subs in lookups {
        be16(&mut list, (head + bodies.len()) as u16);
        let tables: Vec<Vec<u8>> = subs.iter().map(pair_subtable).collect();
        let mut l = Vec::new();
        be16(&mut l, 2);
        be16(&mut l, 0);
        be16(&mut l, tables.len() as u16);
        let mut at = 6 + 2 * tables.len();
        for t in &tables {
            be16(&mut l, at as u16);
            at += t.len();
        }
        for t in tables {
            l.extend(t);
        }
        bodies.extend(l);
    }
    list.extend(bodies);
    let scripts = vec![0u8, 0];
    let mut g = Vec::new();
    be32(&mut g, 0x0001_0000);
    be16(&mut g, 10);
    be16(&mut g, (10 + scripts.len()) as u16);
    be16(&mut g, (10 + scripts.len() + features.len()) as u16);
    g.extend(scripts);
    g.extend(features);
    g.extend(list);
    g
}
