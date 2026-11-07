use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use ssx_content::hash::hash_file;
use ssx_content::{ContentId, Kind};
use ssx_formats::ea_movie::{Movie, VideoHeader, decode_audio};
use ssx_formats::vp6::Decoder;

use super::DiscMovies;
use super::ffmpeg::{Encode, check_ffmpeg, encode};
use crate::content::export::{ExportRow, record_exports};

pub fn export_to(big_root: &Path, id: &ContentId, out_dir: &Path, app_dir: &Path) -> Result<Vec<PathBuf>> {
    if id.kind() != Kind::Movie {
        bail!("{id}: not a movie");
    }
    let disc = DiscMovies::index(big_root)?;
    let path =
        disc.files.get(id).with_context(|| format!("{id}: no such movie on the disc (`content ids --kind movie`)"))?;
    check_ffmpeg()?;
    let data = std::sync::Arc::new(std::fs::read(path).with_context(|| format!("reading {}", path.display()))?);
    let movie = Movie::parse(&data).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
    let v = VideoHeader::parse(&data[8..32]).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
    let stream = movie.audio_stream(None).context("the movie has no sound stream")?;
    let audio = decode_audio(stream).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
    let (layout, order): (&str, &[usize]) = match audio.channels {
        6 => ("5.1", &[0, 2, 1, 5, 3, 4]),
        2 => ("stereo", &[0, 1]),
        1 => ("mono", &[0]),
        n => bail!("{id}: {n} sound channels"),
    };
    std::fs::create_dir_all(out_dir).with_context(|| format!("creating {}", out_dir.display()))?;
    let name = id.path().rsplit('/').next().unwrap_or("movie");
    let out = out_dir.join(format!("{name}.webm"));
    let pcm_file = out_dir.join(format!(".{name}.export-sound.f32"));
    let written = (|| -> std::io::Result<()> {
        let mut f = std::io::BufWriter::new(std::fs::File::create(&pcm_file)?);
        for frame in audio.samples.chunks_exact(audio.channels) {
            for &c in order {
                f.write_all(&frame[c].to_le_bytes())?;
            }
        }
        f.flush()
    })();
    if let Err(e) = written {
        std::fs::remove_file(&pcm_file).ok();
        return Err(anyhow::Error::new(e).context(format!("writing {}", pcm_file.display())));
    }
    let fps = (v.rate, v.scale);
    let seconds = f64::from(v.frame_count) / v.fps();
    let enc = Encode {
        width: u32::from(v.width),
        height: u32::from(v.height),
        fps,
        pix_fmt: "yuv420p",
        audio: Some((&pcm_file, audio.sample_rate, audio.channels as u16, layout)),
        timeout: Duration::from_secs_f64(60.0 + 20.0 * seconds),
    };
    let base = data.as_ptr() as usize;
    let ranges: Vec<(usize, usize)> =
        movie.frames.iter().map(|f| (f.data.as_ptr() as usize - base, f.data.len())).collect();
    let file = data.clone();
    let mut dec = Decoder::new();
    let result = encode(&out, &enc, move |k, buf| {
        let Some(&(at, len)) = ranges.get(k as usize) else { return false };
        if dec.decode_frame(&file[at..at + len]).is_err() {
            return false;
        }
        let Some(pic) = dec.picture() else { return false };
        buf.clear();
        pic.write_i420(buf);
        true
    });
    std::fs::remove_file(&pcm_file).ok();
    result?;
    let (size, hash) = hash_file(&out).with_context(|| format!("hashing {}", out.display()))?;
    record_exports(app_dir, &[ExportRow { hash, size, id: id.to_string(), file: out.display().to_string() }])?;
    Ok(vec![out])
}
