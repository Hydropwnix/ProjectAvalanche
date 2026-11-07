use std::borrow::Cow;
use std::fmt::{self, Write as _};

#[cfg(feature = "log")]
pub mod log;
#[cfg(test)]
mod tests;

pub const VALUE_CHARS: usize = 64;
pub const PATH_CHARS: usize = 1024;
pub const LIST_ITEMS: usize = 8;
pub const MESSAGE_CHARS: usize = 512;
pub const LINE_CHARS: usize = 4096;

const FORMAT: [(u32, u32); 21] = [
    (0x00AD, 0x00AD),
    (0x0600, 0x0605),
    (0x061C, 0x061C),
    (0x06DD, 0x06DD),
    (0x070F, 0x070F),
    (0x0890, 0x0891),
    (0x08E2, 0x08E2),
    (0x180E, 0x180E),
    (0x200B, 0x200F),
    (0x202A, 0x202E),
    (0x2060, 0x2064),
    (0x2066, 0x206F),
    (0xFEFF, 0xFEFF),
    (0xFFF9, 0xFFFB),
    (0x110BD, 0x110BD),
    (0x110CD, 0x110CD),
    (0x13430, 0x1343F),
    (0x1BCA0, 0x1BCA3),
    (0x1D173, 0x1D17A),
    (0xE0001, 0xE0001),
    (0xE0020, 0xE007F),
];

pub fn is_hidden(c: char) -> bool {
    let cp = u32::from(c);
    c.is_control() || cp == 0x2028 || cp == 0x2029 || FORMAT.iter().any(|&(lo, hi)| (lo..=hi).contains(&cp))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Plain,
    Quoted,
    Lines,
}

fn width(c: char, mode: Mode) -> usize {
    match c {
        '\n' if mode == Mode::Lines => 1,
        '\t' | '\n' | '\r' => 2,
        '"' | '\\' if mode == Mode::Quoted => 2,
        c if is_hidden(c) => 4 + hex_digits(u32::from(c)),
        _ => 1,
    }
}

fn hex_digits(cp: u32) -> usize {
    ((32 - cp.leading_zeros()).div_ceil(4)).max(1) as usize
}

fn put(out: &mut dyn fmt::Write, c: char, mode: Mode) -> fmt::Result {
    match c {
        '\n' if mode == Mode::Lines => out.write_char('\n'),
        '\t' => out.write_str("\\t"),
        '\n' => out.write_str("\\n"),
        '\r' => out.write_str("\\r"),
        '"' | '\\' if mode == Mode::Quoted => {
            out.write_char('\\')?;
            out.write_char(c)
        }
        c if is_hidden(c) => write!(out, "\\u{{{:x}}}", u32::from(c)),
        c => out.write_char(c),
    }
}

struct Bounded<'a> {
    out: &'a mut dyn fmt::Write,
    room: usize,
    mode: Mode,
    cut: bool,
    bytes: u64,
}

impl<'a> Bounded<'a> {
    fn new(out: &'a mut dyn fmt::Write, room: usize, mode: Mode) -> Self {
        Bounded { out, room, mode, cut: false, bytes: 0 }
    }

    fn finish(self) -> fmt::Result {
        if self.cut { write!(self.out, "… ({} bytes)", self.bytes) } else { Ok(()) }
    }
}

impl fmt::Write for Bounded<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.bytes = self.bytes.saturating_add(s.len() as u64);
        if self.cut {
            return Ok(());
        }
        for c in s.chars() {
            let n = width(c, self.mode);
            if n > self.room {
                self.cut = true;
                return Ok(());
            }
            self.room -= n;
            put(self.out, c, self.mode)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub struct Shown<'a> {
    text: &'a str,
    max: usize,
    mode: Mode,
}

impl fmt::Display for Shown<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let quoted = self.mode == Mode::Quoted;
        if quoted {
            f.write_char('"')?;
        }
        let mut b = Bounded::new(f, self.max, self.mode);
        b.write_str(self.text)?;
        b.finish()?;
        if quoted { f.write_char('"') } else { Ok(()) }
    }
}

impl fmt::Debug for Shown<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

pub fn shown(s: &str) -> Shown<'_> {
    shown_max(s, VALUE_CHARS)
}

pub fn shown_max(s: &str, max: usize) -> Shown<'_> {
    Shown { text: s, max, mode: Mode::Plain }
}

pub fn quoted(s: &str) -> Shown<'_> {
    quoted_max(s, VALUE_CHARS)
}

pub fn quoted_max(s: &str, max: usize) -> Shown<'_> {
    Shown { text: s, max, mode: Mode::Quoted }
}

pub fn located(place: &str, message: impl fmt::Display) -> Text {
    Text::new(format_args!("{}: {message}", shown_max(place, PATH_CHARS)))
}

pub fn quoted_list<S: AsRef<str>>(items: impl IntoIterator<Item = S>) -> Text {
    let mut out = String::from("[");
    let mut n = 0usize;
    for s in items {
        if n < LIST_ITEMS {
            let gap = if n == 0 { "" } else { ", " };
            let _ = write!(out, "{gap}{}", quoted(s.as_ref()));
        }
        n += 1;
    }
    if n > LIST_ITEMS {
        let _ = write!(out, ", … ({n} in all)");
    }
    out.push(']');
    Text::new(out)
}

pub fn shown_lines(s: &str, max: usize) -> Shown<'_> {
    Shown { text: s, max, mode: Mode::Lines }
}

pub fn escape(s: &str) -> Cow<'_, str> {
    if !s.chars().any(|c| width(c, Mode::Plain) > 1) {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len() + 16);
    let _ = Escaping(&mut out).write_str(s);
    Cow::Owned(out)
}

pub struct Escaping<W>(pub W);

impl<W: fmt::Write> fmt::Write for Escaping<W> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.chars() {
            put(&mut self.0, c, Mode::Plain)?;
        }
        Ok(())
    }
}

#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Text(String);

impl Text {
    pub fn new(d: impl fmt::Display) -> Self {
        Self::with_max(d, LINE_CHARS)
    }

    pub fn with_max(d: impl fmt::Display, max: usize) -> Self {
        let mut out = String::new();
        let mut b = Bounded::new(&mut out, max, Mode::Plain);
        let _ = write!(b, "{d}");
        let _ = b.finish();
        Text(out)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl std::ops::Deref for Text {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for Text {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::borrow::Borrow<str> for Text {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.0.as_str(), f)
    }
}

impl From<String> for Text {
    fn from(s: String) -> Self {
        Text::new(s)
    }
}

impl From<&str> for Text {
    fn from(s: &str) -> Self {
        Text::new(s)
    }
}

impl From<&String> for Text {
    fn from(s: &String) -> Self {
        Text::new(s)
    }
}

impl PartialEq<str> for Text {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for Text {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

pub fn message(m: &str) -> Text {
    Text::with_max(m.trim(), MESSAGE_CHARS)
}

pub const CHAIN_LINES: usize = 256;

pub struct Chain<'a>(pub &'a (dyn std::error::Error + 'static));

impl fmt::Display for Chain<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut left = CHAIN_LINES;
        let mut skipped = 0usize;
        let mut lines = |f: &mut fmt::Formatter<'_>, text: &str, first: &str, rest: &str| -> fmt::Result {
            for (i, line) in text.split('\n').enumerate() {
                if left == 0 {
                    skipped += 1;
                    continue;
                }
                left -= 1;
                let lead = if i == 0 { first } else { rest };
                write!(f, "{lead}{}", Text::new(line))?;
            }
            Ok(())
        };
        lines(f, &self.0.to_string(), "", "\n    ")?;
        let Some(cause) = self.0.source() else { return finish(f, skipped) };
        f.write_str("\n\nCaused by:")?;
        let several = cause.source().is_some();
        let mut next = Some(cause);
        let mut n = 0usize;
        while let Some(c) = next {
            let first = if several { format!("\n{n:>5}: ") } else { "\n    ".to_string() };
            let rest = if several { "\n       " } else { "\n    " };
            lines(f, &c.to_string(), &first, rest)?;
            n += 1;
            next = c.source();
        }
        finish(f, skipped)
    }
}

fn finish(f: &mut fmt::Formatter<'_>, skipped: usize) -> fmt::Result {
    if skipped > 0 { write!(f, "\n    … ({skipped} more lines)") } else { Ok(()) }
}

pub fn one_line(e: &(dyn std::error::Error + 'static)) -> Text {
    struct Joined<'a>(&'a (dyn std::error::Error + 'static));
    impl fmt::Display for Joined<'_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}", self.0)?;
            let mut cause = self.0.source();
            while let Some(c) = cause {
                write!(f, ": {c}")?;
                cause = c.source();
            }
            Ok(())
        }
    }
    Text::new(Joined(e))
}
