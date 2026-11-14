use std::fmt;
use std::io::{self, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use ssx_formats::reader::{Endian, Reader};

const MAX_MESSAGE: usize = 64 * 1024;
const MAX_LINE: usize = 512;
const MAX_SKIPPED: usize = 64;
const MAX_DEPTH: u32 = 64;
const DEFAULT_SYSTEM_BUS: &str = "unix:path=/var/run/dbus/system_bus_socket";

const METHOD_CALL: u8 = 1;
const METHOD_RETURN: u8 = 2;
const ERROR: u8 = 3;

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Address(String),
    Auth(String),
    Protocol(String),
    Remote { name: String, message: String },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::Address(a) => write!(f, "no usable bus address in {a:?}"),
            Error::Auth(line) => write!(f, "authentication refused: {line:?}"),
            Error::Protocol(what) => write!(f, "malformed message: {what}"),
            Error::Remote { name, message } => write!(f, "{name}: {message}"),
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<ssx_formats::Error> for Error {
    fn from(e: ssx_formats::Error) -> Self {
        Error::Protocol(e.to_string())
    }
}

fn protocol(what: &str) -> Error {
    Error::Protocol(what.to_owned())
}

#[derive(Clone, Copy, Debug)]
pub enum Arg<'a> {
    Str(&'a str),
    U32(u32),
    U64(u64),
}

impl Arg<'_> {
    fn code(&self) -> u8 {
        match self {
            Arg::Str(_) => b's',
            Arg::U32(_) => b'u',
            Arg::U64(_) => b't',
        }
    }
}

#[derive(Debug)]
pub struct Reply {
    pub signature: Vec<u8>,
    body: Vec<u8>,
    endian: Endian,
}

impl Reply {
    pub fn variant_int(&self) -> Result<i64, Error> {
        if self.signature != b"v" {
            return Err(protocol("the reply is not one variant"));
        }
        let mut r = Reader::new(&self.body, self.endian);
        let sig = signature(&mut r)?;
        let value = match sig.as_slice() {
            b"y" => i64::from(r.u8()?),
            b"n" => i64::from(padded(&mut r, 2)?.i16()?),
            b"q" => i64::from(padded(&mut r, 2)?.u16()?),
            b"i" => i64::from(padded(&mut r, 4)?.i32()?),
            b"u" => i64::from(padded(&mut r, 4)?.u32()?),
            b"x" => padded(&mut r, 8)?.u64()? as i64,
            b"t" => i64::try_from(padded(&mut r, 8)?.u64()?).map_err(|_| protocol("an integer out of range"))?,
            _ => return Err(protocol("the variant holds no integer")),
        };
        if r.remaining() != 0 {
            return Err(protocol("bytes after the variant"));
        }
        Ok(value)
    }
}

fn padded<'r, 'a>(r: &'r mut Reader<'a>, align: usize) -> Result<&'r mut Reader<'a>, Error> {
    let n = r.pos().next_multiple_of(align) - r.pos();
    if r.bytes(n)?.iter().any(|b| *b != 0) {
        return Err(protocol("padding that is not zero"));
    }
    Ok(r)
}

fn string(r: &mut Reader<'_>) -> Result<String, Error> {
    let len = padded(r, 4)?.u32()? as usize;
    let bytes = r.bytes(len)?;
    if r.u8()? != 0 || bytes.contains(&0) {
        return Err(protocol("a string not ended by its NUL"));
    }
    String::from_utf8(bytes.to_vec()).map_err(|_| protocol("a string that is not UTF-8"))
}

fn signature(r: &mut Reader<'_>) -> Result<Vec<u8>, Error> {
    let len = usize::from(r.u8()?);
    let sig = r.bytes(len)?.to_vec();
    if r.u8()? != 0 {
        return Err(protocol("a signature not ended by its NUL"));
    }
    let mut at = 0;
    while at < sig.len() {
        at += single_type(&sig[at..], 0)?;
    }
    Ok(sig)
}

fn single_type(sig: &[u8], depth: u32) -> Result<usize, Error> {
    if depth > MAX_DEPTH {
        return Err(protocol("a signature nested too deep"));
    }
    match sig.first() {
        Some(b'y' | b'b' | b'n' | b'q' | b'i' | b'u' | b'x' | b't' | b'd' | b'h' | b's' | b'o' | b'g' | b'v') => Ok(1),
        Some(b'a') => Ok(1 + single_type(&sig[1..], depth + 1)?),
        Some(b'(') => {
            let mut at = 1;
            while sig.get(at) != Some(&b')') {
                at += single_type(&sig[at..], depth + 1)?;
            }
            if at == 1 {
                return Err(protocol("an empty struct"));
            }
            Ok(at + 1)
        }
        Some(b'{') => {
            if !matches!(
                sig.get(1),
                Some(b'y' | b'b' | b'n' | b'q' | b'i' | b'u' | b'x' | b't' | b'd' | b'h' | b's' | b'o' | b'g')
            ) {
                return Err(protocol("a dict entry without a basic key"));
            }
            let at = 2 + single_type(&sig[2..], depth + 1)?;
            if sig.get(at) != Some(&b'}') {
                return Err(protocol("a dict entry of more than two types"));
            }
            Ok(at + 1)
        }
        _ => Err(protocol("an unknown type code")),
    }
}

fn alignment(ty: u8) -> usize {
    match ty {
        b'n' | b'q' => 2,
        b'b' | b'i' | b'u' | b'h' | b's' | b'o' | b'a' => 4,
        b'x' | b't' | b'd' | b'(' | b'{' => 8,
        _ => 1,
    }
}

fn skip_value(r: &mut Reader<'_>, ty: &[u8], depth: u32) -> Result<(), Error> {
    if depth > MAX_DEPTH {
        return Err(protocol("a value nested too deep"));
    }
    let Some(&code) = ty.first() else { return Err(protocol("an empty type")) };
    match code {
        b's' | b'o' => drop(string(r)?),
        b'g' => drop(signature(r)?),
        b'v' => {
            let inner = signature(r)?;
            if inner.is_empty() || single_type(&inner, 0)? != inner.len() {
                return Err(protocol("a variant of more than one type"));
            }
            skip_value(r, &inner, depth + 1)?;
        }
        b'a' => {
            let len = padded(r, 4)?.u32()? as usize;
            if len > MAX_MESSAGE {
                return Err(protocol("an array too long"));
            }
            padded(r, alignment(ty[1]))?.skip(len)?;
        }
        b'(' | b'{' => {
            padded(r, 8)?;
            let mut at = 1;
            while at < ty.len() - 1 {
                let n = single_type(&ty[at..], depth + 1)?;
                skip_value(r, &ty[at..at + n], depth + 1)?;
                at += n;
            }
        }
        _ => {
            let size = alignment(code);
            padded(r, size)?.skip(size)?;
        }
    }
    Ok(())
}

fn method_call(serial: u32, dest: &str, path: &str, iface: &str, member: &str, args: &[Arg<'_>]) -> Vec<u8> {
    fn pad(m: &mut Vec<u8>, align: usize) {
        m.resize(m.len().next_multiple_of(align), 0);
    }
    fn string(m: &mut Vec<u8>, s: &str) {
        pad(m, 4);
        m.extend_from_slice(&(s.len() as u32).to_le_bytes());
        m.extend_from_slice(s.as_bytes());
        m.push(0);
    }
    fn signature(m: &mut Vec<u8>, sig: &[u8]) {
        m.push(sig.len() as u8);
        m.extend_from_slice(sig);
        m.push(0);
    }
    let sig: Vec<u8> = args.iter().map(Arg::code).collect();
    let mut m = vec![b'l', METHOD_CALL, 0, 1, 0, 0, 0, 0];
    m.extend_from_slice(&serial.to_le_bytes());
    m.extend_from_slice(&[0; 4]);
    let fields = m.len();
    let field = |m: &mut Vec<u8>, code: u8, ty: &[u8], value: &dyn Fn(&mut Vec<u8>)| {
        pad(m, 8);
        m.push(code);
        signature(m, ty);
        value(m);
    };
    field(&mut m, 1, b"o", &|m| string(m, path));
    field(&mut m, 2, b"s", &|m| string(m, iface));
    field(&mut m, 3, b"s", &|m| string(m, member));
    field(&mut m, 6, b"s", &|m| string(m, dest));
    if !sig.is_empty() {
        field(&mut m, 8, b"g", &|m| signature(m, &sig));
    }
    let len = (m.len() - fields) as u32;
    m[12..16].copy_from_slice(&len.to_le_bytes());
    pad(&mut m, 8);
    let body = m.len();
    for arg in args {
        match *arg {
            Arg::Str(s) => string(&mut m, s),
            Arg::U32(v) => {
                pad(&mut m, 4);
                m.extend_from_slice(&v.to_le_bytes());
            }
            Arg::U64(v) => {
                pad(&mut m, 8);
                m.extend_from_slice(&v.to_le_bytes());
            }
        }
    }
    let body_len = (m.len() - body) as u32;
    m[4..8].copy_from_slice(&body_len.to_le_bytes());
    m
}

#[derive(Debug, Default)]
struct Message {
    kind: u8,
    reply_serial: Option<u32>,
    error_name: Option<String>,
    signature: Vec<u8>,
    body: Vec<u8>,
    endian: Option<Endian>,
}

fn message_len(head: &[u8; 16]) -> Result<usize, Error> {
    let endian = match head[0] {
        b'l' => Endian::Little,
        b'B' => Endian::Big,
        _ => return Err(protocol("an unknown byte order")),
    };
    if head[3] != 1 {
        return Err(protocol("an unknown protocol version"));
    }
    let mut r = Reader::new(&head[4..], endian);
    let body = r.u32()? as usize;
    let _serial = r.u32()?;
    let fields = r.u32()? as usize;
    let len = (16usize.saturating_add(fields)).next_multiple_of(8).saturating_add(body);
    if fields > MAX_MESSAGE || body > MAX_MESSAGE || len > MAX_MESSAGE {
        return Err(protocol("a message over 64 KiB"));
    }
    Ok(len)
}

fn parse(m: &[u8]) -> Result<Message, Error> {
    let endian = if m.first() == Some(&b'B') { Endian::Big } else { Endian::Little };
    let mut r = Reader::new(m, endian);
    let kind = r.bytes(4)?[1];
    let body_len = r.u32()? as usize;
    if r.u32()? == 0 {
        return Err(protocol("a serial of 0"));
    }
    let end = 16 + r.u32()? as usize;
    let mut msg = Message { kind, endian: Some(endian), ..Message::default() };
    while r.pos() < end {
        padded(&mut r, 8)?;
        let code = r.u8()?;
        let ty = signature(&mut r)?;
        if ty.is_empty() || single_type(&ty, 0)? != ty.len() {
            return Err(protocol("a header field of more than one type"));
        }
        match (code, ty.as_slice()) {
            (4, b"s") => msg.error_name = Some(string(&mut r)?),
            (5, b"u") => msg.reply_serial = Some(padded(&mut r, 4)?.u32()?),
            (8, b"g") => msg.signature = signature(&mut r)?,
            (1..=9, _) if !matches!((code, ty.as_slice()), (1, b"o") | (2 | 3 | 6 | 7, b"s") | (9, b"u")) => {
                return Err(protocol("a header field of the wrong type"));
            }
            _ => skip_value(&mut r, &ty, 1)?,
        }
    }
    if r.pos() != end {
        return Err(protocol("header fields past their array"));
    }
    padded(&mut r, 8)?;
    msg.body = r.bytes(body_len)?.to_vec();
    if r.remaining() != 0 {
        return Err(protocol("bytes after the body"));
    }
    Ok(msg)
}

pub struct Bus {
    stream: UnixStream,
    deadline: Instant,
    serial: u32,
    pending: Vec<u8>,
}

impl Bus {
    pub fn system(timeout: Duration) -> Result<Self, Error> {
        let deadline = Instant::now() + timeout;
        let address = std::env::var("DBUS_SYSTEM_BUS_ADDRESS").unwrap_or_else(|_| DEFAULT_SYSTEM_BUS.to_owned());
        Self::start(connect(&address)?, deadline)
    }

    fn start(stream: UnixStream, deadline: Instant) -> Result<Self, Error> {
        let mut bus = Self { stream, deadline, serial: 0, pending: Vec::new() };
        bus.write(b"\0AUTH EXTERNAL\r\nDATA\r\n")?;
        let mut line = bus.line()?;
        if line == "DATA" || line.starts_with("DATA ") {
            line = bus.line()?;
        }
        if !line.starts_with("OK ") {
            return Err(Error::Auth(line));
        }
        bus.write(b"BEGIN\r\n")?;
        let hello = bus.call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "Hello", &[])?;
        if hello.signature != b"s" {
            return Err(protocol("Hello's reply is not a name"));
        }
        Ok(bus)
    }

    pub fn call(
        &mut self,
        dest: &str,
        path: &str,
        iface: &str,
        member: &str,
        args: &[Arg<'_>],
    ) -> Result<Reply, Error> {
        self.serial = self.serial.wrapping_add(1).max(1);
        let serial = self.serial;
        self.write(&method_call(serial, dest, path, iface, member, args))?;
        for _ in 0..MAX_SKIPPED {
            let mut head = [0u8; 16];
            head.copy_from_slice(self.take(16)?.as_slice());
            let len = message_len(&head)?;
            let mut m = head.to_vec();
            m.extend(self.take(len - 16)?);
            let msg = parse(&m)?;
            if msg.reply_serial != Some(serial) || !matches!(msg.kind, METHOD_RETURN | ERROR) {
                continue;
            }
            let endian = msg.endian.unwrap_or(Endian::Little);
            if msg.kind == ERROR {
                let name = msg.error_name.unwrap_or_default();
                let message = if msg.signature.first() == Some(&b's') {
                    string(&mut Reader::new(&msg.body, endian)).unwrap_or_default()
                } else {
                    String::new()
                };
                return Err(Error::Remote { name, message });
            }
            return Ok(Reply { signature: msg.signature, body: msg.body, endian });
        }
        Err(protocol("no reply among the bus's messages"))
    }

    fn left(&self) -> io::Result<Duration> {
        self.deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "the bus did not answer in time"))
    }

    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.stream.set_write_timeout(Some(self.left()?))?;
        self.stream.write_all(bytes)
    }

    fn fill(&mut self, n: usize) -> io::Result<()> {
        let mut chunk = [0u8; 4096];
        while self.pending.len() < n {
            self.stream.set_read_timeout(Some(self.left()?))?;
            match self.stream.read(&mut chunk)? {
                0 => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the bus closed the connection")),
                got => self.pending.extend_from_slice(&chunk[..got]),
            }
        }
        Ok(())
    }

    fn take(&mut self, n: usize) -> Result<Vec<u8>, Error> {
        self.fill(n)?;
        Ok(self.pending.drain(..n).collect())
    }

    fn line(&mut self) -> Result<String, Error> {
        loop {
            if let Some(end) = self.pending.windows(2).position(|w| w == b"\r\n") {
                let line: Vec<u8> = self.pending.drain(..end + 2).take(end).collect();
                if !line.iter().all(|b| b.is_ascii_graphic() || *b == b' ') {
                    return Err(Error::Auth(String::from_utf8_lossy(&line).into_owned()));
                }
                return Ok(String::from_utf8_lossy(&line).into_owned());
            }
            if self.pending.len() > MAX_LINE {
                return Err(protocol("an authentication line too long"));
            }
            let want = self.pending.len() + 1;
            self.fill(want)?;
        }
    }
}

fn connect(address: &str) -> Result<UnixStream, Error> {
    let mut last = None;
    for one in address.split(';') {
        let Some(params) = one.strip_prefix("unix:") else { continue };
        for (key, value) in params.split(',').filter_map(|kv| kv.split_once('=')) {
            let Some(value) = unescape(value) else { continue };
            let connected = match key {
                "path" => UnixStream::connect(std::ffi::OsStr::from_bytes(&value)),
                "abstract" => {
                    use std::os::linux::net::SocketAddrExt;
                    std::os::unix::net::SocketAddr::from_abstract_name(&value)
                        .and_then(|a| UnixStream::connect_addr(&a))
                }
                _ => continue,
            };
            match connected {
                Ok(s) => return Ok(s),
                Err(e) => last = Some(e),
            }
        }
    }
    Err(last.map_or_else(|| Error::Address(address.chars().take(200).collect()), Error::Io))
}

fn unescape(value: &str) -> Option<Vec<u8>> {
    let b = value.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = b.get(i + 1..i + 3).filter(|h| h.iter().all(u8::is_ascii_hexdigit))?;
            out.push(u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(big: bool, kind: u8, serial: u32, fields: &[(u8, &str, Value)], body: &[u8]) -> Vec<u8> {
        let mut w = W { b: vec![if big { b'B' } else { b'l' }, kind, 0, 1], big };
        w.u32(body.len() as u32);
        w.u32(serial);
        w.u32(0);
        for (code, ty, value) in fields {
            w.pad(8);
            w.b.push(*code);
            w.sig(ty);
            match value {
                Value::U32(v) => w.u32(*v),
                Value::Str(s) => w.str(s),
                Value::Sig(s) => w.sig(s),
                Value::U64s(vs) => {
                    w.u32(8 * vs.len() as u32);
                    w.pad(8);
                    vs.iter().for_each(|v| w.u64(*v));
                }
            }
        }
        let len = (w.b.len() - 16) as u32;
        w.b[12..16].copy_from_slice(&if big { len.to_be_bytes() } else { len.to_le_bytes() });
        w.pad(8);
        w.b.extend_from_slice(body);
        w.b
    }

    enum Value {
        U32(u32),
        Str(&'static str),
        Sig(String),
        U64s(Vec<u64>),
    }

    struct W {
        b: Vec<u8>,
        big: bool,
    }

    impl W {
        fn pad(&mut self, align: usize) {
            self.b.resize(self.b.len().next_multiple_of(align), 0);
        }
        fn u32(&mut self, v: u32) {
            self.pad(4);
            self.b.extend_from_slice(&if self.big { v.to_be_bytes() } else { v.to_le_bytes() });
        }
        fn u64(&mut self, v: u64) {
            self.pad(8);
            self.b.extend_from_slice(&if self.big { v.to_be_bytes() } else { v.to_le_bytes() });
        }
        fn str(&mut self, s: &str) {
            self.u32(s.len() as u32);
            self.b.extend_from_slice(s.as_bytes());
            self.b.push(0);
        }
        fn sig(&mut self, s: &str) {
            self.b.push(s.len() as u8);
            self.b.extend_from_slice(s.as_bytes());
            self.b.push(0);
        }
    }

    fn variant_x(big: bool, v: i64) -> Vec<u8> {
        let mut w = W { b: Vec::new(), big };
        w.sig("x");
        w.u64(v as u64);
        w.b
    }

    fn reply(big: bool, to: u32, sig: &str, body: &[u8]) -> Vec<u8> {
        message(big, METHOD_RETURN, 9, &[(5, "u", Value::U32(to)), (8, "g", Value::Sig(sig.to_owned()))], body)
    }

    fn whole(m: &[u8]) -> Result<Message, Error> {
        let len = message_len(m[..16].try_into().unwrap())?;
        assert_eq!(len, m.len());
        parse(m)
    }

    #[test]
    fn a_call_reads_back_as_written() {
        let m = method_call(7, RTKIT, "/p", RTKIT, "MakeThreadRealtime", &[Arg::U64(1234), Arg::U32(10)]);
        let msg = whole(&m).unwrap();
        assert_eq!((msg.kind, msg.signature.as_slice()), (METHOD_CALL, b"tu".as_slice()));
        assert_eq!(&msg.body[..8], &1234u64.to_le_bytes());
        assert_eq!(&msg.body[8..], &10u32.to_le_bytes());
        let hello = whole(&method_call(1, "a.b", "/a", "a.b", "Hello", &[])).unwrap();
        assert!(hello.signature.is_empty() && hello.body.is_empty());
    }

    const RTKIT: &str = "org.freedesktop.RealtimeKit1";

    #[test]
    fn a_variant_reads_in_either_byte_order() {
        for big in [false, true] {
            let msg = whole(&reply(big, 3, "v", &variant_x(big, 200_000))).unwrap();
            assert_eq!(msg.reply_serial, Some(3));
            let r = Reply { signature: msg.signature, body: msg.body, endian: msg.endian.unwrap() };
            assert_eq!(r.variant_int().unwrap(), 200_000);
        }
        let mut w = W { b: Vec::new(), big: false };
        w.sig("s");
        w.str("20");
        let r = Reply { signature: b"v".to_vec(), body: w.b, endian: Endian::Little };
        assert!(r.variant_int().is_err());
    }

    #[test]
    fn hostile_messages_are_errors() {
        let good = reply(false, 3, "v", &variant_x(false, 1));
        for n in 16..good.len() {
            assert!(parse(&good[..n]).is_err(), "cut at {n}");
        }
        let mut huge = good.clone();
        huge[4..8].copy_from_slice(&(1u32 << 30).to_le_bytes());
        assert!(message_len(huge[..16].try_into().unwrap()).is_err());
        let mut order = good.clone();
        order[0] = b'x';
        assert!(message_len(order[..16].try_into().unwrap()).is_err());
        let mut version = good.clone();
        version[3] = 2;
        assert!(message_len(version[..16].try_into().unwrap()).is_err());
        let mut pad = good.clone();
        assert_eq!(u32::from_le_bytes(pad[12..16].try_into().unwrap()), 15);
        pad[31] = 1;
        assert!(parse(&pad).is_err());
        let msg = parse(&good).unwrap();
        let mut body = msg.body.clone();
        body[7] = 1;
        let r = Reply { signature: msg.signature, body, endian: Endian::Little };
        assert!(r.variant_int().is_err());
        let wrong = message(false, METHOD_RETURN, 9, &[(5, "s", Value::Str("3"))], &[]);
        assert!(parse(&wrong).is_err());
        let deep = format!("{}y", "a".repeat(70));
        assert!(parse(&message(false, METHOD_RETURN, 9, &[(8, "g", Value::Sig(deep))], &[])).is_err());
        assert!(parse(&message(false, METHOD_RETURN, 9, &[(8, "g", Value::Sig("z".into()))], &[])).is_err());
        assert!(parse(&message(false, METHOD_RETURN, 0, &[], &[])).is_err());
        let unknown =
            message(false, METHOD_RETURN, 9, &[(200, "at", Value::U64s(vec![5, 6])), (5, "u", Value::U32(3))], &[]);
        assert_eq!(whole(&unknown).unwrap().reply_serial, Some(3));
    }

    #[test]
    fn addresses() {
        assert_eq!(unescape("%2frun%2Fdbus").as_deref(), Some(b"/run/dbus".as_slice()));
        assert_eq!(unescape("%zz"), None);
        assert_eq!(unescape("%+f"), None);
        assert_eq!(unescape("%2"), None);
        assert!(matches!(connect("tcp:host=localhost,port=1"), Err(Error::Address(_))));
        assert!(matches!(connect("unix:path=/nonexistent/ssx-bus"), Err(Error::Io(_))));
    }

    fn serve(mut s: UnixStream, answer: impl FnOnce(u32) -> Vec<u8>) {
        let mut auth = Vec::new();
        let mut byte = [0u8; 1];
        while !auth.ends_with(b"\r\n") {
            s.read_exact(&mut byte).unwrap();
            auth.push(byte[0]);
        }
        assert_eq!(auth, b"\0AUTH EXTERNAL\r\n");
        s.write_all(b"DATA\r\n").unwrap();
        let mut data = [0u8; 6];
        s.read_exact(&mut data).unwrap();
        assert_eq!(&data, b"DATA\r\n");
        s.write_all(b"OK 0123456789abcdef0123456789abcdef\r\n").unwrap();
        let mut begin = [0u8; 7];
        s.read_exact(&mut begin).unwrap();
        assert_eq!(&begin, b"BEGIN\r\n");
        let read = |s: &mut UnixStream| {
            let mut head = [0u8; 16];
            s.read_exact(&mut head).unwrap();
            let mut m = head.to_vec();
            m.resize(message_len(&head).unwrap(), 0);
            s.read_exact(&mut m[16..]).unwrap();
            let serial = u32::from_le_bytes(m[8..12].try_into().unwrap());
            (parse(&m).unwrap(), serial)
        };
        let (hello, serial) = read(&mut s);
        assert_eq!(hello.kind, METHOD_CALL);
        s.write_all(&message(true, 4, 1, &[(8, "g", Value::Sig("s".into()))], &[0, 0, 0, 1, b'x', 0])).unwrap();
        let mut name = W { b: Vec::new(), big: false };
        name.str(":1.42");
        s.write_all(&reply(false, serial, "s", &name.b)).unwrap();
        let (_, serial) = read(&mut s);
        s.write_all(&answer(serial)).unwrap();
    }

    #[test]
    fn a_call_over_a_connection_gets_its_reply_or_its_error() {
        let deadline = || Instant::now() + Duration::from_secs(10);
        let (ours, theirs) = UnixStream::pair().unwrap();
        let bus = std::thread::spawn(move || serve(theirs, |to| reply(true, to, "v", &variant_x(true, 20))));
        let mut b = Bus::start(ours, deadline()).unwrap();
        let r = b.call(RTKIT, "/p", "org.freedesktop.DBus.Properties", "Get", &[Arg::Str(RTKIT), Arg::Str("x")]);
        assert_eq!(r.unwrap().variant_int().unwrap(), 20);
        bus.join().unwrap();

        let (ours, theirs) = UnixStream::pair().unwrap();
        let bus = std::thread::spawn(move || {
            serve(theirs, |to| {
                let mut text = W { b: Vec::new(), big: false };
                text.str("Operation not permitted");
                let fields = [
                    (4, "s", Value::Str("org.freedesktop.DBus.Error.AccessDenied")),
                    (5, "u", Value::U32(to)),
                    (8, "g", Value::Sig("s".into())),
                ];
                message(false, ERROR, 5, &fields, &text.b)
            })
        });
        let mut b = Bus::start(ours, deadline()).unwrap();
        match b.call(RTKIT, "/p", RTKIT, "MakeThreadRealtime", &[Arg::U64(1), Arg::U32(10)]) {
            Err(Error::Remote { name, message }) => {
                assert_eq!(name, "org.freedesktop.DBus.Error.AccessDenied");
                assert_eq!(message, "Operation not permitted");
            }
            other => panic!("{other:?}"),
        }
        bus.join().unwrap();
    }

    #[test]
    fn a_silent_bus_times_out_and_a_refusal_is_an_error() {
        let (ours, _theirs) = UnixStream::pair().unwrap();
        let t = Instant::now();
        assert!(matches!(Bus::start(ours, Instant::now() + Duration::from_millis(200)), Err(Error::Io(_))));
        assert!(t.elapsed() < Duration::from_secs(5));
        let (ours, mut theirs) = UnixStream::pair().unwrap();
        theirs.write_all(b"REJECTED EXTERNAL\r\n").unwrap();
        assert!(matches!(Bus::start(ours, Instant::now() + Duration::from_secs(10)), Err(Error::Auth(_))));
    }
}
