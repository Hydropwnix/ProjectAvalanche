use super::Limits;
use super::bits::{Lsb, ilog};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Headers {
    pub channels: u8,
    pub rate: u32,
    pub blocksize_exp: [u8; 2],
    pub codebooks: usize,
    pub table_bytes: u64,
    pub temp_bytes: u64,
    pub modes: usize,
}

const ENTRY_BYTES: u64 = 16;
const VQ_VALUE_BYTES: u64 = 4;

impl Headers {
    pub fn parse(private: &[u8], limits: &Limits) -> Result<Headers, String> {
        let [id, comment, setup] = split(private)?;
        let (channels, rate, blocksize_exp) = identification(id)?;
        comment_header(comment)?;
        let s = setup_header(setup, channels, limits)?;
        Ok(Headers {
            channels,
            rate,
            blocksize_exp,
            codebooks: s.codebooks,
            table_bytes: s.table_bytes,
            temp_bytes: s.temp_bytes,
            modes: s.modes,
        })
    }
}

pub fn split(b: &[u8]) -> Result<[&[u8]; 3], String> {
    if b.first() != Some(&2) {
        return Err("the codec private data is not three Xiph-laced headers".into());
    }
    let mut pos = 1;
    let mut sizes = [0usize; 2];
    for s in &mut sizes {
        loop {
            let v = *b.get(pos).ok_or("the headers' sizes are cut short")?;
            pos += 1;
            *s += usize::from(v);
            if v != 255 {
                break;
            }
        }
    }
    let a =
        pos.checked_add(sizes[0]).filter(|&e| e <= b.len()).ok_or("the identification header reaches past the data")?;
    let c = a.checked_add(sizes[1]).filter(|&e| e <= b.len()).ok_or("the comment header reaches past the data")?;
    Ok([&b[pos..a], &b[a..c], &b[c..]])
}

fn packet<'a>(p: &'a [u8], kind: u8, name: &str) -> Result<&'a [u8], String> {
    if p.len() < 7 || p[0] != kind || &p[1..7] != b"vorbis" {
        return Err(format!("the {name} header is not one"));
    }
    Ok(&p[7..])
}

fn identification(p: &[u8]) -> Result<(u8, u32, [u8; 2]), String> {
    let b = packet(p, 1, "identification")?;
    if b.len() != 23 {
        return Err(format!("an identification header of {} bytes (30)", p.len()));
    }
    let u32le = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    if u32le(0) != 0 {
        return Err(format!("Vorbis version {} (0)", u32le(0)));
    }
    let (channels, rate) = (b[4], u32le(5));
    let (bs0, bs1) = (b[21] & 0xF, b[21] >> 4);
    if channels == 0 || rate == 0 {
        return Err(format!("{channels} channels at {rate} Hz"));
    }
    if !(6..=13).contains(&bs0) || !(6..=13).contains(&bs1) || bs0 > bs1 {
        return Err(format!("block sizes 2^{bs0} / 2^{bs1} (from 2^6 to 2^13, the short not above the long)"));
    }
    if b[22] & 1 == 0 {
        return Err("the identification header's framing bit is clear".into());
    }
    Ok((channels, rate, [bs0, bs1]))
}

fn comment_header(p: &[u8]) -> Result<(), String> {
    let b = packet(p, 3, "comment")?;
    let mut pos = 0usize;
    let mut take = |n: usize| -> Result<usize, String> {
        let at = pos;
        pos = pos.checked_add(n).filter(|&e| e <= b.len()).ok_or("the comment header's lengths reach past it")?;
        Ok(at)
    };
    let len = |b: &[u8], at: usize| u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]) as usize;
    let at = take(4)?;
    take(len(b, at))?;
    let at = take(4)?;
    for _ in 0..len(b, at) {
        let at = take(4)?;
        take(len(b, at))?;
    }
    let at = take(1)?;
    if b[at] & 1 == 0 {
        return Err("the comment header's framing bit is clear".into());
    }
    Ok(())
}

struct Setup {
    codebooks: usize,
    table_bytes: u64,
    temp_bytes: u64,
    modes: usize,
}

fn setup_header(p: &[u8], channels: u8, limits: &Limits) -> Result<Setup, String> {
    let b = packet(p, 5, "setup")?;
    let mut r = Lsb::new(b);
    let bits = |r: &mut Lsb, n: u32| r.read(n).ok_or_else(|| "the setup header ends early".to_string());
    let codebooks = bits(&mut r, 8)? as usize + 1;
    let (mut table_bytes, mut temp_bytes) = (0u64, 0u64);
    for k in 0..codebooks {
        let (kept, temp) = codebook(&mut r, k).map_err(|e| format!("codebook {k}: {e}"))?;
        table_bytes += kept;
        temp_bytes = temp_bytes.max(temp);
        if table_bytes > limits.max_vorbis_tables || temp_bytes > limits.max_vorbis_tables {
            return Err(format!(
                "codebooks whose tables take more than {} bytes in the decoder (a header bomb)",
                limits.max_vorbis_tables
            ));
        }
    }
    let book = |v: u32, what: &str| -> Result<(), String> {
        if (v as usize) < codebooks { Ok(()) } else { Err(format!("{what} names codebook {v} of {codebooks}")) }
    };
    for _ in 0..bits(&mut r, 6)? + 1 {
        if bits(&mut r, 16)? != 0 {
            return Err("a time-domain transform other than 0".into());
        }
    }
    let floors = bits(&mut r, 6)? + 1;
    for f in 0..floors {
        match bits(&mut r, 16)? {
            0 => {
                bits(&mut r, 8)?;
                bits(&mut r, 16)?;
                bits(&mut r, 16)?;
                bits(&mut r, 6)?;
                bits(&mut r, 8)?;
                for _ in 0..bits(&mut r, 4)? + 1 {
                    book(bits(&mut r, 8)?, "floor 0")?;
                }
            }
            1 => floor1(&mut r, &book).map_err(|e| format!("floor {f}: {e}"))?,
            t => return Err(format!("floor {f} of type {t} (0 or 1)")),
        }
    }
    let residues = bits(&mut r, 6)? + 1;
    for k in 0..residues {
        let t = bits(&mut r, 16)?;
        if t > 2 {
            return Err(format!("residue {k} of type {t} (0 to 2)"));
        }
        let (begin, end) = (bits(&mut r, 24)?, bits(&mut r, 24)?);
        if end < begin {
            return Err(format!("residue {k} ends before it begins"));
        }
        bits(&mut r, 24)?;
        let classifications = bits(&mut r, 6)? + 1;
        book(bits(&mut r, 8)?, "a residue's class book")?;
        let mut cascade = Vec::new();
        for _ in 0..classifications {
            let low = bits(&mut r, 3)?;
            let high = if bits(&mut r, 1)? == 1 { bits(&mut r, 5)? } else { 0 };
            cascade.push(high * 8 + low);
        }
        for c in cascade {
            for j in 0..8 {
                if c & (1 << j) != 0 {
                    book(bits(&mut r, 8)?, "a residue")?;
                }
            }
        }
    }
    let mappings = bits(&mut r, 6)? + 1;
    let ch = u32::from(channels);
    for m in 0..mappings {
        if bits(&mut r, 16)? != 0 {
            return Err(format!("mapping {m} of a type other than 0"));
        }
        let submaps = if bits(&mut r, 1)? == 1 { bits(&mut r, 4)? + 1 } else { 1 };
        if bits(&mut r, 1)? == 1 {
            let w = ilog(ch - 1);
            for _ in 0..bits(&mut r, 8)? + 1 {
                let (magnitude, angle) = (bits(&mut r, w)?, bits(&mut r, w)?);
                if magnitude == angle || magnitude >= ch || angle >= ch {
                    return Err(format!("mapping {m} couples channels {magnitude} and {angle} of {ch}"));
                }
            }
        }
        if bits(&mut r, 2)? != 0 {
            return Err(format!("mapping {m}'s reserved bits are set"));
        }
        if submaps > 1 {
            for _ in 0..ch {
                if bits(&mut r, 4)? >= submaps {
                    return Err(format!("mapping {m} names a submap past its {submaps}"));
                }
            }
        }
        for _ in 0..submaps {
            bits(&mut r, 8)?;
            if bits(&mut r, 8)? >= floors || bits(&mut r, 8)? >= residues {
                return Err(format!("mapping {m} names a floor or residue past the setup's"));
            }
        }
    }
    let modes = bits(&mut r, 6)? as usize + 1;
    for k in 0..modes {
        bits(&mut r, 1)?;
        if bits(&mut r, 16)? != 0 || bits(&mut r, 16)? != 0 || bits(&mut r, 8)? >= mappings {
            return Err(format!("mode {k}: a window or transform other than 0, or a mapping past the setup's"));
        }
    }
    if bits(&mut r, 1)? != 1 {
        return Err("the setup header's framing bit is clear".into());
    }
    Ok(Setup { codebooks, table_bytes, temp_bytes, modes })
}

fn codebook(r: &mut Lsb, _k: usize) -> Result<(u64, u64), String> {
    let mut bits = |n: u32| r.read(n).ok_or_else(|| "the setup header ends early".to_string());
    if bits(24)? != 0x56_4342 {
        return Err("no codebook sync pattern".into());
    }
    let dims = bits(16)?;
    let entries = bits(24)?;
    if dims == 0 || dims > 32 || entries == 0 || entries > 128 * 1024 {
        return Err(format!("{entries} entries in {dims} dimensions (1 to 131 072 entries, 1 to 32 dimensions)"));
    }
    if bits(1)? == 1 {
        let (mut current, mut length) = (0u32, bits(5)? + 1);
        while current < entries {
            if length > 32 {
                return Err("code lengths past 32 bits".into());
            }
            current += bits(ilog(entries - current))?;
            length += 1;
        }
        if current != entries {
            return Err("ordered code lengths past its entries".into());
        }
    } else {
        let sparse = bits(1)? == 1;
        for _ in 0..entries {
            if !sparse || bits(1)? == 1 {
                bits(5)?;
            }
        }
    }
    let lookup = bits(4)?;
    let values = match lookup {
        0 => 0,
        1 | 2 => {
            bits(32)?;
            bits(32)?;
            let value_bits = bits(4)? + 1;
            bits(1)?;
            let values = if lookup == 1 { lookup1(entries, dims)? } else { u64::from(entries) * u64::from(dims) };
            for _ in 0..values {
                bits(value_bits)?;
            }
            values
        }
        t => return Err(format!("lookup type {t} (0 to 2)")),
    };
    let vq = if lookup == 0 { 0 } else { u64::from(entries) * u64::from(dims) * VQ_VALUE_BYTES };
    Ok((u64::from(entries) * ENTRY_BYTES + vq, u64::from(entries) * 5 + values * 2))
}

fn lookup1(entries: u32, dims: u32) -> Result<u64, String> {
    let pow = |r: u64| (0..dims).try_fold(1u64, |acc, _| acc.checked_mul(r).filter(|&v| v <= u64::from(u32::MAX)));
    let mut r = 0u64;
    while pow(r + 1).is_some_and(|v| v <= u64::from(entries)) {
        r += 1;
    }
    Ok(r)
}

fn floor1(r: &mut Lsb, book: &dyn Fn(u32, &str) -> Result<(), String>) -> Result<(), String> {
    let bits = |r: &mut Lsb, n: u32| r.read(n).ok_or_else(|| "the setup header ends early".to_string());
    let partitions = bits(r, 5)?;
    let mut classes = Vec::new();
    for _ in 0..partitions {
        classes.push(bits(r, 4)? as usize);
    }
    let max_class = classes.iter().copied().max().map_or(0, |m| m + 1);
    let mut dims = vec![0u32; max_class];
    for d in &mut dims {
        *d = bits(r, 3)? + 1;
        let subclasses = bits(r, 2)?;
        if subclasses > 0 {
            book(bits(r, 8)?, "a floor's master book")?;
        }
        for _ in 0..1u32 << subclasses {
            let b = bits(r, 8)?;
            if b > 0 {
                book(b - 1, "a floor's subclass")?;
            }
        }
    }
    bits(r, 2)?;
    let range_bits = bits(r, 4)?;
    let mut points = 2u32;
    for c in classes {
        for _ in 0..dims[c] {
            bits(r, range_bits)?;
            points += 1;
        }
    }
    if points > 65 {
        return Err(format!("{points} points in the X list (at most 65)"));
    }
    Ok(())
}
