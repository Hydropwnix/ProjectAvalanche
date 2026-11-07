use std::collections::VecDeque;
use std::io::{Read, Seek};
use std::time::Duration;

use ssx_content::movie::{Demuxer, Limits, MovieInfo, Track};

use crate::av1::{self, Expect, Frame};
use crate::isolate::{Isolated, Out};
use crate::vorbis::VorbisStream;
use crate::yuv::{Matrix, Siting, packed_to_rgba};

pub const MAX_TURNS: u32 = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Threads {
    pub threads: u16,
    pub frame_delay: u16,
}

#[derive(Debug)]
pub enum Event {
    Picture(Frame),
    Audio(Vec<f32>),
}

pub fn video_settings(info: &MovieInfo, threads: Threads) -> Result<(av1::Settings, Expect), String> {
    let v = &info.video;
    let s = &v.config.sequence;
    let matrix = Matrix::of(s.color.matrix).ok_or_else(|| format!("matrix coefficients {}", s.color.matrix))?;
    let expect = Expect {
        width: v.width,
        height: v.height,
        mono: s.mono,
        matrix,
        full_range: s.color.full_range,
        siting: Siting::of(s.color.chroma_position),
    };
    let settings =
        av1::Settings { threads: threads.threads, frame_delay: threads.frame_delay, max_pixels: v.width * v.height };
    Ok((settings, expect))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Where {
    Here,
    Child,
}

enum Video {
    Here(av1::Decoder),
    Child(Isolated, Expect),
}

const CHILD_POLL: Duration = Duration::from_millis(50);

pub struct MovieDecoder<R> {
    demux: Demuxer<R>,
    video: Video,
    audio: Option<VorbisStream>,
    queue: VecDeque<Event>,
    sent: VecDeque<i64>,
    pub pictures: u64,
    finished: bool,
}

impl<R: Read + Seek> MovieDecoder<R> {
    pub fn open(inner: R, len: u64, limits: &Limits, threads: Threads, at: Where) -> Result<Self, String> {
        let demux = Demuxer::open(inner, len, limits).map_err(|e| e.to_string())?;
        let info = demux.info().clone();
        let (settings, expect) = video_settings(&info, threads)?;
        let video = match at {
            Where::Here => Video::Here(av1::Decoder::new(&settings, expect)?),
            Where::Child => Video::Child(Isolated::spawn(&settings, &expect)?, expect),
        };
        let audio = match &info.audio {
            Some(a) => Some(VorbisStream::new(a, info.duration_ns, limits.max_overrun_ns)?),
            None => None,
        };
        Ok(Self { demux, video, audio, queue: VecDeque::new(), sent: VecDeque::new(), pictures: 0, finished: false })
    }

    pub fn info(&self) -> &MovieInfo {
        self.demux.info()
    }

    pub fn keep_planes(&mut self, keep: bool) {
        if let Video::Here(v) = &mut self.video {
            v.keep_planes(keep);
        }
    }

    pub fn next_event(&mut self) -> Result<Option<Event>, String> {
        loop {
            if let Some(e) = self.queue.pop_front() {
                return Ok(Some(e));
            }
            if self.finished {
                return Ok(None);
            }
            match self.demux.next_packet().map_err(|e| e.to_string())? {
                Some(p) if p.track == Track::Video => {
                    self.sent.push_back(p.pts_ns);
                    self.send(p.data, p.pts_ns)?;
                }
                Some(p) => {
                    let mut pcm = Vec::new();
                    if let Some(a) = &mut self.audio {
                        a.decode(&p, &mut pcm)?;
                    }
                    if !pcm.is_empty() {
                        self.queue.push_back(Event::Audio(pcm));
                    }
                }
                None => {
                    self.drain()?;
                    if let Some(&t) = self.sent.front() {
                        return Err(format!("AV1: the temporal unit at {:.3} s gave no picture", t as f64 / 1e9));
                    }
                    self.finished = true;
                }
            }
        }
    }

    fn send(&mut self, unit: Vec<u8>, pts: i64) -> Result<(), String> {
        let mut pending = Some(unit);
        let mut turns = 0;
        while let Some(u) = pending.take() {
            turns += 1;
            if turns > MAX_TURNS {
                return Err("AV1: the decoder takes no more data and gives no picture".into());
            }
            pending = match &mut self.video {
                Video::Here(v) if turns == 1 => (!v.send(&u, pts)?).then_some(u),
                Video::Here(v) => (!v.resend()?).then_some(u),
                Video::Child(c, _) => c.push(pts, u)?,
            };
            if pending.is_some() {
                self.take(CHILD_POLL)?;
            }
        }
        self.take(Duration::ZERO)?;
        Ok(())
    }

    fn take(&mut self, wait: Duration) -> Result<Option<bool>, String> {
        match &mut self.video {
            Video::Here(v) => match v.picture()? {
                Some(f) => self.picture(f).map(|()| Some(true)),
                None => Ok(None),
            },
            Video::Child(c, e) => {
                let e = *e;
                match c.next(wait)? {
                    Some(Out::Picture(pts, planes)) => {
                        let (w, h) = (e.width as usize, e.height as usize);
                        let rgba = packed_to_rgba(&planes, w, h, e.mono, e.matrix, e.full_range, e.siting);
                        let f = Frame { pts_ns: pts, width: e.width, height: e.height, rgba, planes: None };
                        self.picture(f).map(|()| Some(true))
                    }
                    Some(Out::Done) => Ok(Some(false)),
                    None => Ok(None),
                }
            }
        }
    }

    fn drain(&mut self) -> Result<(), String> {
        if let Video::Child(c, _) = &mut self.video {
            c.finish();
            let mut idle = 0;
            loop {
                match self.take(CHILD_POLL)? {
                    Some(false) => return Ok(()),
                    Some(true) => idle = 0,
                    None => {
                        idle += 1;
                        if idle > MAX_TURNS {
                            return Err("AV1: the decoder's process gives no more pictures and no end".into());
                        }
                    }
                }
            }
        }
        while self.take(Duration::ZERO)? == Some(true) {}
        Ok(())
    }

    fn picture(&mut self, f: Frame) -> Result<(), String> {
        match self.sent.pop_front() {
            Some(t) if t == f.pts_ns => {}
            _ => {
                return Err(format!(
                    "AV1: a picture at {:.3} s that no temporal unit stands for",
                    f.pts_ns as f64 / 1e9
                ));
            }
        }
        self.pictures += 1;
        self.queue.push_back(Event::Picture(f));
        Ok(())
    }
}
