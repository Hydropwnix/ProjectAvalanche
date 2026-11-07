use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

pub const VIDEO_ARGS: [&str; 22] = [
    "-c:v",
    "libaom-av1",
    "-cpu-used",
    "6",
    "-row-mt",
    "1",
    "-tiles",
    "2x2",
    "-crf",
    "30",
    "-b:v",
    "0",
    "-g",
    "150",
    "-pix_fmt",
    "yuv420p",
    "-colorspace",
    "bt709",
    "-color_primaries",
    "bt709",
    "-color_trc",
    "bt709",
];
pub const AUDIO_ARGS: [&str; 4] = ["-c:a", "libvorbis", "-q:a", "5"];
const MAX_MESSAGE: usize = 4096;

pub struct Encode<'a> {
    pub width: u32,
    pub height: u32,
    pub fps: (u32, u32),
    pub pix_fmt: &'static str,
    pub audio: Option<(&'a Path, u32, u16, &'static str)>,
    pub timeout: Duration,
}

fn file_arg(p: &Path) -> Result<String> {
    let abs = std::path::absolute(p).with_context(|| format!("{}", p.display()))?;
    let s = abs.to_str().with_context(|| format!("{}: not a UTF-8 path", abs.display()))?;
    Ok(format!("file:{s}"))
}

pub fn check_ffmpeg() -> Result<()> {
    let mut child = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-encoders"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("FFmpeg is not on the path (the movie tools run `ffmpeg` with libaom-av1 and libvorbis)")?;
    let mut out = String::new();
    let mut stdout = child.stdout.take().context("ffmpeg's output")?;
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stdout.by_ref().take(1 << 20).read_to_string(&mut s);
        s
    });
    wait(&mut child, Duration::from_secs(20))?;
    out.push_str(&reader.join().unwrap_or_default());
    for enc in ["libaom-av1", "libvorbis"] {
        if !out.split_whitespace().any(|w| w == enc) {
            bail!("this FFmpeg has no {enc} encoder (the movie tools need libaom-av1 and libvorbis)");
        }
    }
    Ok(())
}

fn wait(child: &mut std::process::Child, limit: Duration) -> Result<std::process::ExitStatus> {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if start.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            bail!("FFmpeg took longer than {:.0} s and was stopped", limit.as_secs_f64());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

pub fn encode(
    out: &Path,
    e: &Encode,
    mut picture: impl FnMut(u64, &mut Vec<u8>) -> bool + Send + 'static,
) -> Result<()> {
    let (w, h) = (e.width.to_string(), e.height.to_string());
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-hide_banner", "-loglevel", "error", "-y", "-f", "rawvideo", "-pix_fmt", e.pix_fmt]);
    cmd.args(["-s", &format!("{w}x{h}"), "-r", &format!("{}/{}", e.fps.0, e.fps.1), "-i", "pipe:0"]);
    if let Some((file, rate, channels, layout)) = e.audio {
        cmd.args(["-f", "f32le", "-ar", &rate.to_string(), "-ac", &channels.to_string(), "-ch_layout", layout]);
        cmd.args(["-i", &file_arg(file)?, "-map", "0:v", "-map", "1:a"]);
        cmd.args(AUDIO_ARGS);
    }
    cmd.args(VIDEO_ARGS).args(["-color_range", "tv", "-f", "webm", &file_arg(out)?]);
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("FFmpeg is not on the path (the movie tools run `ffmpeg` with libaom-av1 and libvorbis)")?;
    let mut stdin = child.stdin.take().context("ffmpeg's input")?;
    let mut stderr = child.stderr.take().context("ffmpeg's messages")?;
    let messages = std::thread::spawn(move || {
        let mut s = Vec::new();
        let _ = stderr.by_ref().take(MAX_MESSAGE as u64).read_to_end(&mut s);
        let _ = std::io::copy(&mut stderr, &mut std::io::sink());
        String::from_utf8_lossy(&s)
            .chars()
            .map(|c| if c.is_control() && c != '\n' { ' ' } else { c })
            .collect::<String>()
    });
    let writer = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let mut k = 0;
        while picture(k, &mut buf) {
            if stdin.write_all(&buf).is_err() {
                return;
            }
            k += 1;
        }
    });
    let status = wait(&mut child, e.timeout);
    let _ = writer.join();
    let said = messages.join().unwrap_or_default();
    let status = status?;
    if !status.success() {
        bail!("FFmpeg failed ({status}): {}", said.trim());
    }
    Ok(())
}
