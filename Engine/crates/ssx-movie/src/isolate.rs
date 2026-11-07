use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use crate::av1::{self, Expect};
use crate::check::bounded_text;
use crate::isolate_os::{self, Process, Seal};
use crate::yuv::packed_len;

pub use crate::isolate_os::{MAX_THREADS, MEMORY_LIMIT};

pub const CHILD_ARG: &str = "--ssx-movie-decoder";
const MAGIC: &[u8; 8] = b"SSXAV1\x00\x01";
const UNITS_QUEUED: usize = 16;
const MAX_UNIT: u32 = 64 << 20;
const MAX_STDERR: u64 = 2048;
pub const HANG: Duration = Duration::from_secs(10);

pub fn is_child() -> bool {
    let mut args = std::env::args_os().skip(1);
    args.next().as_deref() == Some(std::ffi::OsStr::new(CHILD_ARG)) && args.next().is_none()
}

pub fn serve_if_child() {
    if is_child() {
        run(Seal::Enforce);
    }
}

#[doc(hidden)]
pub fn serve_if_measured_child() {
    if is_child() {
        run(Seal::Measure);
    }
}

fn run(seal: Seal) -> ! {
    let code = match enter_child(seal).and_then(ChildSession::serve) {
        Ok(()) => 0,
        Err(e) => {
            ssx_core::eprintln!("{e}");
            1
        }
    };
    std::process::exit(code);
}

fn read_exact(r: &mut impl Read, n: usize) -> Result<Vec<u8>, String> {
    let mut b = vec![0u8; n];
    r.read_exact(&mut b).map_err(|e| format!("the pipe: {e}"))?;
    Ok(b)
}

fn u32le(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

pub struct ChildSession {
    dec: av1::Decoder,
    mono: bool,
    r: BufReader<std::io::StdinLock<'static>>,
    w: BufWriter<std::io::StdoutLock<'static>>,
}

pub fn enter_child(seal: Seal) -> Result<ChildSession, String> {
    panic_report();
    isolate_os::prepare(seal).map_err(|e| format!("the movie decoder's sandbox: {e}"))?;
    let mut r = BufReader::new(std::io::stdin().lock());
    let w = BufWriter::with_capacity(1 << 20, std::io::stdout().lock());
    let (settings, expect) = hello(&mut r)?;
    if seal == Seal::Measure {
        isolate_os::measure_from_here().map_err(|e| format!("the movie decoder's sandbox: {e}"))?;
    }
    let mut dec = av1::Decoder::new(&settings, expect)?;
    dec.planes_only();
    isolate_os::seal(seal).map_err(|e| format!("the movie decoder's sandbox: {e}"))?;
    Ok(ChildSession { dec, mono: expect.mono, r, w })
}

fn panic_report() {
    std::panic::set_hook(Box::new(|info| {
        let place = info.location().map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column())).unwrap_or_default();
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("<unnamed>");
        ssx_core::eprintln!("thread '{name}' panicked at {place}: {}", info.payload_as_str().unwrap_or("(no message)"));
    }));
}

fn hello(r: &mut impl Read) -> Result<(av1::Settings, Expect), String> {
    let hello = read_exact(r, 8 + 4 + 4 + 4 + 4 + 4 + 1)?;
    if &hello[..8] != MAGIC {
        return Err("not the movie decoder's parent".into());
    }
    let threads = u32le(&hello[8..]);
    if threads == 0 || threads > u32::from(MAX_THREADS) {
        return Err(format!("{threads} decoder threads (1 to {MAX_THREADS})"));
    }
    let settings = av1::Settings {
        threads: threads as u16,
        frame_delay: u32le(&hello[12..]).min(64) as u16,
        max_pixels: u32le(&hello[16..]),
    };
    let expect = Expect {
        width: u32le(&hello[20..]),
        height: u32le(&hello[24..]),
        mono: hello[28] != 0,
        matrix: crate::yuv::Matrix::Bt709,
        full_range: false,
        siting: crate::yuv::Siting::Centre,
    };
    Ok((settings, expect))
}

impl ChildSession {
    pub fn read_unit(&mut self) -> Result<Option<(i64, Vec<u8>)>, String> {
        read_unit(&mut self.r)
    }

    pub fn serve(self) -> Result<(), String> {
        self.serve_from(None)
    }

    pub fn serve_from(mut self, first: Option<(i64, Vec<u8>)>) -> Result<(), String> {
        let mut first = first;
        let result = (|| -> Result<(), String> {
            loop {
                let next = match first.take() {
                    Some(u) => Some(u),
                    None => self.read_unit()?,
                };
                let Some((pts, unit)) = next else {
                    while let Some(f) = self.dec.picture()? {
                        out(&mut self.w, self.mono, f)?;
                    }
                    return Ok(());
                };
                let mut taken = self.dec.send(&unit, pts)?;
                let mut turns = 0;
                while !taken {
                    turns += 1;
                    if turns > crate::decode::MAX_TURNS {
                        return Err("AV1: the decoder takes no more data and gives no picture".into());
                    }
                    if let Some(f) = self.dec.picture()? {
                        out(&mut self.w, self.mono, f)?;
                    }
                    taken = self.dec.resend()?;
                }
                if let Some(f) = self.dec.picture()? {
                    out(&mut self.w, self.mono, f)?;
                }
                self.w.flush().map_err(|e| format!("the pipe: {e}"))?;
            }
        })();
        match result {
            Ok(()) => self.w.write_all(&[3]),
            Err(e) => {
                let text = e.as_bytes();
                let n = text.len().min(1024);
                let mut m = vec![2u8];
                m.extend((n as u16).to_le_bytes());
                m.extend(&text[..n]);
                self.w.write_all(&m)
            }
        }
        .and_then(|()| self.w.flush())
        .map_err(|e| format!("the pipe: {e}"))
    }
}

fn read_unit(r: &mut impl Read) -> Result<Option<(i64, Vec<u8>)>, String> {
    let tag = read_exact(r, 1)?[0];
    if tag == 2 {
        return Ok(None);
    }
    let head = read_exact(r, 12)?;
    let pts = i64::from_le_bytes(head[..8].try_into().map_err(|_| "a unit's head")?);
    let len = u32le(&head[8..]);
    if tag != 1 || len == 0 || len > MAX_UNIT {
        return Err(format!("a unit message of tag {tag} and {len} bytes"));
    }
    Ok(Some((pts, read_exact(r, len as usize)?)))
}

fn out(w: &mut impl Write, mono: bool, f: av1::Frame) -> Result<(), String> {
    let planes = f.planes.unwrap_or_default();
    let mut head = vec![1u8];
    head.extend(f.pts_ns.to_le_bytes());
    head.extend(f.width.to_le_bytes());
    head.extend(f.height.to_le_bytes());
    head.push(u8::from(mono));
    head.extend((planes.len() as u32).to_le_bytes());
    w.write_all(&head).and_then(|()| w.write_all(&planes)).map_err(|e| format!("the pipe: {e}"))
}

#[derive(Debug)]
pub enum Out {
    Picture(i64, Vec<u8>),
    Done,
}

pub struct Isolated {
    child: Arc<Mutex<Process>>,
    units: Option<SyncSender<(i64, Vec<u8>)>>,
    out: Receiver<Result<Out, String>>,
    stderr: Arc<Mutex<String>>,
    owed: u64,
    since: std::time::Instant,
}

fn threads(asked: u16) -> u16 {
    let n = if asked == 0 { std::thread::available_parallelism().map_or(1, |n| n.get()) } else { usize::from(asked) };
    n.clamp(1, usize::from(MAX_THREADS)) as u16
}

impl Isolated {
    pub fn spawn(settings: &av1::Settings, expect: &Expect) -> Result<Self, String> {
        let exe = std::env::current_exe().map_err(|e| format!("this program's path: {e}"))?;
        let isolate_os::Spawned { process, stdin, stdout, stderr: mut err } = isolate_os::spawn(&exe, CHILD_ARG)?;
        let child = Arc::new(Mutex::new(process));
        let said = Arc::new(Mutex::new(String::new()));
        let s = said.clone();
        let started = std::thread::Builder::new().name("ssx_movie_child_err".into()).spawn(move || {
            let mut b = Vec::new();
            let _ = Read::by_ref(&mut err).take(MAX_STDERR).read_to_end(&mut b);
            let _ = std::io::copy(&mut err, &mut std::io::sink());
            *s.lock().unwrap_or_else(PoisonError::into_inner) = String::from_utf8_lossy(&b).into_owned();
        });
        let mut hello = MAGIC.to_vec();
        for v in [
            u32::from(threads(settings.threads)),
            u32::from(settings.frame_delay),
            settings.max_pixels,
            expect.width,
            expect.height,
        ] {
            hello.extend(v.to_le_bytes());
        }
        hello.push(u8::from(expect.mono));
        let (units, rx) = sync_channel::<(i64, Vec<u8>)>(UNITS_QUEUED);
        let fed = started.and_then(|_| {
            std::thread::Builder::new().name("ssx_movie_child_in".into()).spawn(move || feed(stdin, &hello, &rx))
        });
        let (tx, out) = sync_channel(4);
        let want = packed_len(expect.width as usize, expect.height as usize, expect.mono);
        let (w, h, mono) = (expect.width, expect.height, expect.mono);
        let read = fed.and_then(|_| {
            std::thread::Builder::new().name("ssx_movie_child_out".into()).spawn(move || {
                let mut r = BufReader::with_capacity(1 << 20, stdout);
                loop {
                    let m = read_out(&mut r, want, w, h, mono);
                    let last = !matches!(m, Ok(Out::Picture(..)));
                    if tx.send(m).is_err() || last {
                        return;
                    }
                }
            })
        });
        let me = Isolated { child, units: Some(units), out, stderr: said, owed: 0, since: std::time::Instant::now() };
        read.map_err(|e| format!("no thread for the movie decoder: {e}"))?;
        Ok(me)
    }

    pub fn id(&self) -> u32 {
        self.child.lock().unwrap_or_else(PoisonError::into_inner).id()
    }

    pub fn push(&mut self, pts: i64, unit: Vec<u8>) -> Result<Option<Vec<u8>>, String> {
        let Some(u) = &self.units else { return Err("the stream was finished".into()) };
        match u.try_send((pts, unit)) {
            Ok(()) => {
                if self.owed == 0 {
                    self.since = std::time::Instant::now();
                }
                self.owed += 1;
                Ok(None)
            }
            Err(TrySendError::Full((_, unit))) => Ok(Some(unit)),
            Err(TrySendError::Disconnected(_)) => Err(self.death("its input closed")),
        }
    }

    pub fn finish(&mut self) {
        if self.units.take().is_some() {
            self.since = std::time::Instant::now();
        }
    }

    pub fn next(&mut self, wait: Duration) -> Result<Option<Out>, String> {
        match self.out.recv_timeout(wait) {
            Ok(Ok(o)) => {
                self.owed = self.owed.saturating_sub(u64::from(matches!(o, Out::Picture(..))));
                self.since = std::time::Instant::now();
                Ok(Some(o))
            }
            Ok(Err(e)) => Err(self.death(&e)),
            Err(RecvTimeoutError::Timeout) => {
                if (self.owed > 0 || self.units.is_none()) && self.since.elapsed() > HANG {
                    return Err(self.death(&format!("it gave nothing for {} s: hung", HANG.as_secs())));
                }
                Ok(None)
            }
            Err(RecvTimeoutError::Disconnected) => Err(self.death("its output closed")),
        }
    }

    fn death(&self, what: &str) -> String {
        let status = {
            let mut c = self.child.lock().unwrap_or_else(PoisonError::into_inner);
            let mut status = None;
            for _ in 0..50 {
                if let Ok(Some(s)) = c.try_wait() {
                    status = Some(s);
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            if status.is_none() {
                let _ = c.kill();
                status = c.wait().ok();
            }
            match status {
                Some(s) if s.by_sandbox() => format!("stopped by its sandbox: {s}"),
                Some(s) => s.to_string(),
                None => "unknown".into(),
            }
        };
        let mut said = String::new();
        for _ in 0..20 {
            said = self.stderr.lock().unwrap_or_else(PoisonError::into_inner).clone();
            if !said.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let said = bounded_text(said.trim());
        let said = if said.is_empty() { String::new() } else { format!("; it said: {said}") };
        format!("the movie decoder's process stopped ({what}; {status}){said}")
    }
}

impl Drop for Isolated {
    fn drop(&mut self) {
        let mut c = self.child.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = c.kill();
        let _ = c.wait();
    }
}

fn feed(stdin: File, hello: &[u8], units: &Receiver<(i64, Vec<u8>)>) {
    let mut w = BufWriter::with_capacity(1 << 20, stdin);
    if w.write_all(hello).and_then(|()| w.flush()).is_err() {
        return;
    }
    while let Ok((pts, unit)) = units.recv() {
        let mut head = vec![1u8];
        head.extend(pts.to_le_bytes());
        head.extend((unit.len() as u32).to_le_bytes());
        if w.write_all(&head).and_then(|()| w.write_all(&unit)).and_then(|()| w.flush()).is_err() {
            return;
        }
    }
    let _ = w.write_all(&[2]).and_then(|()| w.flush());
}

fn read_out(r: &mut impl Read, want: usize, w: u32, h: u32, mono: bool) -> Result<Out, String> {
    let tag = read_exact(r, 1)?[0];
    match tag {
        1 => {
            let head = read_exact(r, 8 + 4 + 4 + 1 + 4)?;
            let pts = i64::from_le_bytes(head[..8].try_into().map_err(|_| "a picture's head")?);
            let (pw, ph, pm, len) = (u32le(&head[8..]), u32le(&head[12..]), head[16] != 0, u32le(&head[17..]));
            if (pw, ph, pm) != (w, h, mono) || len as usize != want {
                return Err(format!("a picture of {pw} × {ph} ({len} bytes) in a stream of {w} × {h}"));
            }
            Ok(Out::Picture(pts, read_exact(r, want)?))
        }
        2 => {
            let n = u16::from_le_bytes(read_exact(r, 2)?.try_into().map_err(|_| "an error's length")?);
            let text = read_exact(r, usize::from(n).min(1024))?;
            Err(bounded_text(&String::from_utf8_lossy(&text)))
        }
        3 => Ok(Out::Done),
        t => Err(format!("a message of tag {t}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(pts: i64, w: u32, h: u32, mono: bool, len: u32, body: usize) -> Vec<u8> {
        let mut m = vec![1u8];
        m.extend(pts.to_le_bytes());
        m.extend(w.to_le_bytes());
        m.extend(h.to_le_bytes());
        m.push(u8::from(mono));
        m.extend(len.to_le_bytes());
        m.extend(vec![7u8; body]);
        m
    }

    #[test]
    fn the_parent_holds_the_child_to_the_stream() {
        let want = packed_len(16, 16, false);
        let ok = picture(5, 16, 16, false, want as u32, want);
        assert!(matches!(read_out(&mut ok.as_slice(), want, 16, 16, false), Ok(Out::Picture(5, p)) if p.len() == want));
        let lie = picture(5, 16, 16, false, u32::MAX, 0);
        assert!(read_out(&mut lie.as_slice(), want, 16, 16, false).unwrap_err().contains("4294967295 bytes"));
        let other = picture(5, 32, 16, false, (2 * want) as u32, 0);
        assert!(read_out(&mut other.as_slice(), want, 16, 16, false).is_err());
        let cut = &ok[..ok.len() - 1];
        assert!(read_out(&mut &cut[..], want, 16, 16, false).unwrap_err().contains("the pipe"));
        let mut long = vec![2u8];
        long.extend(u16::MAX.to_le_bytes());
        long.extend(b"\x1b[2J");
        long.extend(vec![b'x'; 70_000]);
        let e = read_out(&mut long.as_slice(), want, 16, 16, false).unwrap_err();
        assert!(e.starts_with("\\u{1b}[2J") && e.ends_with("… (1024 bytes)"), "{e}");
        assert_eq!(e.chars().count(), 256 + "… (1024 bytes)".chars().count(), "{e}");
        assert!(read_out(&mut [9u8].as_slice(), want, 16, 16, false).unwrap_err().contains("tag 9"));
    }

    #[test]
    fn the_child_reads_only_its_parent() {
        assert!(hello(&mut &b"not the parent's hello, 29 by"[..]).unwrap_err().contains("not the movie"));
        let hello_of = |threads: u32| {
            let mut h = MAGIC.to_vec();
            for v in [threads, 1, 64 * 64, 64, 64] {
                h.extend(v.to_le_bytes());
            }
            h.push(0);
            h
        };
        assert!(hello(&mut hello_of(1).as_slice()).is_ok());
        assert!(hello(&mut hello_of(0).as_slice()).unwrap_err().contains("0 decoder threads"));
        let many = u32::from(MAX_THREADS) + 1;
        assert!(hello(&mut hello_of(many).as_slice()).unwrap_err().contains("decoder threads"));
        let mut bad = vec![1u8];
        bad.extend(0i64.to_le_bytes());
        bad.extend((MAX_UNIT + 1).to_le_bytes());
        assert!(read_unit(&mut bad.as_slice()).unwrap_err().contains("bytes"));
        assert!(read_unit(&mut [2u8].as_slice()).unwrap().is_none());
    }

    #[test]
    fn the_parent_names_the_threads() {
        assert!((1..=MAX_THREADS).contains(&threads(0)));
        assert_eq!(threads(3), 3.min(MAX_THREADS));
        assert_eq!(threads(u16::MAX), MAX_THREADS);
    }
}
