use std::io::{BufReader, BufWriter, Read, Write};

use super::{MAGIC, MAX_THREADS, MAX_UNIT, read_exact, u32le};
use crate::av1::{self, Expect};
use crate::isolate_os::{self, Seal};

pub fn serve() -> ! {
    if std::env::args_os().len() > 1 {
        ssx_core::eprintln!(
            "avalanche-movie is Project Avalanche's movie decoder: the game starts it, without arguments"
        );
        std::process::exit(2);
    }
    run(Seal::Enforce)
}

#[doc(hidden)]
pub fn serve_measured() -> ! {
    run(Seal::Measure)
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
