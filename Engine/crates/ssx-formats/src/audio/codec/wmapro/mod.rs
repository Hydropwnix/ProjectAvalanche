mod subframe;
mod transform;
mod vlc;

use std::f64::consts::PI;
use std::sync::LazyLock;

use super::wmapro_tables as t;
use crate::{Error, Result};

pub use transform::*;
pub use vlc::*;

pub const FRAME_SAMPLES: usize = 512;
pub const FRAME_LEN_BITS: usize = 15;
const MAX_SUBFRAMES: usize = 4;
const MIN_SUBFRAME: usize = FRAME_SAMPLES / MAX_SUBFRAMES;
const BLOCK_SIZES: usize = 3;
const MAX_BANDS: usize = 29;
const QUANT_BASE: i32 = 90;

#[derive(Clone)]
struct Channel {
    prev_block_len: usize,
    num_subframes: usize,
    subframe_len: [usize; MAX_SUBFRAMES],
    cur_subframe: usize,
    decoded_samples: usize,
    grouped: bool,
    transmit_coefs: bool,
    quant_step: i32,
    reuse_sf: bool,
    sf_step: i32,
    max_sf: i32,
    saved_sf: [[i32; MAX_BANDS]; 2],
    sf_last: usize,
    sf_cur: usize,
    sf_table: usize,
    num_vec_coeffs: usize,
    coeffs: [f32; FRAME_SAMPLES],
    out: [f32; FRAME_SAMPLES + FRAME_SAMPLES / 2],
}

impl Default for Channel {
    fn default() -> Self {
        Self {
            prev_block_len: FRAME_SAMPLES,
            num_subframes: 0,
            subframe_len: [0; MAX_SUBFRAMES],
            cur_subframe: 0,
            decoded_samples: 0,
            grouped: false,
            transmit_coefs: false,
            quant_step: 0,
            reuse_sf: false,
            sf_step: 1,
            max_sf: 0,
            saved_sf: [[0; MAX_BANDS]; 2],
            sf_last: 0,
            sf_cur: 0,
            sf_table: 0,
            num_vec_coeffs: 0,
            coeffs: [0.0; FRAME_SAMPLES],
            out: [0.0; FRAME_SAMPLES + FRAME_SAMPLES / 2],
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FrameStats {
    pub frames: u64,
    pub subframes: [u64; BLOCK_SIZES],
    pub split_tiling: u64,
    pub transforms: [u64; 3],
    pub postproc: u64,
    pub drc_nonzero: u64,
    pub trim_start: u64,
    pub trim_end: u64,
    pub ext_header: u64,
    pub sf_sets: [u64; 3],
    pub quant_escapes: u64,
    pub vec_escapes: u64,
    pub rl_escapes: u64,
    pub vec_count_sent: u64,
    pub rl_eob: u64,
    pub rl_full: u64,
    pub stale_sf: u64,
    pub end_bit_zero: u64,
}

impl FrameStats {
    pub fn add(&mut self, o: &FrameStats) {
        self.frames += o.frames;
        for i in 0..BLOCK_SIZES {
            self.subframes[i] += o.subframes[i];
            self.transforms[i] += o.transforms[i];
            self.sf_sets[i] += o.sf_sets[i];
        }
        self.split_tiling += o.split_tiling;
        self.postproc += o.postproc;
        self.drc_nonzero += o.drc_nonzero;
        self.trim_start += o.trim_start;
        self.trim_end += o.trim_end;
        self.ext_header += o.ext_header;
        self.quant_escapes += o.quant_escapes;
        self.vec_escapes += o.vec_escapes;
        self.rl_escapes += o.rl_escapes;
        self.vec_count_sent += o.vec_count_sent;
        self.rl_eob += o.rl_eob;
        self.rl_full += o.rl_full;
        self.stale_sf += o.stale_sf;
        self.end_bit_zero += o.end_bit_zero;
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameInfo {
    pub len: usize,
    pub trim_start: Option<u16>,
    pub trim_end: Option<u16>,
    pub drc_gain: u8,
    pub slack: isize,
    pub end_bit: bool,
    pub trailer: bool,
}

#[derive(Clone)]
pub struct StreamDecoder {
    channels: usize,
    bands: BandLayout,
    ch: [Channel; 2],
    subframe_len: usize,
    table_idx: usize,
    sub_channels: [usize; 2],
    num_sub_channels: usize,
    transmit_num_vec_coeffs: bool,
    groups: [Group; 2],
    num_groups: usize,
}

#[derive(Clone, Copy, Default)]
struct Group {
    channels: [usize; 2],
    num_channels: usize,
    transform: bool,
    transform_band: [bool; MAX_BANDS],
}

impl StreamDecoder {
    pub fn new(channels: usize, rate_index: u8) -> Result<Self> {
        Self::with_band_rate(channels, rate_of_index(rate_index))
    }

    pub fn with_band_rate(channels: usize, band_rate: u32) -> Result<Self> {
        if !(1..=2).contains(&channels) {
            return Err(Error::invalid(format!("XMA stream with {channels} channels (1 or 2)")));
        }
        Ok(Self {
            channels,
            bands: BandLayout::new(band_rate),
            ch: [Channel::default(), Channel::default()],
            subframe_len: FRAME_SAMPLES,
            table_idx: 0,
            sub_channels: [0; 2],
            num_sub_channels: 0,
            transmit_num_vec_coeffs: false,
            groups: [Group::default(); 2],
            num_groups: 0,
        })
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn tail(&self, out: &mut [[f32; FRAME_SAMPLES / 2]; 2]) {
        for (c, o) in self.ch[..self.channels].iter().zip(out.iter_mut()) {
            o.copy_from_slice(&c.out[..FRAME_SAMPLES / 2]);
        }
    }

    pub fn decode_frame(
        &mut self,
        data: &[u8],
        start: usize,
        out: &mut [[f32; FRAME_SAMPLES]; 2],
        stats: &mut FrameStats,
    ) -> Result<FrameInfo> {
        let mut r = BitReader::new(data, start, data.len() * 8);
        let len = r.bits(FRAME_LEN_BITS as u32) as usize;
        if len <= FRAME_LEN_BITS + 2 || start + len > data.len() * 8 {
            return Err(Error::invalid(format!("XMA frame length {len} at bit {start}")));
        }
        r = BitReader { end: start + len, ..r };
        let mut info = FrameInfo { len, ..Default::default() };
        self.tile_header(&mut r, stats)?;
        if self.channels > 1 && r.bit() {
            stats.postproc += 1;
            if r.bit() {
                return Err(Error::invalid("XMA frame with a post-processing matrix (none on the disc)"));
            }
        }
        info.drc_gain = r.bits(8) as u8;
        stats.drc_nonzero += u64::from(info.drc_gain != 0);
        if info.drc_gain != 0 {
            return Err(Error::invalid("XMA frame with a nonzero dynamic-range gain (none on the disc)"));
        }
        if r.bit() {
            if r.bit() {
                info.trim_start = Some(r.bits(10) as u16);
                stats.trim_start += 1;
            }
            if r.bit() {
                info.trim_end = Some(r.bits(10) as u16);
                stats.trim_end += 1;
            }
        }
        for c in &mut self.ch[..self.channels] {
            c.decoded_samples = 0;
            c.cur_subframe = 0;
            c.reuse_sf = false;
        }
        while self.ch[..self.channels].iter().any(|c| c.decoded_samples < FRAME_SAMPLES) {
            self.subframe(&mut r, stats)?;
        }
        info.slack = r.left();
        if info.slack != 2 {
            return Err(Error::invalid(format!(
                "XMA frame at bit {start}: {} bits left after the subframes of a {len}-bit frame (2 expected)",
                info.slack
            )));
        }
        info.end_bit = r.bit();
        info.trailer = r.bit();
        stats.end_bit_zero += u64::from(!info.end_bit);
        for (c, o) in self.ch[..self.channels].iter_mut().zip(out.iter_mut()) {
            o.copy_from_slice(&c.out[..FRAME_SAMPLES]);
            c.out.copy_within(FRAME_SAMPLES.., 0);
        }
        stats.frames += 1;
        Ok(info)
    }

    fn tile_header(&mut self, r: &mut BitReader, stats: &mut FrameStats) -> Result<()> {
        let n = self.channels;
        let mut num_samples = [0usize; 2];
        let mut contains = [false; 2];
        for c in &mut self.ch[..n] {
            c.num_subframes = 0;
        }
        let fixed = r.bit();
        let mut min_len = 0;
        let mut channels_for_subframe = n;
        while min_len < FRAME_SAMPLES {
            for c in 0..n {
                contains[c] = num_samples[c] == min_len
                    && (fixed || channels_for_subframe == 1 || min_len == FRAME_SAMPLES - MIN_SUBFRAME || r.bit());
            }
            let len = if min_len == FRAME_SAMPLES - MIN_SUBFRAME {
                MIN_SUBFRAME
            } else if r.bit() {
                FRAME_SAMPLES >> (1 + r.bits(1))
            } else {
                FRAME_SAMPLES
            };
            if r.pos() > r.end() {
                return Err(Error::invalid("XMA tile header runs past the frame"));
            }
            min_len += len;
            for c in 0..n {
                let ch = &mut self.ch[c];
                if contains[c] {
                    if ch.num_subframes >= MAX_SUBFRAMES || num_samples[c] + len > FRAME_SAMPLES {
                        return Err(Error::invalid("XMA tile header: channel overfull"));
                    }
                    ch.subframe_len[ch.num_subframes] = len;
                    ch.num_subframes += 1;
                    num_samples[c] += len;
                } else if num_samples[c] <= min_len {
                    if num_samples[c] < min_len {
                        channels_for_subframe = 0;
                        min_len = num_samples[c];
                    }
                    channels_for_subframe += 1;
                }
            }
        }
        if n == 2
            && self.ch[0].subframe_len[..self.ch[0].num_subframes]
                != self.ch[1].subframe_len[..self.ch[1].num_subframes]
        {
            stats.split_tiling += 1;
            return Err(Error::invalid("XMA tile header: channels tiled apart (none on the disc)"));
        }
        if self.ch[..n].iter().any(|c| c.subframe_len[..c.num_subframes].iter().any(|&l| l != MIN_SUBFRAME)) {
            return Err(Error::invalid("XMA tile header: a subframe other than 128 samples (none on the disc)"));
        }
        Ok(())
    }
}

fn overlap_window(buf: &mut [f32], b: usize, w: &[f32]) {
    let (l, h) = (w.len(), w.len() / 2);
    for i in 0..h {
        let (p, q) = (buf[b - h + i], buf[b + h - 1 - i]);
        let (wl, wh) = (w[i], w[l - 1 - i]);
        buf[b - h + i] = p * wh - q * wl;
        buf[b + h - 1 - i] = p * wl + q * wh;
    }
}

#[cfg(test)]
pub(crate) mod test_util {
    #[derive(Default)]
    pub struct BitWriter {
        pub bytes: Vec<u8>,
        pub len: usize,
    }

    impl BitWriter {
        pub fn put(&mut self, v: u32, n: usize) {
            for i in (0..n).rev() {
                if self.len.is_multiple_of(8) {
                    self.bytes.push(0);
                }
                if (v >> i) & 1 != 0 {
                    *self.bytes.last_mut().unwrap() |= 0x80 >> (self.len % 8);
                }
                self.len += 1;
            }
        }
    }

    pub fn silent_frame(channels: usize, trim_start: Option<u16>, trim_end: Option<u16>, more: bool) -> BitWriter {
        let mut body = BitWriter::default();
        body.put(1, 1);
        for _ in 0..3 {
            body.put(0b11, 2);
        }
        if channels > 1 {
            body.put(0, 1);
        }
        body.put(0, 8);
        body.put(u32::from(trim_start.is_some() || trim_end.is_some()), 1);
        if trim_start.is_some() || trim_end.is_some() {
            for t in [trim_start, trim_end] {
                body.put(u32::from(t.is_some()), 1);
                if let Some(v) = t {
                    body.put(u32::from(v), 10);
                }
            }
        }
        for _ in 0..4 {
            body.put(0, 2);
            if channels > 1 {
                body.put(0, 1);
                body.put(0b10, 2);
            }
            body.put(0, channels);
        }
        body.put(1, 1);
        body.put(u32::from(more), 1);
        let mut f = BitWriter::default();
        f.put((body.len + super::FRAME_LEN_BITS) as u32, super::FRAME_LEN_BITS);
        for i in 0..body.len {
            f.put(u32::from(body.bytes[i / 8] >> (7 - i % 8) & 1), 1);
        }
        f
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(n: usize, seed: u64) -> Vec<f64> {
        let mut x = seed;
        (0..n)
            .map(|_| {
                x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                (x >> 11) as f64 / (1u64 << 52) as f64 - 1.0
            })
            .collect()
    }

    fn basis(n: usize, m: usize, k: usize) -> f64 {
        (PI / n as f64 * (m as f64 + 0.5 + n as f64 / 2.0) * (k as f64 + 0.5)).cos()
    }

    #[test]
    fn imdct_matches_direct_formula() {
        for n in [128, 256, 512] {
            let x = noise(n, n as u64);
            let coef: Vec<f32> = x.iter().map(|&v| v as f32).collect();
            let mut out = vec![0f32; n];
            Imdct::new(n, 2.0 / n as f64).run(&coef, &mut out);
            for (j, &o) in out.iter().enumerate() {
                let want: f64 = (0..n).map(|k| x[k] * basis(n, n / 2 + j, k)).sum::<f64>() * 2.0 / n as f64;
                assert!((f64::from(o) - want).abs() < 1e-4, "n {n} j {j}: {o} vs {want}");
            }
        }
    }

    #[test]
    fn code_tables_are_complete_prefix_codes() {
        let c = &*CODES;
        for (name, v, n) in [
            ("scale", &c.scale, 121),
            ("scale_rl", &c.scale_rl, 120),
            ("coef0", &c.coef[0], 272),
            ("coef1", &c.coef[1], 244),
            ("vec4", &c.vec4, 127),
            ("vec2", &c.vec2, 137),
            ("vec1", &c.vec1, 101),
        ] {
            assert_eq!(v.kraft(), 1.0, "{name}");
            let mut syms = v.syms.clone();
            syms.sort_unstable();
            syms.dedup();
            assert_eq!(syms.len(), n, "{name}: symbols not unique");
        }
        assert_eq!((c.scale_rl_runlevel.len(), c.coef_runlevel[0].len(), c.coef_runlevel[1].len()), (120, 272, 244));
        assert_eq!(c.coef_runlevel[0][271], (0, 28));
        assert_eq!(c.coef_runlevel[1][151], (0, 6));
        assert!(Vlc::new(&[1, 1, 1], &[0, 1, 2]).is_err());
        assert!(Vlc::new(&[1, 2], &[0, 1]).is_err());
    }

    #[test]
    fn vlc_round_trip() {
        let c = &*CODES;
        for v in [&c.scale, &c.scale_rl, &c.coef[0], &c.coef[1], &c.vec4, &c.vec2, &c.vec1] {
            let mut w = test_util::BitWriter::default();
            for (&code, &len) in v.codes.iter().zip(&v.lens) {
                w.put(code >> (32 - u32::from(len)), usize::from(len));
            }
            let mut r = BitReader::new(&w.bytes, 0, w.len);
            for &sym in &v.syms {
                assert_eq!(v.read(&mut r), sym);
            }
            assert_eq!(r.pos(), w.len);
        }
    }

    #[test]
    fn bit_reader() {
        let data = [0b1011_0011, 0xFF, 0x01];
        let mut r = BitReader::new(&data, 0, 24);
        assert_eq!(r.bits(3), 0b101);
        assert_eq!(r.sbits(3), -4);
        assert_eq!(r.pos(), 6);
        assert_eq!(r.bits(10), 0b11_1111_1111);
        assert_eq!(r.bits(8), 0b0000_0001);
        assert_eq!(r.left(), 0);
        assert_eq!(r.bits(16), 0);
        assert!(r.left() < 0);
    }

    #[test]
    fn mdct_overlap_add_reconstructs() {
        let n = 128;
        let x = noise(6 * n, 7);
        let w: Vec<f64> = (0..2 * n).map(|m| ((m as f64 + 0.5) * PI / (2.0 * n as f64)).sin()).collect();
        let imdct = Imdct::new(n, 2.0 / n as f64);
        let window = &TRANSFORMS.windows[2];
        let mut buf = vec![0f32; 6 * n];
        for j in 1..5 {
            let coef: Vec<f32> = (0..n)
                .map(|k| (0..2 * n).map(|m| w[m] * x[j * n - n / 2 + m] * basis(n, m, k)).sum::<f64>() as f32)
                .collect();
            imdct.run(&coef, &mut buf[j * n..j * n + n]);
            if j > 1 {
                overlap_window(&mut buf, j * n, window);
            }
        }
        for i in 2 * n - n / 2..4 * n + n / 2 {
            assert!((f64::from(buf[i]) - x[i]).abs() < 1e-4, "sample {i}: {} vs {}", buf[i], x[i]);
        }
    }

    #[test]
    fn band_layouts() {
        for rate in [24000, 32000, 44100, 48000] {
            let l = BandLayout::new(rate);
            for i in 0..BLOCK_SIZES {
                let e = &l.offsets[i][..=l.num_bands[i]];
                assert!(e.windows(2).all(|p| p[0] < p[1] && p[0] % 4 == 0), "{rate} {i}: {e:?}");
                assert_eq!(usize::from(e[e.len() - 1]), FRAME_SAMPLES >> i);
                for x in 0..BLOCK_SIZES {
                    assert!(l.resample[i][x][..l.num_bands[i]].iter().all(|&b| usize::from(b) < l.num_bands[x]));
                }
            }
        }
        let nb = |r| BandLayout::new(r).num_bands[2];
        assert_eq!([nb(24000), nb(32000), nb(36000), nb(44100), nb(48000)], [15, 15, 15, 14, 14]);
    }

    #[test]
    fn silent_frames_decode() {
        for channels in [1, 2] {
            let mut dec = StreamDecoder::new(channels, 2).unwrap();
            let mut out = [[1f32; FRAME_SAMPLES]; 2];
            let mut stats = FrameStats::default();
            let f = test_util::silent_frame(channels, Some(512), Some(77), true);
            let mut bytes = f.bytes.clone();
            bytes.extend([0; 8]);
            let info = dec.decode_frame(&bytes, 0, &mut out, &mut stats).unwrap();
            assert_eq!((info.len, info.trim_start, info.trim_end, info.slack), (f.len, Some(512), Some(77), 2));
            assert!(info.end_bit && info.trailer);
            assert!(out[..channels].iter().all(|o| o.iter().all(|&v| v == 0.0)));
            assert_eq!((stats.frames, stats.subframes[2], stats.trim_start), (1, 4, 1));
            let mut bad = bytes.clone();
            bad[1] ^= 0x02;
            assert!(dec.decode_frame(&bad, 0, &mut out, &mut stats).is_err());
        }
    }
}
