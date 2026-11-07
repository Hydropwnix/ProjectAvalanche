use crate::Error;

pub struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
    what: &'static str,
}

impl<'a> Cursor<'a> {
    pub fn new(bytes: &'a [u8], what: &'static str) -> Self {
        Self { bytes, at: 0, what }
    }

    pub fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self.at.checked_add(n).ok_or_else(|| self.short())?;
        let s = self.bytes.get(self.at..end).ok_or_else(|| self.short())?;
        self.at = end;
        Ok(s)
    }

    fn short(&self) -> Error {
        Error::malformed(format!("{}: {} bytes end early", self.what, self.bytes.len()))
    }

    pub fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.take(1)?.first().copied().unwrap_or(0))
    }

    pub fn u16(&mut self) -> Result<u16, Error> {
        let s = self.take(2)?;
        Ok(u16::from_be_bytes([s.first().copied().unwrap_or(0), s.get(1).copied().unwrap_or(0)]))
    }

    pub fn u32(&mut self) -> Result<u32, Error> {
        let s = self.take(4)?;
        let mut b = [0u8; 4];
        b.copy_from_slice(s);
        Ok(u32::from_be_bytes(b))
    }

    pub fn array<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        let mut b = [0u8; N];
        b.copy_from_slice(self.take(N)?);
        Ok(b)
    }

    pub fn left(&self) -> usize {
        self.bytes.len().saturating_sub(self.at)
    }
}
