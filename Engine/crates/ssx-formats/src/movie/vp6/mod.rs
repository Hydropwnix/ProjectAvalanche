#![allow(clippy::needless_range_loop)]

mod coeffs;
mod dsp;
mod huffman;
mod macroblock;
mod models;
mod rac;
mod tables;

use crate::{Error, Result};
use huffman::{BitReader, COEFF_MAP, RUN_MAP, Tree};
use rac::RangeDecoder;
use tables::*;

#[derive(Clone, Debug, Default)]
pub struct Picture {
    pub width: usize,
    pub height: usize,
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
}

impl Picture {
    fn new(width: usize, height: usize) -> Self {
        let c = (width / 2) * (height / 2);
        Self { width, height, y: vec![0; width * height], u: vec![0; c], v: vec![0; c] }
    }

    fn plane(&self, p: usize) -> (&[u8], usize) {
        match p {
            0 => (&self.y, self.width),
            1 => (&self.u, self.width / 2),
            _ => (&self.v, self.width / 2),
        }
    }

    fn plane_mut(&mut self, p: usize) -> (&mut [u8], usize) {
        match p {
            0 => (&mut self.y, self.width),
            1 => (&mut self.u, self.width / 2),
            _ => (&mut self.v, self.width / 2),
        }
    }

    pub fn write_i420(&self, out: &mut Vec<u8>) {
        for (plane, w) in [(&self.y, self.width), (&self.u, self.width / 2), (&self.v, self.width / 2)] {
            for row in plane.chunks_exact(w).rev() {
                out.extend_from_slice(row);
            }
        }
    }

    pub fn write_rgba(&self, out: &mut Vec<u8>) {
        let (w, h, cw, ch) = (self.width, self.height, self.width / 2, self.height / 2);
        let taps = |p: usize, n: usize| -> (usize, usize, f32) {
            let (a, b, wb) = if p.is_multiple_of(2) {
                (p / 2).checked_sub(1).map_or((0, 0, 0.75), |a| (a, a + 1, 0.75))
            } else {
                (p / 2, p / 2 + 1, 0.25)
            };
            (a.min(n - 1), b.min(n - 1), wb)
        };
        let cols: Vec<_> = (0..w).map(|x| taps(x, cw)).collect();
        out.reserve(w * h * 4);
        for y in 0..h {
            let (r0, r1, wy) = taps(y, ch);
            let (r0, r1) = ((ch - 1 - r0) * cw, (ch - 1 - r1) * cw);
            let yrow = &self.y[(h - 1 - y) * w..(h - y) * w];
            for (x, &(c0, c1, wx)) in cols.iter().enumerate() {
                let sample = |p: &[u8]| {
                    let top = f32::from(p[r0 + c0]) * (1.0 - wx) + f32::from(p[r0 + c1]) * wx;
                    let bottom = f32::from(p[r1 + c0]) * (1.0 - wx) + f32::from(p[r1 + c1]) * wx;
                    (top * (1.0 - wy) + bottom * wy) / 255.0
                };
                let (l, u, v) = (f32::from(yrow[x]) / 255.0, sample(&self.u), sample(&self.v));
                let r = 1.164383 * l + 1.596027 * v - 0.870_787_4;
                let g = (1.164383 * l - 0.072_773_94) + (-0.391762 * u - 0.812968 * v) + 0.602365;
                let b = 1.164383 * l + 2.017232 * u - 1.081_389_9;
                let q = |c: f32| (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                out.extend_from_slice(&[q(r), q(g), q(b), 255]);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameInfo {
    pub key: bool,
    pub quantizer: u8,
    pub golden: bool,
    pub coeff_offset: usize,
    pub huffman: bool,
    pub mb_types: [u32; 10],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Mv {
    x: i32,
    y: i32,
}

#[derive(Clone, Copy, Default)]
struct MbInfo {
    ty: u8,
    mv: Mv,
}

#[derive(Clone, Copy)]
struct RefDc {
    not_null_dc: bool,
    ref_frame: u8,
    dc: i16,
}

const REF_NONE: u8 = 0xFF;
const NO_DC: RefDc = RefDc { not_null_dc: false, ref_frame: REF_NONE, dc: 0 };

#[derive(Clone)]
struct Model {
    coeff_reorder: [u8; 64],
    index_to_pos: [u8; 64],
    vector_sig: [u8; 2],
    vector_dct: [u8; 2],
    vector_pdv: [[u8; 7]; 2],
    vector_fdv: [[u8; 8]; 2],
    coeff_dccv: [[u8; 11]; 2],
    coeff_ract: [[[[u8; 11]; 6]; 3]; 2],
    coeff_dcct: [[[u8; 5]; 3]; 2],
    coeff_runv: [[u8; 14]; 2],
    mb_type: [[[u8; 10]; 10]; 3],
    mb_types_stats: [[[u8; 2]; 10]; 3],
}

impl Default for Model {
    fn default() -> Self {
        Self {
            coeff_reorder: [0; 64],
            index_to_pos: [0; 64],
            vector_sig: [0; 2],
            vector_dct: [0; 2],
            vector_pdv: [[0; 7]; 2],
            vector_fdv: [[0; 8]; 2],
            coeff_dccv: [[0; 11]; 2],
            coeff_ract: [[[[0; 11]; 6]; 3]; 2],
            coeff_dcct: [[[0; 5]; 3]; 2],
            coeff_runv: [[0; 14]; 2],
            mb_type: [[[0; 10]; 10]; 3],
            mb_types_stats: [[[0; 2]; 10]; 3],
        }
    }
}

struct State {
    mb_w: usize,
    mb_h: usize,
    model: Model,
    dequant_dc: i32,
    dequant_ac: i32,
    macroblocks: Vec<MbInfo>,
    above: Vec<RefDc>,
    left: [RefDc; 4],
    above_idx: [usize; 6],
    prev_dc: [[i16; 3]; 3],
    coeffs: [[i16; 64]; 6],
    mv: [Mv; 6],
    candidates: [Mv; 2],
    candidate_pos: usize,
    prev_type: u8,
    huff_dc: [Tree; 2],
    huff_run: [Tree; 2],
    huff_ac: [[[Tree; 4]; 3]; 2],
    nb_null: [[u32; 2]; 2],
}

impl Default for RefDc {
    fn default() -> Self {
        NO_DC
    }
}

impl Default for State {
    fn default() -> Self {
        Self {
            mb_w: 0,
            mb_h: 0,
            model: Model::default(),
            dequant_dc: 0,
            dequant_ac: 0,
            macroblocks: Vec::new(),
            above: Vec::new(),
            left: [NO_DC; 4],
            above_idx: [0; 6],
            prev_dc: [[0; 3]; 3],
            coeffs: [[0; 64]; 6],
            mv: [Mv::default(); 6],
            candidates: [Mv::default(); 2],
            candidate_pos: 0,
            prev_type: 0,
            huff_dc: [Tree::default(); 2],
            huff_run: [Tree::default(); 2],
            huff_ac: [[[Tree::default(); 4]; 3]; 2],
            nb_null: [[0; 2]; 2],
        }
    }
}

#[derive(Default)]
pub struct Decoder {
    pics: [Picture; 3],
    prev: Option<usize>,
    golden: Option<usize>,
    st: State,
}

enum Coeffs<'a> {
    Main,
    Range(RangeDecoder<'a>),
    Huffman(BitReader<'a>),
}

struct Header {
    key: bool,
    quantizer: u8,
    start: usize,
    coeff_offset: usize,
}

fn parse_header(data: &[u8], sizes: Option<(usize, usize)>) -> Result<(Header, Option<(usize, usize)>)> {
    let err = |m: &str| Err(Error::invalid(format!("VP6 frame: {m}")));
    let key = data.first().is_some_and(|b| b & 0x80 == 0);
    if data.len() < if key { 8 } else { 3 } {
        return err("shorter than its header");
    }
    let quantizer = (data[0] >> 1) & 0x3F;
    if data[0] & 1 == 0 {
        return err("single-partition frame (not on the disc)");
    }
    let be16 = |o: usize| usize::from(u16::from_be_bytes([data[o], data[o + 1]]));
    if key {
        let (version, profile, interlaced) = (data[1] >> 3, (data[1] >> 1) & 3, data[1] & 1);
        if version != 7 || profile != 0 || interlaced != 0 {
            return err(&format!("version {version} profile {profile} interlaced {interlaced} (not on the disc)"));
        }
        let (rows, cols) = (usize::from(data[4]), usize::from(data[5]));
        if rows == 0 || cols == 0 || data[6] != data[4] || data[7] != data[5] {
            return err("bad macroblock dimensions");
        }
        let h = Header { key, quantizer, start: 8, coeff_offset: be16(2) };
        Ok((h, Some((cols, rows))))
    } else {
        if sizes.is_none() {
            return err("inter frame before the first key frame");
        }
        Ok((Header { key, quantizer, start: 3, coeff_offset: be16(1) }, sizes))
    }
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn picture(&self) -> Option<&Picture> {
        self.prev.map(|i| &self.pics[i])
    }

    pub fn decode_frame(&mut self, data: &[u8]) -> Result<FrameInfo> {
        let sizes = (self.st.mb_w > 0).then_some((self.st.mb_w, self.st.mb_h));
        let (h, dims) = parse_header(data, sizes)?;
        let (mb_w, mb_h) = dims.expect("checked by parse_header");
        if sizes.is_some_and(|s| s != (mb_w, mb_h)) {
            return Err(Error::invalid("VP6 frame: size change within a stream (not on the disc)"));
        }
        if sizes.is_none() {
            self.init(mb_w, mb_h);
        }
        let q = usize::from(h.quantizer);
        self.st.dequant_dc = i32::from(DC_DEQUANT[q]) << 2;
        self.st.dequant_ac = i32::from(AC_DEQUANT[q]) << 2;

        let mut c = RangeDecoder::new(&data[h.start..]);
        let mut golden = h.key;
        if h.key {
            c.literal(2);
        } else {
            golden = c.flag();
        }
        let huffman = c.flag();
        let mut coeffs = match h.coeff_offset {
            2 => Coeffs::Main,
            o if o > h.start && o <= data.len() && huffman => Coeffs::Huffman(BitReader::new(&data[o..])),
            o if o > h.start && o <= data.len() => Coeffs::Range(RangeDecoder::new(&data[o..])),
            o => return Err(Error::invalid(format!("VP6 frame: coefficient partition at {o} of {}", data.len()))),
        };
        let huffman = huffman && !matches!(coeffs, Coeffs::Main);

        let cur = (0..3).find(|i| Some(*i) != self.prev && Some(*i) != self.golden).expect("3 buffers, 2 references");
        let mut info = FrameInfo {
            key: h.key,
            quantizer: h.quantizer,
            golden,
            coeff_offset: h.coeff_offset,
            huffman,
            ..Default::default()
        };
        let st = &mut self.st;
        if h.key {
            st.default_models();
            st.macroblocks.fill(MbInfo { ty: mb::INTRA, mv: Mv::default() });
        } else {
            st.parse_mb_type_models(&mut c);
            st.parse_vector_models(&mut c);
            st.prev_type = mb::INTER_NOVEC_PF;
        }
        st.parse_coeff_models(&mut c, h.key, huffman);

        st.prev_dc = [[0; 3]; 3];
        st.prev_dc[1][usize::from(REF_CURRENT)] = 128;
        st.prev_dc[2][usize::from(REF_CURRENT)] = 128;
        st.above.fill(NO_DC);
        let (cur_pic, refs) = split_pics(&mut self.pics, cur, self.prev, self.golden);
        for row in 0..mb_h {
            st.left = [NO_DC; 4];
            st.above_idx = [1, 2, 1, 2, 2 * mb_w + 3, 3 * mb_w + 5];
            for col in 0..mb_w {
                let ty = if h.key { mb::INTRA } else { st.decode_mv(&mut c, row, col) };
                info.mb_types[usize::from(ty)] += 1;
                match &mut coeffs {
                    Coeffs::Main => st.parse_coeff(&mut c)?,
                    Coeffs::Range(cc) => st.parse_coeff(cc)?,
                    Coeffs::Huffman(br) => st.parse_coeff_huffman(br)?,
                }
                st.render_mb(cur_pic, &refs, row, col, ty)?;
                for i in &mut st.above_idx[..4] {
                    *i += 2;
                }
                for i in &mut st.above_idx[4..] {
                    *i += 1;
                }
            }
        }
        if golden {
            self.golden = Some(cur);
        }
        self.prev = Some(cur);
        Ok(info)
    }

    fn init(&mut self, mb_w: usize, mb_h: usize) {
        self.pics = std::array::from_fn(|_| Picture::new(mb_w * 16, mb_h * 16));
        self.st.mb_w = mb_w;
        self.st.mb_h = mb_h;
        self.st.macroblocks = vec![MbInfo::default(); mb_w * mb_h];
        self.st.above = vec![NO_DC; 4 * mb_w + 6];
    }
}

fn split_pics(
    pics: &mut [Picture; 3],
    cur: usize,
    prev: Option<usize>,
    golden: Option<usize>,
) -> (&mut Picture, [Option<&Picture>; 3]) {
    let [a, b, c] = pics;
    let (m, o1, o2, i1, i2) = match cur {
        0 => (a, &*b, &*c, 1, 2),
        1 => (b, &*a, &*c, 0, 2),
        _ => (c, &*a, &*b, 0, 1),
    };
    let get = |i: Option<usize>| match i {
        Some(i) if i == i1 => Some(o1),
        Some(i) if i == i2 => Some(o2),
        _ => None,
    };
    (m, [None, get(prev), get(golden)])
}
