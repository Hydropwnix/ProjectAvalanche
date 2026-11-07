use std::io::{Read, Seek};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use ssx_content::movie::{Limits, MovieInfo};

use crate::decode::{Event, MovieDecoder, Threads, Where};

#[derive(Clone, Debug)]
pub struct Report {
    pub info: MovieInfo,
    pub pictures: u64,
    pub audio_frames: u64,
    pub seconds: f64,
    pub frame_ms: Vec<f32>,
}

impl Report {
    pub fn fps(&self) -> f64 {
        self.pictures as f64 / self.seconds.max(1e-9)
    }

    pub fn speed(&self) -> f64 {
        self.info.duration_seconds() / self.seconds.max(1e-9)
    }

    pub fn frame_ms_quantile(&self, q: f64) -> f32 {
        let mut v = self.frame_ms.clone();
        v.sort_by(f32::total_cmp);
        v.get(((v.len().saturating_sub(1)) as f64 * q).round() as usize).copied().unwrap_or(0.0)
    }
}

pub fn budget(duration_s: f64) -> Duration {
    Duration::from_secs_f64(2.0 * duration_s.clamp(0.0, 3_600.0) + 30.0)
}

pub fn full_decode<R, F>(open: F, limits: &Limits, threads: Threads, budget: Duration) -> Result<Report, String>
where
    R: Read + Seek,
    F: FnOnce() -> Result<(R, u64), String> + Send + 'static,
{
    let limits = *limits;
    let (tx, rx) = mpsc::sync_channel(1);
    let spawned = std::thread::Builder::new().name("ssx_movie_check".into()).spawn(move || {
        let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| decode_all(open, &limits, threads)));
        let result = run.unwrap_or_else(|p| Err(format!("the decoder stopped: {}", panic_text(&p))));
        let _ = tx.send(result);
    });
    spawned.map_err(|e| format!("no worker thread for the decode: {e}"))?;
    match rx.recv_timeout(budget) {
        Ok(r) => r,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            Err(format!("the decode took longer than {:.0} s", budget.as_secs_f64()))
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => Err("the decode's worker ended without a result".into()),
    }
}

fn decode_all<R: Read + Seek, F: FnOnce() -> Result<(R, u64), String>>(
    open: F,
    limits: &Limits,
    threads: Threads,
) -> Result<Report, String> {
    let (inner, len) = open()?;
    let start = Instant::now();
    let mut d = MovieDecoder::open(inner, len, limits, threads, Where::Child)?;
    let info = d.info().clone();
    let (mut audio_frames, mut frame_ms, mut last) = (0u64, Vec::new(), Instant::now());
    let channels = info.audio.as_ref().map_or(1, |a| u64::from(a.channels));
    while let Some(e) = d.next_event()? {
        match e {
            Event::Picture(_) => {
                frame_ms.push(last.elapsed().as_secs_f32() * 1000.0);
                last = Instant::now();
            }
            Event::Audio(pcm) => audio_frames += pcm.len() as u64 / channels,
        }
    }
    Ok(Report { info, pictures: d.pictures, audio_frames, seconds: start.elapsed().as_secs_f64(), frame_ms })
}

pub fn panic_text(p: &(dyn std::any::Any + Send)) -> String {
    let s = p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned());
    bounded_text(&s.unwrap_or_else(|| "a panic without a message".into()))
}

pub fn bounded_text(s: &str) -> String {
    ssx_core::text::shown_max(s, 256).to_string()
}
