use std::io::Write as _;

#[derive(Default)]
struct Bits {
    out: Vec<u8>,
    acc: u32,
    n: u32,
}

impl Bits {
    fn ub(&mut self, v: u32, bits: u32) {
        for i in (0..bits).rev() {
            self.acc = (self.acc << 1) | ((v >> i) & 1);
            self.n += 1;
            if self.n == 8 {
                self.out.push(self.acc as u8);
                self.acc = 0;
                self.n = 0;
            }
        }
    }

    fn sb(&mut self, v: i32, bits: u32) {
        self.ub(v as u32 & ((1u64 << bits) - 1) as u32, bits);
    }

    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push((self.acc << (8 - self.n)) as u8);
        }
        self.out
    }
}

fn sbits(values: &[i32]) -> u32 {
    let bits = |v: i32| 33 - if v >= 0 { v as u32 } else { !v as u32 }.leading_zeros();
    values.iter().map(|&v| bits(v)).max().unwrap_or(1).max(1)
}

pub fn rect(x0: i32, x1: i32, y0: i32, y1: i32) -> Vec<u8> {
    let n = sbits(&[x0, x1, y0, y1]);
    let mut b = Bits::default();
    b.ub(n, 5);
    for v in [x0, x1, y0, y1] {
        b.sb(v, n);
    }
    b.finish()
}

pub fn matrix(tx: i32, ty: i32, scale: f32) -> Vec<u8> {
    let mut b = Bits::default();
    if scale != 1.0 {
        let s = (scale * 65536.0) as i32;
        let n = sbits(&[s]);
        b.ub(1, 1);
        b.ub(n, 5);
        b.sb(s, n);
        b.sb(s, n);
    } else {
        b.ub(0, 1);
    }
    b.ub(0, 1);
    let n = sbits(&[tx, ty]);
    b.ub(n, 5);
    b.sb(tx, n);
    b.sb(ty, n);
    b.finish()
}

pub struct TextField<'a> {
    pub id: u16,
    pub bounds: (i32, i32, i32, i32),
    pub font_id: u16,
    pub height: u16,
    pub rgba: [u8; 4],
    pub text: &'a str,
    pub html: bool,
    pub align: u8,
}

#[derive(Default)]
pub struct Tags {
    pub bytes: Vec<u8>,
    frames: u16,
}

impl Tags {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn tag(&mut self, code: u16, body: &[u8]) -> &mut Self {
        if body.len() < 63 {
            self.bytes.extend_from_slice(&((code << 6) | body.len() as u16).to_le_bytes());
        } else {
            self.bytes.extend_from_slice(&((code << 6) | 0x3F).to_le_bytes());
            self.bytes.extend_from_slice(&(body.len() as u32).to_le_bytes());
        }
        self.bytes.extend_from_slice(body);
        if code == 1 {
            self.frames += 1;
        }
        self
    }

    pub fn show_frame(&mut self) -> &mut Self {
        self.tag(1, &[])
    }

    pub fn background(&mut self, rgb: [u8; 3]) -> &mut Self {
        self.tag(9, &rgb)
    }

    pub fn rect_shape(&mut self, id: u16, w: i32, h: i32, rgba: [u8; 4]) -> &mut Self {
        let mut d = id.to_le_bytes().to_vec();
        d.extend(rect(0, w, 0, h));
        d.extend_from_slice(&[1, 0x00]);
        d.extend_from_slice(&rgba);
        d.push(0);
        let mut b = Bits::default();
        b.ub(1, 4);
        b.ub(0, 4);
        b.ub(0, 1);
        b.ub(0b00101, 5);
        b.ub(1, 5);
        b.sb(0, 1);
        b.sb(0, 1);
        b.ub(1, 1);
        for (dx, dy) in [(w, 0), (0, h), (-w, 0), (0, -h)] {
            let n = sbits(&[dx, dy]).max(2);
            b.ub(1, 1);
            b.ub(1, 1);
            b.ub(n - 2, 4);
            if dx != 0 && dy != 0 {
                b.ub(1, 1);
                b.sb(dx, n);
                b.sb(dy, n);
            } else {
                b.ub(0, 1);
                b.ub(u32::from(dx == 0), 1);
                b.sb(if dx == 0 { dy } else { dx }, n);
            }
        }
        b.ub(0, 6);
        d.extend(b.finish());
        self.tag(32, &d)
    }

    pub fn polygon_shape(&mut self, id: u16, points: &[(i32, i32)], rgba: [u8; 4]) -> &mut Self {
        let Some(&(x0, y0)) = points.first() else { return self };
        let (xs, ys) = (points.iter().map(|p| p.0), points.iter().map(|p| p.1));
        let (min_x, max_x) = (xs.clone().min().unwrap_or(0), xs.max().unwrap_or(0));
        let (min_y, max_y) = (ys.clone().min().unwrap_or(0), ys.max().unwrap_or(0));
        let mut d = id.to_le_bytes().to_vec();
        d.extend(rect(min_x, max_x, min_y, max_y));
        d.extend_from_slice(&[1, 0x00]);
        d.extend_from_slice(&rgba);
        d.push(0);
        let mut b = Bits::default();
        b.ub(1, 4);
        b.ub(0, 4);
        let n = sbits(&[x0, y0]);
        b.ub(0, 1);
        b.ub(0b00101, 5);
        b.ub(n, 5);
        b.sb(x0, n);
        b.sb(y0, n);
        b.ub(1, 1);
        let mut at = (x0, y0);
        for &p in points.iter().skip(1).chain(std::iter::once(&(x0, y0))) {
            let (dx, dy) = (p.0 - at.0, p.1 - at.1);
            at = p;
            if dx == 0 && dy == 0 {
                continue;
            }
            let n = sbits(&[dx, dy]).max(2);
            b.ub(1, 1);
            b.ub(1, 1);
            b.ub(n - 2, 4);
            b.ub(1, 1);
            b.sb(dx, n);
            b.sb(dy, n);
        }
        b.ub(0, 6);
        d.extend(b.finish());
        self.tag(32, &d)
    }

    pub fn font_placeholder(&mut self, id: u16, name: &str) -> &mut Self {
        let mut d = id.to_le_bytes().to_vec();
        d.push(0x04);
        d.push(1);
        d.push((name.len() + 1) as u8);
        d.extend_from_slice(name.as_bytes());
        d.push(0);
        d.extend_from_slice(&0u16.to_le_bytes());
        self.tag(75, &d)
    }

    pub fn edit_text(&mut self, f: &TextField) -> &mut Self {
        let mut d = f.id.to_le_bytes().to_vec();
        d.extend(rect(f.bounds.0, f.bounds.1, f.bounds.2, f.bounds.3));
        d.push(0x80 | 0x08 | 0x04 | 0x01);
        d.push(0x20 | 0x10 | if f.html { 0x02 } else { 0 } | 0x01);
        d.extend_from_slice(&f.font_id.to_le_bytes());
        d.extend_from_slice(&f.height.to_le_bytes());
        d.extend_from_slice(&f.rgba);
        d.push(f.align);
        d.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]);
        d.push(0);
        d.extend_from_slice(f.text.as_bytes());
        d.push(0);
        self.tag(37, &d)
    }

    pub fn place(&mut self, depth: u16, id: u16, name: Option<&str>, tx: i32, ty: i32) -> &mut Self {
        let mut d = vec![0x02 | 0x04 | if name.is_some() { 0x20 } else { 0 }];
        d.extend_from_slice(&depth.to_le_bytes());
        d.extend_from_slice(&id.to_le_bytes());
        d.extend(matrix(tx, ty, 1.0));
        if let Some(n) = name {
            d.extend_from_slice(n.as_bytes());
            d.push(0);
        }
        self.tag(26, &d)
    }

    pub fn sprite(&mut self, id: u16, inner: &Tags) -> &mut Self {
        let mut d = id.to_le_bytes().to_vec();
        d.extend_from_slice(&inner.frames.max(1).to_le_bytes());
        d.extend_from_slice(&inner.bytes);
        d.extend_from_slice(&[0, 0]);
        self.tag(39, &d)
    }

    pub fn do_action(&mut self, code: &[u8]) -> &mut Self {
        self.tag(12, code)
    }

    pub fn do_init_action(&mut self, sprite: u16, code: &[u8]) -> &mut Self {
        let mut d = sprite.to_le_bytes().to_vec();
        d.extend_from_slice(code);
        self.tag(59, &d)
    }

    pub fn export(&mut self, assets: &[(u16, &str)]) -> &mut Self {
        let mut d = (assets.len() as u16).to_le_bytes().to_vec();
        for (id, name) in assets {
            d.extend_from_slice(&id.to_le_bytes());
            d.extend_from_slice(name.as_bytes());
            d.push(0);
        }
        self.tag(56, &d)
    }

    pub fn import(&mut self, url: &str, assets: &[(u16, &str)]) -> &mut Self {
        let mut d = url.as_bytes().to_vec();
        d.extend_from_slice(&[0, 1, 0]);
        d.extend_from_slice(&(assets.len() as u16).to_le_bytes());
        for (id, name) in assets {
            d.extend_from_slice(&id.to_le_bytes());
            d.extend_from_slice(name.as_bytes());
            d.push(0);
        }
        self.tag(71, &d)
    }

    pub fn frames(&self) -> u16 {
        self.frames
    }
}

pub fn movie(version: u8, width: u16, height: u16, fps: u8, tags: &Tags, zlib: bool) -> Vec<u8> {
    let mut body = rect(0, i32::from(width) * 20, 0, i32::from(height) * 20);
    body.extend_from_slice(&[0, fps]);
    body.extend_from_slice(&tags.frames.max(1).to_le_bytes());
    body.extend_from_slice(&tags.bytes);
    body.extend_from_slice(&[0, 0]);
    let length = (8 + body.len()) as u32;
    let mut out = if zlib { b"CWS".to_vec() } else { b"FWS".to_vec() };
    out.push(version);
    out.extend_from_slice(&length.to_le_bytes());
    if zlib {
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(6));
        let _ = z.write_all(&body);
        out.extend(z.finish().unwrap_or_default());
    } else {
        out.extend(body);
    }
    out
}
