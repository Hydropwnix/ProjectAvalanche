use std::path::Path;
use std::time::{Duration, Instant};

use ssx_content::movie::av1::OBU_TEMPORAL_DELIMITER;
use ssx_movie::av1::{Expect, Settings};
use ssx_movie::isolate::{Isolated, Out};
use ssx_movie::yuv::{Matrix, Siting};

#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
#[path = "sandbox/linux.rs"]
mod platform;
#[cfg(windows)]
#[path = "sandbox/windows.rs"]
mod platform;
#[cfg(not(any(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"), windows)))]
mod platform {
    pub fn run(rec: &mut super::Record) {
        rec.fail("the movie decoder has no sandbox on this system (Linux x86-64 and Windows have one)".into());
    }
}

const PROGRAM: &str = env!("CARGO_BIN_EXE_avalanche-movie");
const PROBE: &str = env!("CARGO_BIN_EXE_avalanche-movie-probe");

const ATTEMPT: &[u8] = b"SSX-SANDBOX-ATTEMPT\0";

const TINY: &[u8] = &[
    0x12, 0x00, 0x0a, 0x0d, 0x00, 0x00, 0x00, 0x02, 0xaf, 0xff, 0x9b, 0x5f, 0x22, 0x04, 0x04, 0x02, 0x08, 0x32, 0xbd,
    0x01, 0x14, 0x00, 0x3d, 0xe0, 0x34, 0xd4, 0xa1, 0x8b, 0x2e, 0xba, 0xd9, 0x6e, 0x95, 0xb5, 0x07, 0x57, 0x93, 0x16,
    0x1d, 0xfd, 0x8d, 0x0f, 0x81, 0xb9, 0xb9, 0xd9, 0xe3, 0x3b, 0xd2, 0xb9, 0x66, 0xbc, 0x24, 0xdc, 0x38, 0xf1, 0xe7,
    0x5a, 0x5e, 0x3d, 0x05, 0xf9, 0x51, 0x4e, 0x34, 0xe9, 0x76, 0x5e, 0x41, 0x4e, 0xd3, 0xf9, 0xe2, 0xb7, 0x67, 0x1b,
    0x65, 0x3d, 0xe2, 0x4b, 0x73, 0xb6, 0xf9, 0x3e, 0x4c, 0x15, 0x64, 0x12, 0xbb, 0x64, 0x9c, 0xfe, 0xe0, 0xc9, 0xa4,
    0x4d, 0xbe, 0x61, 0xf1, 0x76, 0x37, 0x23, 0x98, 0x79, 0xac, 0x6f, 0x31, 0xcd, 0xdf, 0x81, 0x62, 0x19, 0x98, 0x88,
    0x52, 0x2e, 0xe2, 0x13, 0xf0, 0x1a, 0x2f, 0xc8, 0x78, 0x1c, 0x82, 0xde, 0x10, 0xcb, 0x71, 0xf9, 0xaf, 0x7e, 0xc1,
    0x7a, 0x76, 0x6c, 0x33, 0xa4, 0x5e, 0xda, 0x11, 0x18, 0x74, 0x6d, 0x97, 0xee, 0xac, 0xdd, 0xd4, 0x55, 0xb8, 0xbf,
    0x40, 0x45, 0x6c, 0x9e, 0x8e, 0xd3, 0xf7, 0x24, 0xe9, 0x19, 0xfa, 0x66, 0xb6, 0xf9, 0x86, 0x20, 0x38, 0xfa, 0x9e,
    0xe9, 0x3e, 0xb8, 0x17, 0x43, 0x92, 0x02, 0xd1, 0x6d, 0xe0, 0x6c, 0x85, 0x88, 0x77, 0xbd, 0xbb, 0x55, 0x3c, 0x46,
    0x1c, 0x90, 0x4c, 0xd0, 0xb9, 0x7e, 0x46, 0x80, 0xfd, 0x8c, 0xfd, 0x62, 0xf6, 0xd3, 0x84, 0x99, 0xac, 0xe5, 0x88,
    0x12, 0x00, 0x32, 0x11, 0x32, 0x01, 0xe0, 0x40, 0x00, 0x00, 0x03, 0x7e, 0xc0, 0x00, 0x01, 0x60, 0x00, 0x10, 0x00,
    0x99, 0x10, 0x12, 0x00, 0x32, 0x11, 0x32, 0x02, 0xe0, 0x82, 0x00, 0x00, 0x03, 0x7f, 0xc0, 0x00, 0x01, 0x60, 0x00,
    0x10, 0x00, 0x9c, 0xf0,
];
const TINY_PICTURES: usize = 3;

fn main() {
    let mut args = libtest_mimic::Arguments::from_args();
    args.test_threads = Some(1);
    let trial = libtest_mimic::Trial::test("sandbox", || {
        let mut rec = Record { failed: Vec::new() };
        platform::run(&mut rec);
        if rec.failed.is_empty() {
            println!("sandbox: every check passed");
            Ok(())
        } else {
            Err(rec.failed.iter().map(|f| format!("FAILED: {f}")).collect::<Vec<_>>().join("\n").into())
        }
    });
    libtest_mimic::run(&args, vec![trial]).exit();
}

pub struct Record {
    failed: Vec<String>,
}

impl Record {
    pub fn check(&mut self, what: String, ok: bool) {
        if ok {
            println!("holds    {what}");
        } else {
            self.fail(what);
        }
    }

    pub fn fail(&mut self, what: String) {
        println!("FAILS    {what}");
        self.failed.push(what);
    }

    pub fn info(&self, what: &str) {
        println!("info     {what}");
    }

    pub fn stopped(&self, what: &str) {
        println!("stopped  {what}");
    }
}

fn tiny(prompt: bool) -> (Settings, Expect) {
    let expect =
        Expect { width: 64, height: 64, mono: false, matrix: Matrix::Bt709, full_range: false, siting: Siting::Centre };
    (Settings { threads: 4, frame_delay: u16::from(prompt), max_pixels: 64 * 64 }, expect)
}

fn tiny_units() -> Vec<Vec<u8>> {
    let obus = ssx_content::movie::av1::obus(TINY, 64).unwrap_or_default();
    let mut cuts: Vec<usize> = obus.iter().filter(|o| o.kind == OBU_TEMPORAL_DELIMITER).map(|o| o.start).collect();
    cuts.push(TINY.len());
    cuts.windows(2).map(|w| TINY[w[0]..w[1]].to_vec()).collect()
}

pub type Units = Vec<(i64, Vec<u8>)>;

pub fn running(program: &Path) -> Result<(Isolated, Units, usize), String> {
    let (settings, expect) = tiny(true);
    let mut iso = Isolated::spawn_program(program, None, &settings, &expect)?;
    let mut units = tiny_units().into_iter().enumerate().map(|(k, u)| (k as i64, u));
    let start = Instant::now();
    loop {
        if let Some((pts, u)) = units.next()
            && iso.push(pts, u)?.is_some()
        {
            return Err("the child's queue is full before its first picture".into());
        }
        match iso.next(Duration::from_millis(100))? {
            Some(Out::Picture(..)) => return Ok((iso, units.collect(), 1)),
            Some(Out::Done) => return Err("the child ended before the stream did".into()),
            None if start.elapsed() > Duration::from_secs(30) => return Err("no picture within 30 s".into()),
            None => {}
        }
    }
}

pub fn finish(iso: &mut Isolated, rest: Units, mut pictures: usize) -> Result<usize, String> {
    for (pts, u) in rest {
        if iso.push(pts, u)?.is_some() {
            return Err("the child's queue is full".into());
        }
    }
    iso.finish();
    let start = Instant::now();
    loop {
        match iso.next(Duration::from_millis(100))? {
            Some(Out::Picture(..)) => pictures += 1,
            Some(Out::Done) => return Ok(pictures),
            None if start.elapsed() > Duration::from_secs(30) => return Err("no end within 30 s".into()),
            None => {}
        }
    }
}

pub fn attempt(name: &str, arg: &str) -> Result<String, String> {
    let (settings, expect) = tiny(false);
    let mut iso = Isolated::spawn_program(Path::new(PROBE), None, &settings, &expect)?;
    let mut unit = ATTEMPT.to_vec();
    unit.extend(name.as_bytes());
    unit.push(0);
    unit.extend(arg.as_bytes());
    if iso.push(0, unit)?.is_some() {
        return Err("the child's queue is full before its first unit".into());
    }
    iso.finish();
    let start = Instant::now();
    loop {
        match iso.next(Duration::from_millis(100)) {
            Err(reason) => return Ok(reason),
            Ok(Some(Out::Picture(..))) => return Err("the child gave a picture".into()),
            Ok(Some(Out::Done)) => return Err("the child ended its stream normally".into()),
            Ok(None) if start.elapsed() > Duration::from_secs(30) => return Err("no end within 30 s".into()),
            Ok(None) => {}
        }
    }
}

pub fn expect_end(rec: &mut Record, name: &str, arg: &str, want: &[&str], not: &[&str]) {
    expect_end_either(rec, name, arg, want, &[], not);
}

pub fn expect_end_either(rec: &mut Record, name: &str, arg: &str, want: &[&str], either: &[&str], not: &[&str]) {
    match attempt(name, arg) {
        Ok(reason) => {
            let missing: Vec<_> = want.iter().filter(|w| !reason.contains(*w)).collect();
            let one_of = either.is_empty() || either.iter().any(|w| reason.contains(*w));
            let present: Vec<_> = not.iter().filter(|w| reason.contains(*w)).collect();
            if missing.is_empty() && one_of && present.is_empty() && !reason.contains("NOT STOPPED") {
                rec.stopped(&format!("{name}: {reason}"));
            } else {
                rec.fail(format!("{name}: {reason} (wanted {want:?}, one of {either:?}, not {not:?})"));
            }
        }
        Err(e) => rec.fail(format!("{name}: {e}")),
    }
}
