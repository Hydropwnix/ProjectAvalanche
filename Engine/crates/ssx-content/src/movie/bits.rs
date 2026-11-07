pub(super) struct Msb<'a> {
    data: &'a [u8],
    bit: usize,
}

impl<'a> Msb<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, bit: 0 }
    }

    pub fn f(&mut self, n: u32) -> Result<u32, String> {
        let mut v = 0u32;
        for _ in 0..n {
            let byte = self.data.get(self.bit / 8).ok_or("the sequence header ends early")?;
            v = (v << 1) | u32::from((byte >> (7 - self.bit % 8)) & 1);
            self.bit += 1;
        }
        Ok(v)
    }

    pub fn flag(&mut self) -> Result<bool, String> {
        Ok(self.f(1)? == 1)
    }

    pub fn uvlc(&mut self) -> Result<u32, String> {
        let mut zeros = 0u32;
        while !self.flag()? {
            zeros += 1;
        }
        if zeros >= 32 {
            return Ok(u32::MAX);
        }
        Ok(self.f(zeros)?.saturating_add((1u32 << zeros) - 1))
    }

    pub fn bits_read(&self) -> usize {
        self.bit
    }
}

pub(super) struct Lsb<'a> {
    data: &'a [u8],
    bit: usize,
}

impl<'a> Lsb<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, bit: 0 }
    }

    pub fn read(&mut self, n: u32) -> Option<u32> {
        let mut v = 0u32;
        for i in 0..n {
            let byte = self.data.get(self.bit / 8)?;
            v |= u32::from((byte >> (self.bit % 8)) & 1) << i;
            self.bit += 1;
        }
        Some(v)
    }
}

pub(super) fn ilog(v: u32) -> u32 {
    32 - v.leading_zeros()
}
