use crate::{Error, Result};

#[derive(Clone)]
pub struct SwfReader<'a> {
    data: &'a [u8],
    pos: usize,
    bit: u8,
}

impl<'a> SwfReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0, bit: 0 }
    }

    pub fn pos(&self) -> usize {
        self.pos + usize::from(self.bit > 0)
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos())
    }

    pub fn at_end(&self) -> bool {
        self.remaining() == 0
    }

    pub fn data(&self) -> &'a [u8] {
        self.data
    }

    pub fn align(&mut self) {
        if self.bit > 0 {
            self.pos += 1;
            self.bit = 0;
        }
    }

    fn eof(&self, wanted: usize) -> Error {
        Error::UnexpectedEof { offset: self.pos, wanted, available: self.data.len().saturating_sub(self.pos) }
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        self.align();
        let end = self.pos.checked_add(n).filter(|&e| e <= self.data.len()).ok_or_else(|| self.eof(n))?;
        let out = &self.data[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    pub fn rest(&mut self) -> &'a [u8] {
        self.align();
        let out = &self.data[self.pos.min(self.data.len())..];
        self.pos = self.data.len();
        out
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }

    pub fn u16(&mut self) -> Result<u16> {
        let b = self.bytes(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    pub fn i16(&mut self) -> Result<i16> {
        Ok(self.u16()? as i16)
    }

    pub fn u32(&mut self) -> Result<u32> {
        let b = self.bytes(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()? as i32)
    }

    pub fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.u32()?))
    }

    pub fn f64_swapped(&mut self) -> Result<f64> {
        let hi = u64::from(self.u32()?);
        let lo = u64::from(self.u32()?);
        Ok(f64::from_bits(hi << 32 | lo))
    }

    pub fn string(&mut self) -> Result<String> {
        self.align();
        let rest = &self.data[self.pos.min(self.data.len())..];
        let n = rest.iter().position(|&b| b == 0).ok_or_else(|| self.eof(rest.len() + 1))?;
        let s = std::str::from_utf8(&rest[..n])
            .map_err(|e| Error::invalid(format!("SWF string at {:#x}: {e}", self.pos)))?
            .to_owned();
        self.pos += n + 1;
        Ok(s)
    }

    pub fn string_u8(&mut self) -> Result<String> {
        let n = usize::from(self.u8()?);
        let at = self.pos;
        let b = self.bytes(n)?;
        std::str::from_utf8(b).map(str::to_owned).map_err(|e| Error::invalid(format!("string at {at:#x}: {e}")))
    }

    pub fn ub(&mut self, n: u32) -> Result<u32> {
        if n > 32 {
            return Err(Error::invalid(format!("bit field of {n} bits")));
        }
        let mut v = 0u64;
        for _ in 0..n {
            let byte = *self.data.get(self.pos).ok_or_else(|| self.eof(1))?;
            v = v << 1 | u64::from(byte >> (7 - self.bit) & 1);
            self.bit += 1;
            if self.bit == 8 {
                self.bit = 0;
                self.pos += 1;
            }
        }
        Ok(v as u32)
    }

    pub fn sb(&mut self, n: u32) -> Result<i32> {
        let v = self.ub(n)?;
        Ok(if n == 0 || n == 32 { v as i32 } else { ((v << (32 - n)) as i32) >> (32 - n) })
    }

    pub fn fb(&mut self, n: u32) -> Result<f32> {
        Ok(self.sb(n)? as f32 / 65536.0)
    }

    pub fn flag(&mut self) -> Result<bool> {
        Ok(self.ub(1)? == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bit_fields_are_msb_first() {
        let mut r = SwfReader::new(&[0b1011_0011, 0b0101_0000]);
        assert_eq!(r.ub(1).unwrap(), 1);
        assert_eq!(r.ub(3).unwrap(), 0b011);
        assert_eq!(r.sb(4).unwrap(), 0b0011);
        assert_eq!(r.sb(3).unwrap(), 2);
        assert_eq!(r.sb(2).unwrap(), -2);
        assert_eq!(r.pos(), 2);
        r.align();
        assert!(r.at_end());
    }

    #[test]
    fn signed_fields_extend() {
        let mut r = SwfReader::new(&[0b1110_0000]);
        assert_eq!(r.sb(3).unwrap(), -1);
        let mut r = SwfReader::new(&[0b1000_0000]);
        assert_eq!(r.sb(1).unwrap(), -1);
        let mut r = SwfReader::new(&[0x80, 0, 0, 0]);
        assert_eq!(r.sb(32).unwrap(), i32::MIN);
        assert_eq!(SwfReader::new(&[0x40, 0x00, 0x00]).fb(18).unwrap(), 1.0);
        assert_eq!(SwfReader::new(&[0x80, 0x00, 0x00]).fb(17).unwrap(), -1.0);
    }

    #[test]
    fn byte_reads_align_and_are_little_endian() {
        let mut r = SwfReader::new(&[0xFF, 0x34, 0x12, 0x78, 0x56, 0x34, 0x12, b'h', b'i', 0]);
        assert_eq!(r.ub(3).unwrap(), 7);
        assert_eq!(r.pos(), 1);
        assert_eq!(r.u16().unwrap(), 0x1234);
        assert_eq!(r.u32().unwrap(), 0x1234_5678);
        assert_eq!(r.string().unwrap(), "hi");
        assert!(r.at_end());
        assert!(r.u8().is_err());
    }

    #[test]
    fn swapped_double() {
        let v = 1.5f64.to_bits();
        let mut b = Vec::new();
        b.extend_from_slice(&((v >> 32) as u32).to_le_bytes());
        b.extend_from_slice(&(v as u32).to_le_bytes());
        assert_eq!(SwfReader::new(&b).f64_swapped().unwrap(), 1.5);
    }

    #[test]
    fn eof_is_an_error() {
        let mut r = SwfReader::new(&[0xAA]);
        assert!(r.ub(9).is_err());
        let mut r = SwfReader::new(b"x");
        assert!(r.string().is_err());
    }
}
