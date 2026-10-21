use std::path::Path;

use anyhow::{Context, Result, bail};

pub struct Wav {
    pub rate: u32,
    pub channels: usize,
    pub samples: Vec<f32>,
}

pub fn read_wav(path: &Path) -> Result<Wav> {
    let b = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    if b.len() < 44 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        bail!("{}: not a RIFF WAVE file", path.display());
    }
    let u16_at = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]);
    let u32_at = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    let (mut at, mut fmt, mut data) = (12, None, None);
    while at + 8 <= b.len() {
        let (id, size) = (&b[at..at + 4], u32_at(at + 4) as usize);
        if id == b"fmt " && at + 24 <= b.len() {
            fmt = Some((u16_at(at + 8), u16_at(at + 10), u32_at(at + 12), u16_at(at + 22)));
        } else if id == b"data" {
            data = Some(at + 8..b.len());
            break;
        }
        at += 8 + size + (size & 1);
    }
    let (format, channels, rate, bits) = fmt.context("no fmt chunk")?;
    let raw = &b[data.context("no data chunk")?];
    let samples: Vec<f32> = match (format, bits) {
        (3, 32) => raw.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect(),
        (1, 16) => raw.as_chunks::<2>().0.iter().map(|c| f32::from(i16::from_le_bytes(*c)) / 32768.0).collect(),
        _ => bail!("{}: format {format} with {bits} bits is not read", path.display()),
    };
    let channels = usize::from(channels.max(1));
    let whole = samples.len() / channels * channels;
    Ok(Wav { rate, channels, samples: samples[..whole].to_vec() })
}

pub struct PullRow {
    pub at_ms: f64,
    pub frames: usize,
    pub pulled: usize,
    pub level: usize,
    pub target: usize,
    pub silence: usize,
    pub startup: bool,
    pub wait_ms: f64,
    pub took_ms: f64,
    pub gap_ms: f64,
    pub underruns: u64,
    pub starved: u64,
}

pub fn read_pulls(path: &Path) -> Option<Vec<PullRow>> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next()?.split('\t').collect();
    let col = |name: &str| header.iter().position(|h| *h == name);
    let idx = [
        col("t_ms")?,
        col("frames")?,
        col("level")?,
        col("target")?,
        col("silence")?,
        col("startup")?,
        col("wait_ms")?,
        col("took_ms")?,
        col("gap_ms")?,
        col("underruns")?,
        col("starved")?,
        col("pulled")?,
    ];
    let mut rows = Vec::new();
    for l in lines {
        let f: Vec<&str> = l.split('\t').collect();
        let num = |k: usize| f.get(idx[k]).and_then(|v| v.parse::<f64>().ok());
        let Some(v) = (0..idx.len()).map(num).collect::<Option<Vec<f64>>>() else { continue };
        rows.push(PullRow {
            at_ms: v[0],
            frames: v[1] as usize,
            pulled: v[11] as usize,
            level: v[2] as usize,
            target: v[3] as usize,
            silence: v[4] as usize,
            startup: v[5] != 0.0,
            wait_ms: v[6],
            took_ms: v[7],
            gap_ms: v[8],
            underruns: v[9] as u64,
            starved: v[10] as u64,
        });
    }
    Some(rows)
}

pub fn silent(w: &Wav, frame: usize) -> bool {
    w.samples[frame * w.channels..(frame + 1) * w.channels].iter().all(|x| *x == 0.0)
}

pub fn level(w: &Wav, frame: usize) -> f32 {
    w.samples[frame * w.channels..(frame + 1) * w.channels].iter().fold(0.0f32, |m, x| m.max(x.abs()))
}

pub fn dropouts(w: &Wav, min: usize, edge: f32) -> Vec<(usize, usize)> {
    let frames = w.samples.len() / w.channels;
    let (mut out, mut f) = (Vec::new(), 1);
    while f < frames {
        if !silent(w, f) {
            f += 1;
            continue;
        }
        let start = f;
        while f < frames && silent(w, f) {
            f += 1;
        }
        if f - start >= min && f < frames && level(w, start - 1) > edge && level(w, f) > edge {
            out.push((start, f - start));
        }
    }
    out
}

pub fn step(w: &Wav, frame: usize) -> f32 {
    let c = w.channels;
    (0..c).map(|k| (w.samples[frame * c + k] - w.samples[(frame - 1) * c + k]).abs()).fold(0.0, f32::max)
}

pub fn step_stats(mut v: Vec<f32>) -> (f32, f32, f32) {
    if v.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    v.sort_by(f32::total_cmp);
    let mean = v.iter().sum::<f32>() / v.len() as f32;
    (mean, v[((v.len() - 1) as f64 * 0.999) as usize], v[v.len() - 1])
}

pub struct Compared {
    pub frames: usize,
    pub gaps: Vec<(usize, usize)>,
    pub mismatch: Option<usize>,
}

pub fn compare(cap: &Wav, reference: &Wav) -> Result<Compared> {
    if cap.rate != reference.rate || cap.channels != reference.channels {
        bail!(
            "the reference is {} Hz × {}, the capture {} Hz × {}",
            reference.rate,
            reference.channels,
            cap.rate,
            cap.channels
        );
    }
    let c = cap.channels;
    let (cf, rf) = (cap.samples.len() / c, reference.samples.len() / c);
    let tol = 1.5 / 32768.0;
    let same =
        |a: usize, b: usize| (0..c).all(|k| (cap.samples[a * c + k] - reference.samples[b * c + k]).abs() <= tol);
    let first = (0..cf).find(|&f| level(cap, f) > 1e-3).context("the capture is silent")?;
    let probe = 2048.min(cf - first);
    let start = (0..rf.saturating_sub(probe))
        .find(|&r| (0..probe).all(|i| same(first + i, r + i)))
        .context("the capture's start is not in the reference")?;
    let (mut a, mut b, mut gaps) = (first, start, Vec::new());
    while a < cf && b < rf {
        if same(a, b) {
            (a, b) = (a + 1, b + 1);
            continue;
        }
        let mut end = a;
        while end < cf && silent(cap, end) {
            end += 1;
        }
        let agree = |from: usize| (0..256.min(cf - from).min(rf - b)).all(|i| same(from + i, b + i));
        if end > a && end < cf && agree(end) {
            gaps.push((a, end - a));
            a = end;
        } else {
            return Ok(Compared { frames: a - first, gaps, mismatch: Some(a) });
        }
    }
    Ok(Compared { frames: a - first, gaps, mismatch: None })
}
