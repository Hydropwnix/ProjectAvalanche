use crate::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Endian {
    Big,
    Little,
}

#[derive(Clone)]
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    endian: Endian,
}

macro_rules! read_int {
    ($name:ident, $ty:ty) => {
        pub fn $name(&mut self) -> Result<$ty> {
            let bytes = self.array::<{ std::mem::size_of::<$ty>() }>()?;
            Ok(match self.endian {
                Endian::Big => <$ty>::from_be_bytes(bytes),
                Endian::Little => <$ty>::from_le_bytes(bytes),
            })
        }
    };
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8], endian: Endian) -> Self {
        Self { data, pos: 0, endian }
    }

    pub fn be(data: &'a [u8]) -> Self {
        Self::new(data, Endian::Big)
    }

    pub fn le(data: &'a [u8]) -> Self {
        Self::new(data, Endian::Little)
    }

    pub fn endian(&self) -> Endian {
        self.endian
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    pub fn data(&self) -> &'a [u8] {
        self.data
    }

    pub fn seek(&mut self, pos: usize) -> Result<()> {
        if pos > self.data.len() {
            return Err(Error::UnexpectedEof { offset: pos, wanted: 0, available: 0 });
        }
        self.pos = pos;
        Ok(())
    }

    pub fn skip(&mut self, n: usize) -> Result<()> {
        self.bytes(n).map(|_| ())
    }

    pub fn align(&mut self, align: usize) -> Result<()> {
        let aligned = self.pos.next_multiple_of(align);
        self.seek(aligned)
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        if n > self.remaining() {
            return Err(Error::UnexpectedEof { offset: self.pos, wanted: n, available: self.remaining() });
        }
        let out = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(out)
    }

    pub fn peek(&self, n: usize) -> Result<&'a [u8]> {
        self.clone().bytes(n)
    }

    pub fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        Ok(self.bytes(N)?.try_into().expect("length checked"))
    }

    pub fn expect(&mut self, magic: &'static [u8]) -> Result<()> {
        let found = self.peek(magic.len().min(self.remaining()))?;
        if found != magic {
            return Err(Error::BadMagic { offset: self.pos, expected: magic, found: found.to_vec() });
        }
        self.pos += magic.len();
        Ok(())
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.array::<1>()?[0])
    }

    pub fn i8(&mut self) -> Result<i8> {
        Ok(self.u8()? as i8)
    }

    read_int!(u16, u16);
    read_int!(i16, i16);
    read_int!(u32, u32);
    read_int!(i32, i32);
    read_int!(u64, u64);

    pub fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.u32()?))
    }

    pub fn vec3(&mut self) -> Result<[f32; 3]> {
        Ok([self.f32()?, self.f32()?, self.f32()?])
    }

    pub fn fixed_str(&mut self, n: usize) -> Result<String> {
        let raw = self.bytes(n)?;
        let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
        Ok(String::from_utf8_lossy(&raw[..end]).into_owned())
    }

    pub fn cstr(&mut self) -> Result<String> {
        let rest = &self.data[self.pos..];
        let end = rest.iter().position(|&b| b == 0).ok_or(Error::UnexpectedEof {
            offset: self.pos,
            wanted: rest.len() + 1,
            available: rest.len(),
        })?;
        let s = String::from_utf8_lossy(&rest[..end]).into_owned();
        self.pos += end + 1;
        Ok(s)
    }
}
