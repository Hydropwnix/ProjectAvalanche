use std::ffi::c_int;
use std::mem::MaybeUninit;
use std::ptr::NonNull;

use rav1d::Dav1dResult;
use rav1d::include::dav1d::data::Dav1dData;
use rav1d::include::dav1d::dav1d::{
    DAV1D_DECODEFRAMETYPE_ALL, DAV1D_INLOOPFILTER_ALL, Dav1dContext, Dav1dLogger, Dav1dSettings,
};
use rav1d::include::dav1d::headers::{DAV1D_PIXEL_LAYOUT_I400, DAV1D_PIXEL_LAYOUT_I420};
use rav1d::include::dav1d::picture::Dav1dPicture;
use rav1d::src::lib::{
    dav1d_close, dav1d_data_create, dav1d_data_unref, dav1d_default_settings, dav1d_get_picture, dav1d_open,
    dav1d_picture_unref, dav1d_send_data,
};

use crate::yuv::{self, Matrix, Siting};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub threads: u16,
    pub frame_delay: u16,
    pub max_pixels: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub pts_ns: i64,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub planes: Option<Vec<u8>>,
}

#[derive(Clone, Copy, Debug)]
pub struct Expect {
    pub width: u32,
    pub height: u32,
    pub mono: bool,
    pub matrix: Matrix,
    pub full_range: bool,
    pub siting: Siting,
}

pub struct Decoder {
    ctx: Option<Dav1dContext>,
    pending: Dav1dData,
    expect: Expect,
    keep_planes: bool,
    rgba: bool,
}

fn again(r: Dav1dResult) -> bool {
    r.0 == -libc::EAGAIN
}

impl Decoder {
    pub fn new(settings: &Settings, expect: Expect) -> Result<Self, String> {
        let mut s = MaybeUninit::<Dav1dSettings>::uninit();
        unsafe { dav1d_default_settings(NonNull::from(&mut s).cast()) };
        let mut s = unsafe { s.assume_init() };
        s.n_threads = c_int::from(settings.threads);
        s.max_frame_delay = c_int::from(settings.frame_delay);
        s.apply_grain = 0;
        s.operating_point = 0;
        s.all_layers = 0;
        s.frame_size_limit = settings.max_pixels;
        s.strict_std_compliance = 1;
        s.output_invisible_frames = 0;
        s.inloop_filters = DAV1D_INLOOPFILTER_ALL;
        s.decode_frame_type = DAV1D_DECODEFRAMETYPE_ALL;
        s.logger = unsafe { Dav1dLogger::new(None, None) };
        let mut ctx: Option<Dav1dContext> = None;
        let r = unsafe { dav1d_open(Some(NonNull::from(&mut ctx)), Some(NonNull::from(&mut s))) };
        if r.0 != 0 || ctx.is_none() {
            return Err(format!("the AV1 decoder did not start (error {})", r.0));
        }
        Ok(Self { ctx, pending: Dav1dData::default(), expect, keep_planes: false, rgba: true })
    }

    pub fn keep_planes(&mut self, keep: bool) {
        self.keep_planes = keep;
    }

    pub fn planes_only(&mut self) {
        (self.keep_planes, self.rgba) = (true, false);
    }

    pub fn send(&mut self, unit: &[u8], pts_ns: i64) -> Result<bool, String> {
        if self.pending.sz != 0 {
            return Err("a temporal unit is still pending".into());
        }
        if unit.is_empty() {
            return Err("an empty temporal unit".into());
        }
        let ptr = unsafe { dav1d_data_create(Some(NonNull::from(&mut self.pending)), unit.len()) };
        if ptr.is_null() {
            return Err(format!("no memory for a temporal unit of {} bytes", unit.len()));
        }
        unsafe { std::ptr::copy_nonoverlapping(unit.as_ptr(), ptr, unit.len()) };
        self.pending.m.timestamp = pts_ns;
        self.resend()
    }

    pub fn resend(&mut self) -> Result<bool, String> {
        if self.pending.sz == 0 {
            return Ok(true);
        }
        let r = unsafe { dav1d_send_data(self.ctx, Some(NonNull::from(&mut self.pending))) };
        if r.0 == 0 {
            return Ok(self.pending.sz == 0);
        }
        if again(r) {
            return Ok(false);
        }
        Err(format!("the AV1 stream is damaged (decoder error {})", r.0))
    }

    pub fn picture(&mut self) -> Result<Option<Frame>, String> {
        let mut pic = Dav1dPicture::default();
        let r = unsafe { dav1d_get_picture(self.ctx, Some(NonNull::from(&mut pic))) };
        if r.0 != 0 {
            release_default(&mut pic);
            if again(r) {
                return Ok(None);
            }
            return Err(format!("the AV1 stream is damaged (decoder error {})", r.0));
        }
        let out = self.convert(&pic);
        unsafe { dav1d_picture_unref(Some(NonNull::from(&mut pic))) };
        release_default(&mut pic);
        out.map(Some)
    }
}

fn release_default(pic: &mut Dav1dPicture) {
    pic.itut_t35 = None;
    if let Some(raw) = pic.itut_t35_ref.take() {
        drop(unsafe { raw.into_arc() });
    }
}

impl Decoder {
    fn convert(&self, pic: &Dav1dPicture) -> Result<Frame, String> {
        let e = &self.expect;
        let (w, h) = (pic.p.w, pic.p.h);
        if pic.p.bpc != 8 {
            return Err(format!("a {}-bit picture (a movie is 8-bit)", pic.p.bpc));
        }
        if i64::from(w) != i64::from(e.width) || i64::from(h) != i64::from(e.height) {
            return Err(format!(
                "a picture of {w} × {h} in a movie of {} × {} (a movie keeps its size)",
                e.width, e.height
            ));
        }
        let mono = match pic.p.layout {
            DAV1D_PIXEL_LAYOUT_I420 => false,
            DAV1D_PIXEL_LAYOUT_I400 => true,
            l => return Err(format!("a picture of chroma layout {l} (4:2:0 or monochrome)")),
        };
        if mono != e.mono {
            return Err("a picture whose chroma differs from its sequence header's".into());
        }
        let (w, h) = (e.width as usize, e.height as usize);
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        let (ys, cs) = (pic.stride[0], pic.stride[1]);
        let plane = |k: usize, stride: isize, rows: usize, cols: usize| -> Result<&[u8], String> {
            let ptr = pic.data[k].ok_or("a picture without its planes")?;
            let stride = usize::try_from(stride).ok().filter(|&s| s >= cols).ok_or("a picture plane's stride")?;
            let len = stride * (rows - 1) + cols;
            Ok(unsafe { std::slice::from_raw_parts(ptr.as_ptr().cast::<u8>().cast_const(), len) })
        };
        let y = plane(0, ys, h, w)?;
        let chroma = if mono { None } else { Some((plane(1, cs, ch, cw)?, plane(2, cs, ch, cw)?)) };
        let ystride = usize::try_from(ys).map_err(|_| "a picture plane's stride")?;
        let cstride = usize::try_from(cs).unwrap_or(0);
        let mut rgba = Vec::new();
        if self.rgba {
            rgba = vec![0u8; w * h * 4];
            let planes = yuv::Planes { y, ystride, chroma, cstride, width: w, height: h };
            yuv::to_rgba(&planes, e.matrix, e.full_range, e.siting, &mut rgba);
        }
        let planes = self.keep_planes.then(|| {
            let mut out = Vec::with_capacity(w * h + 2 * cw * ch);
            let rows = |p: &[u8], stride: usize, n: usize, len: usize, out: &mut Vec<u8>| {
                (0..n).for_each(|r| out.extend_from_slice(&p[r * stride..r * stride + len]))
            };
            rows(y, ystride, h, w, &mut out);
            if let Some((cb, cr)) = chroma {
                rows(cb, cstride, ch, cw, &mut out);
                rows(cr, cstride, ch, cw, &mut out);
            }
            out
        });
        Ok(Frame { pts_ns: pic.m.timestamp, width: e.width, height: e.height, rgba, planes })
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        if self.pending.sz != 0 {
            unsafe { dav1d_data_unref(Some(NonNull::from(&mut self.pending))) };
        }
        unsafe { dav1d_close(Some(NonNull::from(&mut self.ctx))) };
    }
}
