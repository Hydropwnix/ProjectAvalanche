use super::*;

pub(super) struct Huff {
    lut: Vec<u32>,
}

const PRIMARY: u32 = 8;
const SUB: u32 = 1 << 31;

impl Huff {
    pub(super) fn new(entries: &[(u16, u32, u8)]) -> Self {
        let mut lut = vec![0u32; 1 << PRIMARY];
        let p = PRIMARY as u8;
        for &(sym, code, len) in entries.iter().filter(|e| e.2 <= p) {
            let shift = p - len;
            for j in 0..1u32 << shift {
                lut[((code << shift) | j) as usize] = (u32::from(len) << 16) | u32::from(sym);
            }
        }
        for prefix in 0..1u32 << PRIMARY {
            let long: Vec<_> = entries.iter().filter(|e| e.2 > p && e.1 >> (e.2 - p) == prefix).collect();
            let Some(width) = long.iter().map(|e| e.2 - p).max() else { continue };
            let offset = lut.len() as u32;
            lut.resize(lut.len() + (1 << width), 0);
            for &&(sym, code, len) in &long {
                let rest = code & ((1 << (len - p)) - 1);
                let shift = width - (len - p);
                for j in 0..1u32 << shift {
                    lut[(offset + ((rest << shift) | j)) as usize] = (u32::from(len) << 16) | u32::from(sym);
                }
            }
            lut[prefix as usize] = SUB | (u32::from(width) << 20) | offset;
        }
        Self { lut }
    }

    #[inline]
    pub(super) fn decode(&self, r: &mut BitReader) -> u32 {
        let w = r.peek32();
        let mut e = self.lut[(w >> (32 - PRIMARY)) as usize];
        if e & SUB != 0 {
            let width = (e >> 20) & 0x1F;
            let idx = (w << PRIMARY) >> (32 - width);
            e = self.lut[((e & 0xF_FFFF) + idx) as usize];
        }
        r.skip(e >> 16);
        e & 0xFFFF
    }
}

struct HuffTables {
    pairs: Vec<Option<Huff>>,
    quad_a: Huff,
}

pub(super) fn pair_entries(t: &PairTable) -> Vec<(u16, u32, u8)> {
    (0..t.dim * t.dim).map(|i| ((((i / t.dim) << 4) | (i % t.dim)) as u16, t.codes[i], t.lens[i])).collect()
}

fn huff_tables() -> &'static HuffTables {
    static T: OnceLock<HuffTables> = OnceLock::new();
    T.get_or_init(|| {
        let mut pairs: Vec<Option<Huff>> =
            PAIR_TABLES.iter().map(|t| t.as_ref().map(|t| Huff::new(&pair_entries(t)))).collect();
        pairs.push(Some(Huff::new(&pair_entries(&PAIR_TABLE_16))));
        pairs.push(Some(Huff::new(&pair_entries(&PAIR_TABLE_24))));
        let quad: Vec<_> = (0..16).map(|i| (i as u16, QUAD_A_CODE[i], QUAD_A_LEN[i])).collect();
        HuffTables { pairs, quad_a: Huff::new(&quad) }
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChannelReport {
    pub end_delta: i32,
    pub nonzero_end: usize,
}

pub(super) fn huffman(
    r: &mut BitReader,
    end: usize,
    g: &GranuleChannel,
    sfi: usize,
    q: &mut [i32; 576],
) -> Result<ChannelReport> {
    let ht = huff_tables();
    let big = 2 * g.big_values as usize;
    let (r1, r2) = if g.window_switching {
        (36, 576)
    } else {
        let a = g.region0_count as usize + 1;
        let b = a + g.region1_count as usize + 1;
        (SFB_LONG[sfi][a.min(22)], SFB_LONG[sfi][b.min(22)])
    };
    let mut i = 0;
    while i < big {
        let region = usize::from(i >= r1) + usize::from(i >= r2);
        let stop = big.min([r1, r2, 576][region]).max(i + 2);
        let sel = g.table_select[region] as usize;
        let (table, linbits) = match sel {
            0 => {
                q[i..stop].fill(0);
                i = stop;
                continue;
            }
            1..=15 => (ht.pairs[sel].as_ref(), 0),
            16..=23 => (ht.pairs[16].as_ref(), LINBITS[sel]),
            _ => (ht.pairs[17].as_ref(), LINBITS[sel]),
        };
        let table = table.ok_or_else(|| Error::invalid(format!("Layer III: Huffman table {sel} does not exist")))?;
        while i < stop {
            let sym = table.decode(r);
            for (k, v) in [sym >> 4, sym & 15].into_iter().enumerate() {
                let mut v = v as i32;
                if v == 15 && linbits > 0 {
                    v += r.bits(linbits) as i32;
                }
                if v != 0 && r.bits(1) != 0 {
                    v = -v;
                }
                q[i + k] = v;
            }
            i += 2;
        }
    }
    let mut nonzero_end = big;
    let mut end_delta;
    loop {
        end_delta = r.pos() as i32 - end as i32;
        if i + 4 > 576 || end_delta >= 0 {
            break;
        }
        let quad = if g.count1table_select { 15 - r.bits(4) } else { ht.quad_a.decode(r) };
        let mut vals = [0i32; 4];
        for (k, v) in vals.iter_mut().enumerate() {
            if quad & (8 >> k) != 0 {
                *v = if r.bits(1) != 0 { -1 } else { 1 };
            }
        }
        if r.pos() > end {
            end_delta = r.pos() as i32 - end as i32;
            break;
        }
        q[i..i + 4].copy_from_slice(&vals);
        i += 4;
        nonzero_end = i;
    }
    q[i..].fill(0);
    Ok(ChannelReport { end_delta, nonzero_end })
}
