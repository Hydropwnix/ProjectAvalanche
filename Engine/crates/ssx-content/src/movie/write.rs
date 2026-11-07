pub fn element(id: u32, body: &[u8]) -> Vec<u8> {
    element_sized(id, body, None)
}

pub fn element_with(id: u32, body: &[u8], width: Option<usize>, lie: Option<u64>) -> Vec<u8> {
    let mut out = id_bytes(id);
    out.extend(size_bytes(lie.unwrap_or(body.len() as u64), width));
    out.extend_from_slice(body);
    out
}

fn element_sized(id: u32, body: &[u8], width: Option<usize>) -> Vec<u8> {
    element_with(id, body, width, None)
}

fn id_bytes(id: u32) -> Vec<u8> {
    let b = id.to_be_bytes();
    let skip = b.iter().take_while(|&&x| x == 0).count().min(3);
    b[skip..].to_vec()
}

pub fn size_bytes(v: u64, width: Option<usize>) -> Vec<u8> {
    if v == u64::MAX {
        return vec![0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
    }
    let n = width.unwrap_or_else(|| (1..=8).find(|&n| v < (1u64 << (7 * n)) - 1).unwrap_or(8));
    let mut out = vec![0u8; n];
    let mut x = v | (1u64 << (7 * n));
    for i in (0..n).rev() {
        out[i] = x as u8;
        x >>= 8;
    }
    out
}

pub fn uint(id: u32, v: u64) -> Vec<u8> {
    let b = v.to_be_bytes();
    let skip = b.iter().take_while(|&&x| x == 0).count().min(7);
    element(id, &b[skip..])
}

pub fn float(id: u32, v: f64) -> Vec<u8> {
    element(id, &v.to_be_bytes())
}

pub fn string(id: u32, s: &str) -> Vec<u8> {
    element(id, s.as_bytes())
}

pub fn ebml_header(doc_type: &str) -> Vec<u8> {
    let body = [
        uint(0x4286, 1),
        uint(0x42F7, 1),
        uint(0x42F2, 4),
        uint(0x42F3, 8),
        string(0x4282, doc_type),
        uint(0x4287, 4),
        uint(0x4285, 2),
    ]
    .concat();
    element(0x1A45_DFA3, &body)
}

pub fn info(duration_ms: f64) -> Vec<u8> {
    element(0x1549_A966, &[uint(0x2A_D7B1, 1_000_000), string(0x4D80, "ssx-test"), float(0x4489, duration_ms)].concat())
}

pub fn video_track(number: u64, w: u64, h: u64, private: &[u8], default_duration_ns: Option<u64>) -> Vec<u8> {
    let mut body = [uint(0xD7, number), uint(0x73C5, number), uint(0x83, 1), string(0x86, "V_AV1")].concat();
    if let Some(d) = default_duration_ns {
        body.extend(uint(0x23_E383, d));
    }
    body.extend(element(0x63A2, private));
    body.extend(element(0xE0, &[uint(0xB0, w), uint(0xBA, h)].concat()));
    element(0xAE, &body)
}

pub fn audio_track(number: u64, channels: u64, rate: f64, private: &[u8]) -> Vec<u8> {
    let body = [
        uint(0xD7, number),
        uint(0x73C5, number),
        uint(0x83, 2),
        string(0x86, "A_VORBIS"),
        element(0x63A2, private),
        element(0xE1, &[uint(0x9F, channels), float(0xB5, rate)].concat()),
    ]
    .concat();
    element(0xAE, &body)
}

pub fn simple_block(track: u8, rel: i16, flags: u8, data: &[u8]) -> Vec<u8> {
    let mut body = vec![0x80 | track];
    body.extend(rel.to_be_bytes());
    body.push(flags);
    body.extend_from_slice(data);
    element(0xA3, &body)
}

pub fn cluster(timestamp: u64, children: &[Vec<u8>]) -> Vec<u8> {
    let mut body = uint(0xE7, timestamp);
    for c in children {
        body.extend_from_slice(c);
    }
    element(0x1F43_B675, &body)
}

pub fn file(children: &[Vec<u8>]) -> Vec<u8> {
    let mut out = ebml_header("webm");
    out.extend(element(0x1853_8067, &children.concat()));
    out
}

pub fn xiph_lace(packets: [&[u8]; 3]) -> Vec<u8> {
    let mut out = vec![2u8];
    for p in &packets[..2] {
        let mut n = p.len();
        while n >= 255 {
            out.push(255);
            n -= 255;
        }
        out.push(n as u8);
    }
    for p in packets {
        out.extend_from_slice(p);
    }
    out
}

#[derive(Default)]
pub struct Bits(Vec<u8>, usize);

impl Bits {
    pub fn new() -> Self {
        Bits(Vec::new(), 0)
    }
    pub fn put(&mut self, n: u32, v: u64) -> &mut Self {
        for i in (0..n).rev() {
            if self.1.is_multiple_of(8) {
                self.0.push(0);
            }
            let bit = ((v >> i) & 1) as u8;
            let last = self.0.len() - 1;
            self.0[last] |= bit << (7 - self.1 % 8);
            self.1 += 1;
        }
        self
    }
    pub fn finish(mut self) -> Vec<u8> {
        self.put(1, 1);
        self.0
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Seq {
    pub profile: u64,
    pub still: bool,
    pub w: u64,
    pub h: u64,
    pub ten_bit: bool,
    pub mono: bool,
    pub matrix: Option<u64>,
    pub grain: bool,
    pub timing_fps: Option<u64>,
    pub points: u64,
}

pub const BASE_SEQ: Seq = Seq {
    profile: 0,
    still: false,
    w: 320,
    h: 180,
    ten_bit: false,
    mono: false,
    matrix: Some(1),
    grain: false,
    timing_fps: None,
    points: 1,
};

pub fn seq_header(s: Seq) -> Vec<u8> {
    let mut b = Bits::new();
    b.put(3, s.profile).put(1, u64::from(s.still)).put(1, 0);
    match s.timing_fps {
        Some(fps) => {
            b.put(1, 1).put(32, 1).put(32, fps).put(1, 1).put(1, 1).put(1, 0);
        }
        None => {
            b.put(1, 0);
        }
    }
    b.put(1, 0).put(5, s.points - 1);
    for _ in 0..s.points {
        b.put(12, 0).put(5, 8).put(1, 0);
    }
    b.put(4, 15).put(4, 15).put(16, s.w - 1).put(16, s.h - 1);
    b.put(1, 0);
    b.put(7, 0).put(1, 0);
    b.put(1, 1);
    b.put(1, 1);
    b.put(3, 0);
    b.put(1, u64::from(s.ten_bit)).put(1, u64::from(s.mono));
    match s.matrix {
        Some(m) => b.put(1, 1).put(8, 1).put(8, 1).put(8, m),
        None => b.put(1, 0),
    };
    if s.mono {
        b.put(1, 0);
    } else {
        b.put(1, 0).put(2, 0).put(1, 0);
    }
    b.put(1, u64::from(s.grain));
    b.finish()
}

pub fn obu(kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![(kind << 3) | 0x02];
    let mut n = payload.len();
    loop {
        let byte = (n & 0x7F) as u8;
        n >>= 7;
        out.push(if n > 0 { byte | 0x80 } else { byte });
        if n == 0 {
            break;
        }
    }
    out.extend_from_slice(payload);
    out
}

pub fn av1_private(s: Seq) -> Vec<u8> {
    let flags = if s.mono { 0x1C } else { 0x0C } | if s.ten_bit { 0x40 } else { 0 };
    let mut out = vec![0x81, ((s.profile as u8) << 5) | 8, flags, 0];
    out.extend(obu(super::av1::OBU_SEQUENCE_HEADER, &seq_header(s)));
    out
}

pub fn unit() -> Vec<u8> {
    obu(super::av1::OBU_FRAME, &[0x10, 0x00, 0xAB, 0xCD])
}

pub struct Lsbits(pub Vec<u8>, pub usize);

impl Lsbits {
    pub fn put(&mut self, n: u32, v: u64) -> &mut Self {
        for i in 0..n {
            if self.1.is_multiple_of(8) {
                self.0.push(0);
            }
            let last = self.0.len() - 1;
            self.0[last] |= (((v >> i) & 1) as u8) << (self.1 % 8);
            self.1 += 1;
        }
        self
    }
}

pub fn codebook(b: &mut Lsbits, entries: u64, dims: u64, lookup: u64) {
    b.put(24, 0x56_4342).put(16, dims).put(24, entries).put(1, 1);
    let len = 64 - (entries - 1).leading_zeros() as u64;
    b.put(5, len.max(1) - 1);
    let bits = 32 - (entries as u32).leading_zeros();
    b.put(bits, entries);
    b.put(4, lookup);
    if lookup > 0 {
        b.put(32, 0).put(32, 0).put(4, 0).put(1, 0);
        let lookup1 =
            (1..=entries).take_while(|r| r.checked_pow(dims as u32).is_some_and(|p| p <= entries)).last().unwrap_or(0);
        let values = if lookup == 1 { lookup1 } else { entries * dims };
        for _ in 0..values {
            b.put(1, 0);
        }
    }
}

pub fn vorbis_setup(books: &[(u64, u64, u64)]) -> Vec<u8> {
    let mut b = Lsbits(b"\x05vorbis".to_vec(), 56);
    b.put(8, books.len() as u64 - 1);
    for &(e, d, l) in books {
        codebook(&mut b, e, d, l);
    }
    b.put(6, 0).put(16, 0);
    b.put(6, 0).put(16, 1).put(5, 0).put(2, 0).put(4, 8);
    b.put(6, 0).put(16, 0).put(24, 0).put(24, 0).put(24, 31).put(6, 0).put(8, 0).put(3, 0).put(1, 0);
    b.put(6, 0).put(16, 0).put(1, 0).put(1, 0).put(2, 0).put(8, 0).put(8, 0).put(8, 0);
    b.put(6, 0).put(1, 0).put(16, 0).put(16, 0).put(8, 0);
    b.put(1, 1);
    b.0
}

pub fn vorbis_private(channels: u8, rate: u32, setup: &[u8]) -> Vec<u8> {
    let mut id = b"\x01vorbis".to_vec();
    id.extend(0u32.to_le_bytes());
    id.push(channels);
    id.extend(rate.to_le_bytes());
    id.extend([0u8; 12]);
    id.push(0xB8);
    id.push(1);
    let mut comment = b"\x03vorbis".to_vec();
    comment.extend(4u32.to_le_bytes());
    comment.extend(b"test");
    comment.extend(0u32.to_le_bytes());
    comment.push(1);
    xiph_lace([&id, &comment, setup])
}
