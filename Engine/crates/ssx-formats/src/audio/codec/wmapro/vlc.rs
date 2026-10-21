use super::*;

#[derive(Clone, Debug)]
pub struct BitReader<'a> {
    pub(super) data: &'a [u8],
    pub(super) pos: usize,
    pub(super) end: usize,
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8], start: usize, end: usize) -> Self {
        Self { data, pos: start, end: end.min(data.len() * 8) }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn end(&self) -> usize {
        self.end
    }

    pub fn left(&self) -> isize {
        self.end as isize - self.pos as isize
    }

    #[inline]
    pub fn peek32(&self) -> u32 {
        let byte = self.pos >> 3;
        let w = match self.data.get(byte..byte + 8) {
            Some(b) => u64::from_be_bytes(b.try_into().expect("8 bytes")),
            None => {
                let mut b = [0u8; 8];
                for (i, v) in b.iter_mut().enumerate() {
                    *v = self.data.get(byte + i).copied().unwrap_or(0);
                }
                u64::from_be_bytes(b)
            }
        };
        ((w << (self.pos & 7)) >> 32) as u32
    }

    #[inline]
    pub fn skip(&mut self, n: usize) {
        self.pos += n;
    }

    #[inline]
    pub fn bits(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        let v = self.peek32() >> (32 - n);
        self.pos += n as usize;
        v
    }

    #[inline]
    pub fn bit(&mut self) -> bool {
        self.bits(1) != 0
    }

    pub fn sbits(&mut self, n: u32) -> i32 {
        let v = self.bits(n) as i32;
        (v << (32 - n)) >> (32 - n)
    }
}

pub struct Vlc {
    fast: Vec<u32>,
    pub(super) codes: Vec<u32>,
    pub(super) lens: Vec<u8>,
    pub(super) syms: Vec<u16>,
}

const FAST_BITS: u32 = 10;

impl Vlc {
    pub fn new(lens: &[u8], syms: &[u16]) -> Result<Self> {
        let mut codes = Vec::with_capacity(lens.len());
        let mut fast = vec![0u32; 1 << FAST_BITS];
        let mut next: u64 = 0;
        for (&len, &sym) in lens.iter().zip(syms) {
            if !(1..=32).contains(&len) || next >= 1 << 32 {
                return Err(Error::invalid("WMA Pro code table: bad length or over-full"));
            }
            codes.push(next as u32);
            if u32::from(len) <= FAST_BITS {
                let first = (next >> (32 - FAST_BITS)) as usize;
                for e in &mut fast[first..first + (1 << (FAST_BITS - u32::from(len)))] {
                    *e = u32::from(sym) << 8 | u32::from(len);
                }
            }
            next += 1 << (32 - len);
        }
        if next != 1 << 32 {
            return Err(Error::invalid("WMA Pro code table: incomplete prefix code"));
        }
        Ok(Self { fast, codes, lens: lens.to_vec(), syms: syms.to_vec() })
    }

    pub fn kraft(&self) -> f64 {
        self.lens.iter().map(|&l| 0.5f64.powi(i32::from(l))).sum()
    }

    #[inline]
    pub fn read(&self, r: &mut BitReader) -> u16 {
        let peek = r.peek32();
        let e = self.fast[(peek >> (32 - FAST_BITS)) as usize];
        if e != 0 {
            r.skip((e & 0xFF) as usize);
            return (e >> 8) as u16;
        }
        let i = self.codes.partition_point(|&c| c <= peek) - 1;
        r.skip(usize::from(self.lens[i]));
        self.syms[i]
    }
}

pub struct Codes {
    pub scale: Vlc,
    pub scale_rl: Vlc,
    pub coef: [Vlc; 2],
    pub vec4: Vlc,
    pub vec2: Vlc,
    pub vec1: Vlc,
    pub scale_rl_runlevel: Vec<(u8, u8)>,
    pub coef_runlevel: [Vec<(u16, u8)>; 2],
}

fn widen<const N: usize>(s: &[u8; N]) -> Vec<u16> {
    s.iter().map(|&v| u16::from(v)).collect()
}

fn expand_groups<R: From<u8>>(groups: &[(u8, u8)]) -> Vec<(R, u8)> {
    let mut out: Vec<(R, u8)> = vec![(R::from(0), 0), (R::from(0), 0)];
    for &(level, runs) in groups {
        out.extend((0..runs).map(|r| (R::from(r), level)));
    }
    out
}

pub static CODES: LazyLock<Codes> = LazyLock::new(|| {
    let v = |l: &[u8], s: &[u16]| Vlc::new(l, s).expect("WMA Pro tables are complete prefix codes (unit test)");
    Codes {
        scale: v(&t::SCALE_LENS, &widen(&t::SCALE_SYMS)),
        scale_rl: v(&t::SCALE_RL_LENS, &widen(&t::SCALE_RL_SYMS)),
        coef: [v(&t::COEF0_LENS, &t::COEF0_SYMS), v(&t::COEF1_LENS, &widen(&t::COEF1_SYMS))],
        vec4: v(&t::VEC4_LENS, &t::VEC4_SYMS),
        vec2: v(&t::VEC2_LENS, &widen(&t::VEC2_SYMS)),
        vec1: v(&t::VEC1_LENS, &widen(&t::VEC1_SYMS)),
        scale_rl_runlevel: expand_groups(t::SCALE_RL_GROUPS),
        coef_runlevel: [expand_groups::<u16>(t::COEF0_GROUPS), expand_groups::<u16>(t::COEF1_GROUPS)],
    }
});
