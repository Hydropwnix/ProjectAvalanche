use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::mpsc::{Sender, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::Pull;

#[derive(Clone, Copy, Debug, Default)]
pub struct Record {
    pub at: Duration,
    pub frames: usize,
    pub pulled: usize,
    pub pull: Pull,
    pub took: Duration,
    pub gap: Duration,
    pub rendered: u64,
    pub render: Duration,
    pub underruns: u64,
    pub starved: u64,
}

pub const COLUMNS: &str = "t_ms\tframes\tpulled\tlevel\ttarget\tsilence\tstartup\twait_ms\ttook_ms\tgap_ms\trendered\t\
                           render_ms\tunderruns\tstarved";

impl Record {
    fn line(&self) -> String {
        let ms = |d: Duration| d.as_secs_f64() * 1e3;
        format!(
            "{:.3}\t{}\t{}\t{}\t{}\t{}\t{}\t{:.3}\t{:.3}\t{:.3}\t{}\t{:.3}\t{}\t{}",
            ms(self.at),
            self.frames,
            self.pulled,
            self.pull.level,
            self.pull.target,
            self.pull.silence,
            u8::from(self.pull.startup),
            ms(self.pull.wait),
            ms(self.took),
            ms(self.gap),
            self.rendered,
            ms(self.render),
            self.underruns,
            self.starved
        )
    }
}

pub struct Capture {
    tx: Option<Sender<(Vec<f32>, Record)>>,
    thread: Option<JoinHandle<()>>,
}

fn header(rate: u32, channels: u16, data: u32) -> [u8; 44] {
    let mut h = [0u8; 44];
    let align = channels * 4;
    h[0..4].copy_from_slice(b"RIFF");
    h[4..8].copy_from_slice(&(36 + data).to_le_bytes());
    h[8..16].copy_from_slice(b"WAVEfmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes());
    h[20..22].copy_from_slice(&3u16.to_le_bytes());
    h[22..24].copy_from_slice(&channels.to_le_bytes());
    h[24..28].copy_from_slice(&rate.to_le_bytes());
    h[28..32].copy_from_slice(&(rate * u32::from(align)).to_le_bytes());
    h[32..34].copy_from_slice(&align.to_le_bytes());
    h[34..36].copy_from_slice(&32u16.to_le_bytes());
    h[36..40].copy_from_slice(b"data");
    h[40..44].copy_from_slice(&data.to_le_bytes());
    h
}

fn sync(wav: &mut BufWriter<File>, log: &mut BufWriter<File>, rate: u32, channels: u16, bytes: u64) {
    let patch = |wav: &mut BufWriter<File>| -> std::io::Result<()> {
        wav.flush()?;
        let f = wav.get_mut();
        f.seek(SeekFrom::Start(0))?;
        f.write_all(&header(rate, channels, bytes.min(u64::from(u32::MAX - 36)) as u32))?;
        f.seek(SeekFrom::End(0))?;
        Ok(())
    };
    if let Err(e) = patch(wav).and_then(|()| log.flush()) {
        eprintln!("warning: audio capture: {e}");
    }
}

impl Capture {
    pub fn create(path: &Path, rate: u32, channels: usize) -> std::io::Result<Self> {
        let channels = channels.clamp(1, usize::from(u16::MAX)) as u16;
        let mut wav = BufWriter::new(File::create(path)?);
        wav.write_all(&header(rate, channels, 0))?;
        let mut log_path = path.as_os_str().to_owned();
        log_path.push(".pulls.tsv");
        let mut log = BufWriter::new(File::create(&log_path)?);
        writeln!(log, "{COLUMNS}")?;
        let (tx, rx) = channel::<(Vec<f32>, Record)>();
        let thread = std::thread::Builder::new().name("ssx_audio_capture".into()).spawn(move || {
            let (mut bytes, mut synced) = (0u64, Instant::now());
            loop {
                match rx.recv_timeout(Duration::from_secs(1)) {
                    Ok((samples, rec)) => {
                        let mut raw = Vec::with_capacity(samples.len() * 4);
                        for s in &samples {
                            raw.extend_from_slice(&s.to_le_bytes());
                        }
                        bytes += raw.len() as u64;
                        if wav.write_all(&raw).and_then(|()| writeln!(log, "{}", rec.line())).is_err() {
                            break;
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }
                if synced.elapsed() >= Duration::from_secs(1) {
                    sync(&mut wav, &mut log, rate, channels, bytes);
                    synced = Instant::now();
                }
            }
            sync(&mut wav, &mut log, rate, channels, bytes);
        })?;
        Ok(Self { tx: Some(tx), thread: Some(thread) })
    }

    pub fn push(&self, samples: &[f32], rec: Record) {
        if let Some(tx) = &self.tx {
            let _ = tx.send((samples.to_vec(), rec));
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.tx = None;
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
