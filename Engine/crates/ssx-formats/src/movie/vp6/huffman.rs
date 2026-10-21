pub(crate) struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn left(&self) -> isize {
        (self.data.len() * 8) as isize - self.pos as isize
    }

    #[inline]
    pub fn bit(&mut self) -> u32 {
        let b = self.data.get(self.pos >> 3).map_or(0, |&b| (b >> (7 - (self.pos & 7))) & 1);
        self.pos += 1;
        u32::from(b)
    }

    pub fn bits(&mut self, n: u32) -> u32 {
        (0..n).fold(0, |v, _| v << 1 | self.bit())
    }
}

pub const COEFF_MAP: [u8; 22] = [13, 14, 11, 0, 1, 15, 16, 18, 2, 17, 3, 4, 19, 20, 5, 6, 21, 22, 7, 8, 9, 10];
pub const RUN_MAP: [u8; 16] = [10, 13, 11, 12, 0, 1, 2, 3, 14, 8, 15, 16, 4, 5, 6, 7];

const INTERNAL: i16 = -1;

#[derive(Clone, Copy, Default)]
struct Node {
    sym: i16,
    n0: u16,
    count: u32,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Tree {
    nodes: [Node; 24],
    root: usize,
}

impl Tree {
    pub fn build(probs: &[u8], map: &[u8], size: usize) -> Self {
        let mut t = Self::default();
        let n = &mut t.nodes;
        n[size].count = 256;
        for i in 0..size - 1 {
            let parent = n[size + i].count;
            let p = u32::from(probs[i]);
            n[usize::from(map[2 * i])].count = ((parent * p) >> 8).max(1);
            n[usize::from(map[2 * i + 1])].count = ((parent * (255 - p)) >> 8).max(1);
        }
        for (i, node) in n.iter_mut().enumerate().take(size) {
            node.sym = i as i16;
        }
        n[..size].sort_by(|a, b| a.count.cmp(&b.count).then(b.sym.cmp(&a.sym)));
        for (end, i) in (size..).zip((0..2 * size - 2).step_by(2)) {
            let count = n[i].count + n[i + 1].count;
            let mut j = end;
            while j > i + 2 && count <= n[j - 1].count {
                n[j] = n[j - 1];
                j -= 1;
            }
            n[j] = Node { sym: INTERNAL, n0: i as u16, count };
        }
        t.root = 2 * size - 2;
        t
    }

    #[inline]
    pub fn read(&self, br: &mut BitReader) -> usize {
        let mut i = self.root;
        while self.nodes[i].sym == INTERNAL {
            i = usize::from(self.nodes[i].n0) + br.bit() as usize;
        }
        self.nodes[i].sym as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_complete_and_prefix_free() {
        let probs = [128u8, 90, 200, 30, 128, 250, 7, 160];
        let t = Tree::build(&probs, &RUN_MAP, 9);
        let mut len = [0usize; 9];
        for code in 0u32..256 {
            let bytes = [code as u8];
            let mut br = BitReader::new(&bytes);
            let s = t.read(&mut br);
            assert!(len[s] == 0 || len[s] == br.pos);
            len[s] = br.pos;
        }
        let kraft: f64 = len.iter().map(|&l| 0.5f64.powi(l as i32)).sum();
        assert!((kraft - 1.0).abs() < 1e-12, "{len:?}");
    }

    #[test]
    fn skewed_model_gives_short_code_to_likely_symbol() {
        let mut probs = [128u8; 11];
        probs[0] = 250;
        probs[1] = 250;
        let t = Tree::build(&probs, &COEFF_MAP, 12);
        let mut br = BitReader::new(&[0x80]);
        assert_eq!(t.read(&mut br), 11);
        assert_eq!(br.pos, 1);
        let mut br = BitReader::new(&[0x00]);
        assert_ne!(t.read(&mut br), 11);
    }

    #[test]
    fn bit_reader_msb_first_and_zero_past_end() {
        let mut br = BitReader::new(&[0b1010_0000]);
        assert_eq!(br.bits(3), 0b101);
        assert_eq!(br.bits(8), 0);
        assert_eq!(br.left(), -3);
    }
}
