use std::io::{BufReader, Read, Seek};

use super::{MovieError, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub id: u32,
    pub start: u64,
    pub body: u64,
    pub size: u64,
}

impl Header {
    pub fn end(&self) -> u64 {
        self.body + self.size
    }
}

pub struct Reader<R> {
    inner: BufReader<R>,
    pos: u64,
    len: u64,
}

impl<R: Read + Seek> Reader<R> {
    pub fn new(inner: R, len: u64) -> Self {
        Self { inner: BufReader::with_capacity(64 * 1024, inner), pos: 0, len }
    }

    pub fn pos(&self) -> u64 {
        self.pos
    }

    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn fail(&self, message: impl Into<String>) -> MovieError {
        MovieError::new(self.pos, message)
    }

    pub fn read_exact(&mut self, buf: &mut [u8], what: &str) -> Result<()> {
        if self.pos + buf.len() as u64 > self.len {
            return Err(self.fail(format!("the file ends inside {what} (cut short)")));
        }
        self.inner.read_exact(buf).map_err(|e| self.fail(format!("{what}: {e}")))?;
        self.pos += buf.len() as u64;
        Ok(())
    }

    fn byte(&mut self, what: &str) -> Result<u8> {
        let mut b = [0u8; 1];
        self.read_exact(&mut b, what)?;
        Ok(b[0])
    }

    pub fn seek_to(&mut self, to: u64) -> Result<()> {
        if to > self.len {
            return Err(self.fail(format!("a jump to byte {to}, past the file's end ({})", self.len)));
        }
        let delta = i64::try_from(to).unwrap_or(i64::MAX) - i64::try_from(self.pos).unwrap_or(i64::MAX);
        self.inner.seek_relative(delta).map_err(|e| self.fail(format!("seek: {e}")))?;
        self.pos = to;
        Ok(())
    }

    pub fn header(&mut self, parent_end: u64) -> Result<Header> {
        let start = self.pos;
        let first = self.byte("an element's id")?;
        let len = first.leading_zeros() as usize + 1;
        if len > 4 || first == 0xFF {
            return Err(MovieError::new(
                start,
                format!("an element id beginning 0x{first:02x} (ids are 1 to 4 bytes)"),
            ));
        }
        let mut id = u32::from(first);
        for _ in 1..len {
            id = (id << 8) | u32::from(self.byte("an element's id")?);
        }
        let size_at = self.pos;
        let first = self.byte("an element's size")?;
        let len = first.leading_zeros() as usize + 1;
        if len > 8 {
            return Err(MovieError::new(size_at, "an element size of more than 8 bytes"));
        }
        let mut size = u64::from(first) & (0xFF >> len);
        let mut all_ones = size == (0xFF >> len);
        for _ in 1..len {
            let b = self.byte("an element's size")?;
            all_ones &= b == 0xFF;
            size = (size << 8) | u64::from(b);
        }
        if all_ones {
            return Err(MovieError::new(
                size_at,
                format!(
                    "element 0x{id:X} states no size (a live stream's unknown size; the movie must be written to a file)"
                ),
            ));
        }
        let body = self.pos;
        let end = body.checked_add(size).filter(|&e| e <= parent_end);
        if end.is_none() {
            return Err(MovieError::new(
                start,
                format!("element 0x{id:X} of {size} bytes reaches past its parent's end (byte {parent_end})"),
            ));
        }
        Ok(Header { id, start, body, size })
    }

    pub fn skip(&mut self, h: &Header) -> Result<()> {
        self.seek_to(h.end())
    }

    pub fn uint(&mut self, h: &Header, name: &str) -> Result<u64> {
        if h.size > 8 {
            return Err(MovieError::new(h.start, format!("{name}: an integer of {} bytes (at most 8)", h.size)));
        }
        let mut b = [0u8; 8];
        let n = h.size as usize;
        self.read_exact(&mut b[8 - n..], name)?;
        Ok(u64::from_be_bytes(b))
    }

    pub fn sint(&mut self, h: &Header, name: &str) -> Result<i64> {
        let v = self.uint(h, name)?;
        let bits = h.size * 8;
        Ok(if bits == 0 || bits == 64 { v as i64 } else { ((v << (64 - bits)) as i64) >> (64 - bits) })
    }

    pub fn float(&mut self, h: &Header, name: &str) -> Result<f64> {
        let v = match h.size {
            0 => 0.0,
            4 => {
                let mut b = [0u8; 4];
                self.read_exact(&mut b, name)?;
                f64::from(f32::from_be_bytes(b))
            }
            8 => {
                let mut b = [0u8; 8];
                self.read_exact(&mut b, name)?;
                f64::from_be_bytes(b)
            }
            n => return Err(MovieError::new(h.start, format!("{name}: a float of {n} bytes (4 or 8)"))),
        };
        if !v.is_finite() {
            return Err(MovieError::new(h.start, format!("{name}: {v} is not a finite number")));
        }
        Ok(v)
    }

    pub fn binary(&mut self, h: &Header, cap: u64, name: &str) -> Result<Vec<u8>> {
        if h.size > cap {
            return Err(MovieError::new(h.start, format!("{name} of {} bytes (at most {cap})", h.size)));
        }
        let mut out = vec![0u8; h.size as usize];
        self.read_exact(&mut out, name)?;
        Ok(out)
    }

    pub fn string(&mut self, h: &Header, cap: u64, name: &str) -> Result<String> {
        let mut b = self.binary(h, cap, name)?;
        while b.last() == Some(&0) {
            b.pop();
        }
        let s = String::from_utf8(b).map_err(|_| MovieError::new(h.start, format!("{name} is not UTF-8")))?;
        if s.chars().any(char::is_control) {
            return Err(MovieError::new(h.start, format!("{name} holds control characters")));
        }
        Ok(s)
    }
}
