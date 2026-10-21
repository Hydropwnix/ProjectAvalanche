const fn norm_shift(range: u32) -> u32 {
    if range >= 128 { 0 } else { range.leading_zeros() - 24 }
}

pub(crate) struct RangeDecoder<'a> {
    data: &'a [u8],
    pos: usize,
    range: u32,
    count: i32,
    value: u32,
    end_hits: u32,
}

impl<'a> RangeDecoder<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        let byte = |i: usize| u32::from(data.get(i).copied().unwrap_or(0));
        Self { data, pos: 3, range: 255, count: -16, value: byte(0) << 16 | byte(1) << 8 | byte(2), end_hits: 0 }
    }

    #[inline]
    fn renorm(&mut self) {
        let shift = norm_shift(self.range);
        self.range <<= shift;
        self.value <<= shift;
        self.count += shift as i32;
        if self.count >= 0 && self.pos < self.data.len() {
            let hi = u32::from(self.data[self.pos]);
            let lo = u32::from(self.data.get(self.pos + 1).copied().unwrap_or(0));
            self.value |= (hi << 8 | lo) << self.count;
            self.pos += 2;
            self.count -= 16;
        }
    }

    #[inline]
    fn decide(&mut self, split: u32) -> bool {
        let bound = split << 16;
        if self.value >= bound {
            self.range -= split;
            self.value -= bound;
            true
        } else {
            self.range = split;
            false
        }
    }

    #[inline]
    pub fn bit(&mut self, prob: u8) -> bool {
        self.renorm();
        let split = 1 + (((self.range - 1) * u32::from(prob)) >> 8);
        self.decide(split)
    }

    #[inline]
    pub fn flag(&mut self) -> bool {
        self.renorm();
        let split = (self.range + 1) >> 1;
        self.decide(split)
    }

    pub fn literal(&mut self, n: u32) -> u32 {
        (0..n).fold(0, |v, _| v << 1 | u32::from(self.flag()))
    }

    pub fn prob7(&mut self) -> u8 {
        let v = (self.literal(7) << 1) as u8;
        v.max(1)
    }

    #[inline]
    pub fn tree(&mut self, tree: &[i8], probs: &[u8]) -> usize {
        let mut i = 0usize;
        loop {
            let next = tree[i + usize::from(self.bit(probs[i >> 1]))];
            if next <= 0 {
                return (-next) as usize;
            }
            i = next as usize;
        }
    }

    pub fn past_end(&mut self) -> bool {
        if self.pos >= self.data.len() && self.count >= 0 {
            self.end_hits += 1;
        }
        self.end_hits > 10
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) struct Encoder {
        out: Vec<u8>,
        low: u32,
        range: u32,
        count: i32,
    }

    impl Encoder {
        pub(crate) fn new() -> Self {
            Self { out: Vec::new(), low: 0, range: 255, count: -24 }
        }

        fn carry(&mut self) {
            let mut i = self.out.len();
            while i > 0 {
                i -= 1;
                if self.out[i] == 0xFF {
                    self.out[i] = 0;
                } else {
                    self.out[i] += 1;
                    break;
                }
            }
        }

        pub(crate) fn bit(&mut self, prob: u8, bit: bool) {
            let split = 1 + (((self.range - 1) * u32::from(prob)) >> 8);
            if bit {
                self.low = self.low.wrapping_add(split);
                self.range -= split;
            } else {
                self.range = split;
            }
            let mut shift = norm_shift(self.range) as i32;
            self.range <<= shift;
            self.count += shift;
            if self.count >= 0 {
                let offset = shift - self.count;
                if (self.low << (offset - 1)) & 0x8000_0000 != 0 {
                    self.carry();
                }
                self.out.push((self.low >> (24 - offset)) as u8);
                self.low <<= offset;
                shift = self.count;
                self.low &= 0x00FF_FFFF;
                self.count -= 8;
            }
            self.low <<= shift;
        }

        pub(crate) fn finish(mut self) -> Vec<u8> {
            for _ in 0..32 {
                self.bit(128, false);
            }
            self.out
        }
    }

    #[test]
    fn norm_shift_matches_range_classes() {
        assert_eq!(norm_shift(1), 7);
        assert_eq!(norm_shift(2), 6);
        assert_eq!(norm_shift(127), 1);
        assert_eq!(norm_shift(128), 0);
        assert_eq!(norm_shift(255), 0);
    }

    #[test]
    fn round_trip_bits_flags_and_trees() {
        let mut seed = 0x1234_5678u32;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        let items: Vec<(u8, bool)> = (0..5000)
            .map(|_| {
                let r = next();
                ((r & 0xFF).max(1) as u8, (r >> 8) % 3 == 0)
            })
            .collect();
        let mut enc = Encoder::new();
        for &(p, b) in &items {
            enc.bit(p, b);
        }
        enc.bit(128, true);
        enc.bit(128, false);
        enc.bit(10, true);
        enc.bit(200, true);
        let data = enc.finish();
        let mut dec = RangeDecoder::new(&data);
        for (i, &(p, b)) in items.iter().enumerate() {
            assert_eq!(dec.bit(p), b, "bit {i}");
        }
        assert_eq!(dec.literal(2), 0b10);
        assert_eq!(dec.tree(&[-3, 2, 0, -5], &[10, 200]), 5);
        assert!(!dec.past_end());
    }

    #[test]
    fn prob7_never_zero() {
        let mut enc = Encoder::new();
        for _ in 0..7 {
            enc.bit(128, false);
        }
        let data = enc.finish();
        assert_eq!(RangeDecoder::new(&data).prob7(), 1);
    }

    #[test]
    fn exhausted_buffer_reads_zeros_and_is_flagged() {
        let mut dec = RangeDecoder::new(&[]);
        for _ in 0..200 {
            assert!(!dec.bit(128));
        }
        for _ in 0..10 {
            assert!(!dec.past_end());
        }
        assert!(dec.past_end());
    }
}
